//! El actor del daemon: único dueño de la Estructura y del estado de cada
//! Terminal. Todo pasa por un solo canal de `Cmd`, así no hay locks.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use swarm_ipc::{
    Estado, Evento, Instantanea, MensajeServidor, Pedido, Proyecto, ProyectoId, Terminal,
    TerminalId, VERSION_PROTOCOLO, Valor, paths,
};
use tokio::sync::mpsc;

use crate::estructura::{self, Guardada, TerminalGuardada};
use crate::notificar::Notificador;
use crate::pty::{self, Proceso};
use crate::senales::Detector;

pub type ClienteId = u64;

/// Cuánta salida reciente se guarda por Terminal para repetir al Adjuntar (V4).
const BUFFER_REPETICION: usize = 1024 * 1024;
/// Tiempo sin salida para pasar de Activa a Quieta (E4).
const UMBRAL_QUIETA: Duration = Duration::from_secs(20);

pub enum Cmd {
    Conectar {
        cliente: ClienteId,
        tx: mpsc::Sender<MensajeServidor>,
    },
    Desconectar {
        cliente: ClienteId,
    },
    Pedido {
        cliente: ClienteId,
        req: u64,
        pedido: Pedido,
    },
    Salida {
        id: TerminalId,
        datos: Vec<u8>,
    },
    Termino {
        id: TerminalId,
        codigo: u32,
    },
    Tick,
    /// SIGTERM / SIGINT: igual que `Pedido::Apagar`.
    Apagar,
}

struct Cliente {
    tx: mpsc::Sender<MensajeServidor>,
    mirando: Option<TerminalId>,
}

struct Term {
    id: TerminalId,
    proyecto: Option<ProyectoId>,
    nombre: Option<String>,
    titulo: Option<String>,
    cwd: PathBuf,
    estado: Estado,
    terminada_sin_mirar: bool,
    ultima_salida: Instant,
    buffer: VecDeque<u8>,
    detector: Detector,
    proceso: Option<Proceso>,
    adjuntos: HashSet<ClienteId>,
}

impl Term {
    fn info(&self) -> Terminal {
        Terminal {
            id: self.id,
            proyecto: self.proyecto,
            nombre: self.nombre.clone(),
            titulo: self.titulo.clone(),
            cwd: self.cwd.clone(),
            estado: self.estado,
            terminada_sin_mirar: self.terminada_sin_mirar,
        }
    }

    fn estado_por_actividad(&self, umbral: Duration) -> Estado {
        if self.ultima_salida.elapsed() >= umbral {
            Estado::Quieta
        } else {
            Estado::Activa
        }
    }

    fn rotulo(&self) -> String {
        self.nombre
            .clone()
            .or_else(|| self.titulo.clone())
            .unwrap_or_else(|| format!("Terminal {}", self.id))
    }
}

pub struct Daemon {
    tx: mpsc::Sender<Cmd>,
    clientes: HashMap<ClienteId, Cliente>,
    proyectos: Vec<Proyecto>,
    /// En orden de presentación; el orden dentro de un grupo es el orden relativo acá.
    terminales: Vec<Term>,
    proximo_id: u64,
    ruta_estructura: PathBuf,
    umbral_quieta: Duration,
    notificador: Notificador,
    apagando: bool,
}

