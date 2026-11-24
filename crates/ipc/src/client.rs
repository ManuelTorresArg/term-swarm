//! Cliente síncrono del daemon. Un hilo lee el socket; las Respuestas se
//! emparejan por `req` y los Eventos quedan en cola.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::paths;
use crate::proto::*;

/// Manda Pedidos sin esperar respuesta. Se puede clonar y pasar a otros hilos.
#[derive(Clone)]
pub struct Sender {
    stream: Arc<Mutex<UnixStream>>,
    proximo_req: Arc<AtomicU64>,
}

impl Sender {
    pub fn enviar(&self, pedido: Pedido) -> io::Result<u64> {
        let req = self.proximo_req.fetch_add(1, Ordering::Relaxed);
        let mut linea = serde_json::to_vec(&MensajeCliente { req, pedido })?;
        linea.push(b'\n');
        self.stream.lock().unwrap().write_all(&linea)?;
        Ok(req)
    }
}

pub struct Client {
    sender: Sender,
    rx: Receiver<MensajeServidor>,
    eventos: VecDeque<Evento>,
}

impl Client {
    pub fn conectar() -> Result<Self> {
        Self::conectar_a(&paths::socket())
    }

    pub fn conectar_a(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket)
            .with_context(|| format!("no se pudo conectar a {}", socket.display()))?;
        let lector = stream.try_clone()?;
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("swarm-ipc-lector".into())
            .spawn(move || {
                for linea in BufReader::new(lector).lines() {
                    let Ok(linea) = linea else { break };
                    match serde_json::from_str::<MensajeServidor>(&linea) {
                        Ok(m) => {
                            if tx.send(m).is_err() {
                                break;
                            }
                        }
                        Err(e) => eprintln!("swarm-ipc: mensaje inválido del daemon: {e}"),
                    }
                }
            })?;
        let mut c = Client {
            sender: Sender {
                stream: Arc::new(Mutex::new(stream)),
                proximo_req: Arc::new(AtomicU64::new(1)),
            },
            rx,
            eventos: VecDeque::new(),
        };
        match c.pedir(Pedido::Hola {
            version: VERSION_PROTOCOLO,
        })? {
            Valor::Hola { version } if version == VERSION_PROTOCOLO => Ok(c),
            Valor::Hola { version } => bail!(
                "el daemon habla el protocolo v{version} y este cliente el v{VERSION_PROTOCOLO}; reiniciá swarmd"
            ),
            otro => bail!("respuesta inesperada al saludo: {otro:?}"),
        }
    }

    /// Se conecta al daemon y, si no está corriendo, lo arranca (V2).
    /// `swarmd` es el ejecutable a lanzar; si es `None` se busca junto al
    /// ejecutable actual y después en el PATH.
    pub fn conectar_o_arrancar(swarmd: Option<&Path>) -> Result<Self> {
        let socket = paths::socket();
        if let Ok(c) = Self::conectar_a(&socket) {
            return Ok(c);
        }
        arrancar_daemon(swarmd)?;
        let limite = Instant::now() + Duration::from_secs(5);
        loop {
            match Self::conectar_a(&socket) {
                Ok(c) => return Ok(c),
                Err(e) if Instant::now() > limite => {
                    return Err(e.context("swarmd no arrancó a tiempo"));
                }
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }

    /// Para clientes con su propio loop (la app): entrega el Sender, los
    /// Eventos ya encolados y el canal crudo, donde las Respuestas se
    /// emparejan con el `req` que devuelve `Sender::enviar`.
    pub fn en_partes(self) -> (Sender, Vec<Evento>, Receiver<MensajeServidor>) {
        (self.sender, self.eventos.into(), self.rx)
    }

    /// Manda un Pedido y espera su Respuesta. Los Eventos que lleguen mientras tanto quedan en cola.
    pub fn pedir(&mut self, pedido: Pedido) -> Result<Valor> {
        let req = self.sender.enviar(pedido)?;
        loop {
            match self.rx.recv() {
                Ok(MensajeServidor::Respuesta { req: r, resultado }) if r == req => {
                    return resultado.map_err(|e| anyhow!(e));
                }
                Ok(MensajeServidor::Respuesta { .. }) => {} // de un envío sin espera
                Ok(MensajeServidor::Evento { evento }) => self.eventos.push_back(evento),
                Err(_) => bail!("el daemon cerró la conexión"),
            }
        }
    }

    /// Próximo Evento. `None` si venció `timeout` o se cerró la conexión.
    pub fn proximo_evento(&mut self, timeout: Option<Duration>) -> Option<Evento> {
        if let Some(e) = self.eventos.pop_front() {
            return Some(e);
        }
        let limite = timeout.map(|t| Instant::now() + t);
        loop {
            let m = match limite {
                None => self.rx.recv().ok()?,
                Some(l) => match self
                    .rx
                    .recv_timeout(l.saturating_duration_since(Instant::now()))
                {
                    Ok(m) => m,
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
                },
            };
            if let MensajeServidor::Evento { evento } = m {
                return Some(evento);
            }
        }
    }
}

fn arrancar_daemon(swarmd: Option<&Path>) -> Result<()> {
    let exe = match swarmd {
        Some(p) => p.to_path_buf(),
        None => buscar_swarmd(),
    };
    let estado = paths::state_dir();
    fs::create_dir_all(&estado)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(estado.join("swarmd.log"))?;
    let mut cmd = Command::new(&exe);
    cmd.stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // Sesión propia: el daemon no muere con la terminal o la app que lo lanzó.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut hijo = cmd
        .spawn()
        .with_context(|| format!("no se pudo lanzar {}", exe.display()))?;
    // Recoger su estado de salida para que no quede zombie si el daemon muere antes que nosotros.
    thread::spawn(move || hijo.wait());
    Ok(())
}

fn buscar_swarmd() -> PathBuf {
    if let Some(vecino) = std::env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.join("swarmd")))
        && vecino.is_file()
    {
        return vecino;
    }
    PathBuf::from("swarmd")
}