impl Daemon {
    /// Carga la Estructura y reabre cada Terminal con un shell nuevo (V6).
    pub fn nuevo(tx: mpsc::Sender<Cmd>, state_dir: &Path) -> Self {
        let ruta_estructura = state_dir.join("estructura.json");
        let g = estructura::cargar(&ruta_estructura);
        // Solo para tests: E4 dice que el umbral no es configurable.
        let umbral_quieta = std::env::var("SWARM_UMBRAL_QUIETA_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .map(Duration::from_millis)
            .unwrap_or(UMBRAL_QUIETA);
        let mut d = Daemon {
            tx,
            clientes: HashMap::new(),
            proyectos: g.proyectos,
            terminales: Vec::new(),
            proximo_id: g.proximo_id.max(1),
            ruta_estructura,
            umbral_quieta,
            notificador: Notificador::desde_entorno(),
            apagando: false,
        };
        for t in g.terminales {
            let cwd = if t.cwd.is_dir() { t.cwd } else { paths::home() };
            d.terminales
                .push(d.abrir(t.id, t.proyecto, t.nombre, cwd, 80, 24));
            d.proximo_id = d.proximo_id.max(t.id + 1);
        }
        log::info!(
            "Estructura cargada: {} proyectos, {} terminales",
            d.proyectos.len(),
            d.terminales.len()
        );
        d
    }

    pub async fn correr(mut self, mut rx: mpsc::Receiver<Cmd>) {
        while let Some(cmd) = rx.recv().await {
            self.manejar(cmd);
            if self.apagando {
                break;
            }
        }
    }

    fn manejar(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Conectar { cliente, tx } => {
                self.clientes.insert(cliente, Cliente { tx, mirando: None });
            }
            Cmd::Desconectar { cliente } => self.desconectar(cliente),
            Cmd::Pedido {
                cliente,
                req,
                pedido,
            } => {
                let resultado = self.pedido(cliente, pedido).map_err(|e| e.to_string());
                self.enviar(cliente, MensajeServidor::Respuesta { req, resultado });
            }
            Cmd::Salida { id, datos } => self.salida(id, datos),
            Cmd::Termino { id, codigo } => self.termino(id, codigo),
            Cmd::Tick => self.tick(),
            Cmd::Apagar => self.apagar(),
        }
    }

    // ---- Pedidos ----

    fn pedido(&mut self, cliente: ClienteId, pedido: Pedido) -> Result<Valor> {
        match pedido {
            Pedido::Hola { .. } => Ok(Valor::Hola {
                version: VERSION_PROTOCOLO,
            }),
            Pedido::Estructura => Ok(Valor::Estructura(self.instantanea())),
            Pedido::CrearProyecto { nombre, dir } => {
                let nombre = nombre.trim().to_string();
                if nombre.is_empty() {
                    bail!("el Proyecto necesita un nombre");
                }
                if !dir.is_absolute() {
                    bail!("el directorio tiene que ser una ruta absoluta");
                }
                let dir = dir
                    .canonicalize()
                    .map_err(|e| anyhow!("{}: {e}", dir.display()))?;
                if !dir.is_dir() {
                    bail!("{} no es un directorio", dir.display());
                }
                let id = self.nuevo_id();
                self.proyectos.push(Proyecto { id, nombre, dir });
                self.estructura_cambio();
                Ok(Valor::ProyectoCreado { id })
            }
            Pedido::BorrarProyecto { id } => {
                let pos = self
                    .proyectos
                    .iter()
                    .position(|p| p.id == id)
                    .ok_or_else(|| anyhow!("no existe el Proyecto {id}"))?;
                let suyas: Vec<_> = self
                    .terminales
                    .iter()
                    .filter(|t| t.proyecto == Some(id))
                    .map(|t| t.id)
                    .collect();
                for t in suyas {
                    self.quitar_terminal(t);
                }
                self.proyectos.remove(pos);
                self.estructura_cambio();
                Ok(Valor::Nada)
            }
            Pedido::CrearTerminal {
                proyecto,
                cols,
                rows,
            } => {
                let cwd = match proyecto {
                    None => paths::home(),
                    Some(p) => self.proyecto(p)?.dir.clone(),
                };
                let id = self.nuevo_id();
                let t = self.abrir(id, proyecto, None, cwd, cols, rows);
                if matches!(t.estado, Estado::Terminada { .. }) {
                    bail!("no se pudo lanzar el shell (ver el log de swarmd)");
                }
                self.terminales.push(t);
                self.estructura_cambio();
                Ok(Valor::TerminalCreada { id })
            }
            Pedido::CerrarTerminal { id } => {
                self.term(id)?;
                self.quitar_terminal(id);
                self.estructura_cambio();
                Ok(Valor::Nada)
            }
            Pedido::RenombrarTerminal { id, nombre } => {
                let nombre = nombre
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty());
                self.term_mut(id)?.nombre = nombre;
                self.estructura_cambio();
                Ok(Valor::Nada)
            }
            Pedido::MoverTerminal { id, posicion } => {
                let desde = self.indice(id)?;
                let t = self.terminales.remove(desde);
                let grupo: Vec<usize> = self
                    .terminales
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.proyecto == t.proyecto)
                    .map(|(i, _)| i)
                    .collect();
                let hacia = match grupo.get(posicion) {
                    Some(&i) => i,
                    None => grupo
                        .last()
                        .map(|&i| i + 1)
                        .unwrap_or(self.terminales.len()),
                };
                self.terminales.insert(hacia, t);
                self.estructura_cambio();
                Ok(Valor::Nada)
            }
            Pedido::Adjuntar { id, cols, rows } => {
                let t = self.term_mut(id)?;
                t.adjuntos.insert(cliente);
                let (a, b) = t.buffer.as_slices();
                let repeticion = [a, b].concat();
                if let Some(p) = &t.proceso {
                    let tam = pty::tamano(cols, rows);
                    match p.tamano() {
                        Some(actual) if actual.cols == tam.cols && actual.rows == tam.rows => {
                            p.forzar_redibujado()
                        }
                        _ => p.redimensionar(tam),
                    }
                }
                // Se encola antes que cualquier salida nueva: el actor es secuencial.
                if !repeticion.is_empty() {
                    self.enviar(
                        cliente,
                        evento(Evento::Salida {
                            id,
                            datos: repeticion,
                        }),
                    );
                }
                Ok(Valor::Nada)
            }
            Pedido::Soltar { id } => {
                self.term_mut(id)?.adjuntos.remove(&cliente);
                Ok(Valor::Nada)
            }
            Pedido::Entrada { id, datos } => {
                match &self.term(id)?.proceso {
                    Some(p) => p.escribir(datos),
                    None => bail!("la Terminal {id} está Terminada"),
                }
                Ok(Valor::Nada)
            }
            Pedido::Redimensionar { id, cols, rows } => {
                if let Some(p) = &self.term(id)?.proceso {
                    p.redimensionar(pty::tamano(cols, rows));
                }
                Ok(Valor::Nada)
            }
            Pedido::Mirando { id } => {
                if let Some(id) = id {
                    self.term(id)?;
                }
                if let Some(c) = self.clientes.get_mut(&cliente) {
                    c.mirando = id;
                }
                if let Some(id) = id {
                    self.mirar(id);
                }
                Ok(Valor::Nada)
            }
            Pedido::Apagar => {
                self.apagar();
                Ok(Valor::Nada)
            }
        }
    }

    // ---- Eventos de los procesos ----

    fn salida(&mut self, id: TerminalId, datos: Vec<u8>) {
        let mirada = self.mirada(id);
        let umbral = self.umbral_quieta;
        let Some(t) = self.terminales.iter_mut().find(|t| t.id == id) else {
            return;
        };

        t.buffer.extend(&datos);
        if t.buffer.len() > BUFFER_REPETICION {
            let sobra = t.buffer.len() - BUFFER_REPETICION;
            t.buffer.drain(..sobra);
        }
        t.ultima_salida = Instant::now();

        let senales = t.detector.procesar(&datos);
        let antes = (t.estado, t.titulo.clone());
        if let Some(titulo) = senales.titulo {
            t.titulo = titulo;
        }
        let mut aviso = None;
        match t.estado {
            Estado::Terminada { .. } => {}
            Estado::PideAtencion => {} // pegajoso (E1)
            Estado::Activa | Estado::Quieta => {
                t.estado = t.estado_por_actividad(umbral);
                if let Some(mensaje) = senales.atencion
                    && !mirada
                {
                    t.estado = Estado::PideAtencion;
                    aviso = Some((format!("{} pide atención", t.rotulo()), mensaje));
                }
            }
        }
        let cambio = antes != (t.estado, t.titulo.clone());
        let adjuntos: Vec<_> = t.adjuntos.iter().copied().collect();
        let info = cambio.then(|| t.info());
        let proyecto = t.proyecto;

        for c in adjuntos {
            self.enviar(
                c,
                evento(Evento::Salida {
                    id,
                    datos: datos.clone(),
                }),
            );
        }
        if let Some((resumen, mensaje)) = aviso {
            let cuerpo = match mensaje {
                Some(m) => format!("{m}\n{}", self.nombre_grupo(proyecto)),
                None => self.nombre_grupo(proyecto),
            };
            self.notificador.enviar(id, resumen, cuerpo);
        }
        if let Some(info) = info {
            self.difundir(Evento::TerminalActualizada(info));
        }
    }

    fn termino(&mut self, id: TerminalId, codigo: u32) {
        let mirada = self.mirada(id);
        let Some(t) = self.terminales.iter_mut().find(|t| t.id == id) else {
            return;
        };
        t.proceso = None;
        t.estado = Estado::Terminada { codigo };
        t.terminada_sin_mirar = !mirada;
        let resumen = format!("{} terminó", t.rotulo());
        let proyecto = t.proyecto;
        let info = t.info();
        log::info!("terminal {id} terminó con código {codigo}");
        if !mirada {
            let cuerpo = format!(
                "Código de salida {codigo} · {}",
                self.nombre_grupo(proyecto)
            );
            self.notificador.enviar(id, resumen, cuerpo);
        }
        self.difundir(Evento::TerminalActualizada(info));
    }

    fn tick(&mut self) {
        let umbral = self.umbral_quieta;
        let pasaron: Vec<Terminal> = self
            .terminales
            .iter_mut()
            .filter(|t| t.estado == Estado::Activa && t.ultima_salida.elapsed() >= umbral)
            .map(|t| {
                t.estado = Estado::Quieta;
                t.info()
            })
            .collect();
        for info in pasaron {
            self.difundir(Evento::TerminalActualizada(info));
        }
    }

    // ---- Mirar ----

    fn mirada(&self, id: TerminalId) -> bool {
        self.clientes.values().any(|c| c.mirando == Some(id))
    }

    /// Mirar despeja Pide atención y la marca de Terminada sin mirar (E1, E2, E5).
    fn mirar(&mut self, id: TerminalId) {
        let umbral = self.umbral_quieta;
        let Some(t) = self.terminales.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let antes = (t.estado, t.terminada_sin_mirar);
        if t.estado == Estado::PideAtencion {
            t.estado = t.estado_por_actividad(umbral);
        }
        t.terminada_sin_mirar = false;
        if antes != (t.estado, t.terminada_sin_mirar) {
            let info = t.info();
            self.difundir(Evento::TerminalActualizada(info));
        }
    }

    // ---- Ciclo de vida ----

    fn abrir(
        &self,
        id: TerminalId,
        proyecto: Option<ProyectoId>,
        nombre: Option<String>,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
    ) -> Term {
        let proceso = pty::lanzar(id, &cwd, pty::tamano(cols, rows), self.tx.clone())
            .inspect_err(|e| log::error!("terminal {id}: {e:#}"))
            .ok();
        Term {
            id,
            proyecto,
            nombre,
            titulo: None,
            estado: if proceso.is_some() {
                Estado::Activa
            } else {
                Estado::Terminada { codigo: 127 }
            },
            terminada_sin_mirar: proceso.is_none(),
            cwd,
            ultima_salida: Instant::now(),
            buffer: VecDeque::new(),
            detector: Detector::default(),
            proceso,
            adjuntos: HashSet::new(),
        }
    }

    fn quitar_terminal(&mut self, id: TerminalId) {
        let Some(pos) = self.terminales.iter().position(|t| t.id == id) else {
            return;
        };
        let mut t = self.terminales.remove(pos);
        if let Some(p) = &mut t.proceso {
            p.matar();
        }
        for c in &t.adjuntos {
            self.enviar(*c, evento(Evento::Cerrada { id }));
        }
        for c in self.clientes.values_mut() {
            if c.mirando == Some(id) {
                c.mirando = None;
            }
        }
    }

    fn desconectar(&mut self, cliente: ClienteId) {
        self.clientes.remove(&cliente);
        for t in &mut self.terminales {
            t.adjuntos.remove(&cliente);
        }
    }

    /// Mata todos los procesos y termina el actor. La Estructura queda guardada (V5).
    fn apagar(&mut self) {
        if self.apagando {
            return;
        }
        log::info!("apagando");
        self.difundir(Evento::Apagando);
        for t in &mut self.terminales {
            if let Some(p) = &mut t.proceso {
                p.matar();
            }
        }
        self.apagando = true;
    }

    // ---- Auxiliares ----

    fn nuevo_id(&mut self) -> u64 {
        let id = self.proximo_id;
        self.proximo_id += 1;
        id
    }

    fn proyecto(&self, id: ProyectoId) -> Result<&Proyecto> {
        self.proyectos
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow!("no existe el Proyecto {id}"))
    }

    fn nombre_grupo(&self, proyecto: Option<ProyectoId>) -> String {
        match proyecto.and_then(|p| self.proyecto(p).ok()) {
            Some(p) => p.nombre.clone(),
            None => "Sueltas".into(),
        }
    }

    fn indice(&self, id: TerminalId) -> Result<usize> {
        self.terminales
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| anyhow!("no existe la Terminal {id}"))
    }

    fn term(&self, id: TerminalId) -> Result<&Term> {
        Ok(&self.terminales[self.indice(id)?])
    }

    fn term_mut(&mut self, id: TerminalId) -> Result<&mut Term> {
        let i = self.indice(id)?;
        Ok(&mut self.terminales[i])
    }

    fn instantanea(&self) -> Instantanea {
        Instantanea {
            proyectos: self.proyectos.clone(),
            terminales: self.terminales.iter().map(Term::info).collect(),
        }
    }

    fn estructura_cambio(&mut self) {
        let g = Guardada {
            proximo_id: self.proximo_id,
            proyectos: self.proyectos.clone(),
            terminales: self
                .terminales
                .iter()
                .map(|t| TerminalGuardada {
                    id: t.id,
                    proyecto: t.proyecto,
                    nombre: t.nombre.clone(),
                    cwd: t.cwd.clone(),
                })
                .collect(),
        };
        if let Err(e) = estructura::guardar(&self.ruta_estructura, &g) {
            log::error!("no se pudo guardar la Estructura: {e:#}");
        }
        self.difundir(Evento::Estructura(self.instantanea()));
    }

    fn difundir(&mut self, ev: Evento) {
        let ids: Vec<_> = self.clientes.keys().copied().collect();
        for c in ids {
            self.enviar(c, evento(ev.clone()));
        }
    }

    /// Si un cliente no da abasto, se lo desconecta en vez de frenar al actor.
    fn enviar(&mut self, cliente: ClienteId, m: MensajeServidor) {
        let Some(c) = self.clientes.get(&cliente) else {
            return;
        };
        if c.tx.try_send(m).is_err() {
            log::warn!("cliente {cliente} no da abasto o se fue; lo desconecto");
            self.desconectar(cliente);
        }
    }
}

fn evento(evento: Evento) -> MensajeServidor {
    MensajeServidor::Evento { evento }
}
