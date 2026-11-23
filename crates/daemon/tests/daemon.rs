//! Tests de punta a punta: levantan el binario swarmd con directorios propios.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use swarm_ipc::{Client, Estado, Evento, Instantanea, Pedido, Terminal, TerminalId, Valor};

const ESPERA: Duration = Duration::from_secs(10);

struct Entorno {
    dir: PathBuf,
    hijo: Option<Child>,
}

impl Entorno {
    fn nuevo(nombre: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("swarm-test-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = Entorno { dir, hijo: None };
        e.arrancar();
        e
    }

    fn socket(&self) -> PathBuf {
        self.dir.join("run/swarmd.sock")
    }

    fn arrancar(&mut self) {
        let hijo = Command::new(env!("CARGO_BIN_EXE_swarmd"))
            .env("SWARM_RUNTIME_DIR", self.dir.join("run"))
            .env("SWARM_STATE_DIR", self.dir.join("state"))
            .env("SWARM_NO_NOTIFICAR", "1")
            .env("SWARM_UMBRAL_QUIETA_MS", "400")
            .env("SHELL", "/bin/sh")
            .env("RUST_LOG", "warn")
            .spawn()
            .unwrap();
        self.hijo = Some(hijo);
        let limite = Instant::now() + ESPERA;
        while std::os::unix::net::UnixStream::connect(self.socket()).is_err() {
            assert!(Instant::now() < limite, "swarmd no arrancó");
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn cliente(&self) -> Client {
        Client::conectar_a(&self.socket()).unwrap()
    }

    /// Apaga con el Pedido y espera a que el proceso salga.
    fn apagar(&mut self) {
        self.cliente().pedir(Pedido::Apagar).unwrap();
        let mut hijo = self.hijo.take().unwrap();
        let limite = Instant::now() + ESPERA;
        while hijo.try_wait().unwrap().is_none() {
            assert!(Instant::now() < limite, "swarmd no se apagó");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Entorno {
    fn drop(&mut self) {
        if let Some(mut h) = self.hijo.take() {
            let _ = h.kill();
            let _ = h.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn estructura(c: &mut Client) -> Instantanea {
    match c.pedir(Pedido::Estructura).unwrap() {
        Valor::Estructura(i) => i,
        otro => panic!("{otro:?}"),
    }
}

fn crear_terminal(c: &mut Client, proyecto: Option<u64>) -> TerminalId {
    match c
        .pedir(Pedido::CrearTerminal {
            proyecto,
            cols: 80,
            rows: 24,
        })
        .unwrap()
    {
        Valor::TerminalCreada { id } => id,
        otro => panic!("{otro:?}"),
    }
}

fn escribir(c: &mut Client, id: TerminalId, texto: &str) {
    c.pedir(Pedido::Entrada {
        id,
        datos: texto.as_bytes().to_vec(),
    })
    .unwrap();
}

/// Espera hasta que una TerminalActualizada de `id` cumpla `pred`.
fn esperar_terminal(
    c: &mut Client,
    id: TerminalId,
    mut pred: impl FnMut(&Terminal) -> bool,
) -> Terminal {
    let limite = Instant::now() + ESPERA;
    loop {
        let resta = limite.saturating_duration_since(Instant::now());
        match c.proximo_evento(Some(resta)) {
            Some(Evento::TerminalActualizada(t)) if t.id == id && pred(&t) => return t,
            Some(_) => {}
            None => panic!("la Terminal {id} no llegó al estado esperado"),
        }
    }
}

#[test]
fn titulo_atencion_mirar_y_termino() {
    let e = Entorno::nuevo("senales");
    let mut c = e.cliente();
    let id = crear_terminal(&mut c, None);

    escribir(&mut c, id, "printf '\\033]0;mi tarea\\007'\n");
    let t = esperar_terminal(&mut c, id, |t| t.titulo.is_some());
    assert_eq!(t.titulo.as_deref(), Some("mi tarea"));
    assert_ne!(
        t.estado,
        Estado::PideAtencion,
        "el BEL que cierra la OSC no es atención"
    );

    escribir(&mut c, id, "printf '\\007'\n");
    esperar_terminal(&mut c, id, |t| t.estado == Estado::PideAtencion);

    // La salida nueva no la despeja (E1)...
    escribir(&mut c, id, "echo sigo\n");
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        estructura(&mut c).terminales[0].estado,
        Estado::PideAtencion
    );

    // ...Mirar sí (E2).
    c.pedir(Pedido::Mirando { id: Some(id) }).unwrap();
    esperar_terminal(&mut c, id, |t| t.estado != Estado::PideAtencion);

    // Mientras se la Mira, un BEL no la pone en Pide atención.
    escribir(&mut c, id, "printf '\\007'; echo listo\n");
    thread::sleep(Duration::from_millis(300));
    assert_ne!(
        estructura(&mut c).terminales[0].estado,
        Estado::PideAtencion
    );

    // Terminada sin que nadie la mire → queda marcada (E5).
    c.pedir(Pedido::Mirando { id: None }).unwrap();
    escribir(&mut c, id, "exit 3\n");
    let t = esperar_terminal(&mut c, id, |t| matches!(t.estado, Estado::Terminada { .. }));
    assert_eq!(t.estado, Estado::Terminada { codigo: 3 });
    assert!(t.terminada_sin_mirar);

    c.pedir(Pedido::Mirando { id: Some(id) }).unwrap();
    let t = esperar_terminal(&mut c, id, |t| !t.terminada_sin_mirar);
    assert_eq!(
        t.estado,
        Estado::Terminada { codigo: 3 },
        "sigue visible hasta que se cierra"
    );

    assert!(
        c.pedir(Pedido::Entrada {
            id,
            datos: b"x".to_vec()
        })
        .is_err()
    );
}

#[test]
fn quieta_despues_del_umbral_y_activa_con_salida() {
    let e = Entorno::nuevo("quieta");
    let mut c = e.cliente();
    let id = crear_terminal(&mut c, None);
    esperar_terminal(&mut c, id, |t| t.estado == Estado::Quieta);
    escribir(&mut c, id, "echo hola\n");
    esperar_terminal(&mut c, id, |t| t.estado == Estado::Activa);
    esperar_terminal(&mut c, id, |t| t.estado == Estado::Quieta);
}

#[test]
fn adjuntar_repite_la_salida_reciente() {
    let e = Entorno::nuevo("repeticion");
    let mut c = e.cliente();
    let id = crear_terminal(&mut c, None);
    escribir(&mut c, id, "echo marca-$((40+2))\n");
    esperar_terminal(&mut c, id, |t| t.estado == Estado::Quieta);

    // Otro cliente, como la app al reabrirse.
    let mut otro = e.cliente();
    otro.pedir(Pedido::Adjuntar {
        id,
        cols: 100,
        rows: 30,
    })
    .unwrap();
    let mut visto = Vec::new();
    let limite = Instant::now() + ESPERA;
    while !String::from_utf8_lossy(&visto).contains("marca-42") {
        match otro.proximo_evento(Some(limite.saturating_duration_since(Instant::now()))) {
            Some(Evento::Salida { id: i, datos }) if i == id => visto.extend(datos),
            Some(_) => {}
            None => panic!(
                "no llegó la repetición: {:?}",
                String::from_utf8_lossy(&visto)
            ),
        }
    }

    // Y después sigue llegando la salida en vivo.
    escribir(&mut c, id, "echo vivo-$((1+1))\n");
    let mut vivo = Vec::new();
    while !String::from_utf8_lossy(&vivo).contains("vivo-2") {
        match otro.proximo_evento(Some(limite.saturating_duration_since(Instant::now()))) {
            Some(Evento::Salida { datos, .. }) => vivo.extend(datos),
            Some(_) => {}
            None => panic!("no llegó la salida en vivo"),
        }
    }
}

#[test]
fn proyectos_orden_y_cierre() {
    let e = Entorno::nuevo("proyectos");
    let mut c = e.cliente();
    let dir = e.dir.clone();
    let Valor::ProyectoCreado { id: p } = c
        .pedir(Pedido::CrearProyecto {
            nombre: "web".into(),
            dir: dir.clone(),
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(
        c.pedir(Pedido::CrearProyecto {
            nombre: "x".into(),
            dir: "relativo".into()
        })
        .is_err()
    );

    let a = crear_terminal(&mut c, Some(p));
    let suelta = crear_terminal(&mut c, None);
    let b = crear_terminal(&mut c, Some(p));
    let ids_de = |i: &Instantanea, p| {
        i.terminales
            .iter()
            .filter(|t| t.proyecto == p)
            .map(|t| t.id)
            .collect::<Vec<_>>()
    };

    let i = estructura(&mut c);
    assert_eq!(ids_de(&i, Some(p)), vec![a, b]);
    assert_eq!(
        i.terminales.iter().find(|t| t.id == a).unwrap().cwd,
        dir.canonicalize().unwrap()
    );

    c.pedir(Pedido::MoverTerminal { id: b, posicion: 0 })
        .unwrap();
    assert_eq!(ids_de(&estructura(&mut c), Some(p)), vec![b, a]);
    c.pedir(Pedido::MoverTerminal {
        id: b,
        posicion: 99,
    })
    .unwrap();
    assert_eq!(ids_de(&estructura(&mut c), Some(p)), vec![a, b]);

    // El que estaba adjuntado se entera del cierre.
    let mut otro = e.cliente();
    otro.pedir(Pedido::Adjuntar {
        id: a,
        cols: 80,
        rows: 24,
    })
    .unwrap();
    c.pedir(Pedido::BorrarProyecto { id: p }).unwrap();
    let limite = Instant::now() + ESPERA;
    loop {
        match otro.proximo_evento(Some(limite.saturating_duration_since(Instant::now()))) {
            Some(Evento::Cerrada { id }) if id == a => break,
            Some(_) => {}
            None => panic!("no llegó Cerrada"),
        }
    }
    let i = estructura(&mut c);
    assert!(i.proyectos.is_empty());
    assert_eq!(ids_de(&i, None), vec![suelta]);

    c.pedir(Pedido::CerrarTerminal { id: suelta }).unwrap();
    assert!(estructura(&mut c).terminales.is_empty());
}

#[test]
fn la_estructura_sobrevive_al_apagado() {
    let mut e = Entorno::nuevo("persistencia");
    let mut c = e.cliente();
    let Valor::ProyectoCreado { id: p } = c
        .pedir(Pedido::CrearProyecto {
            nombre: "api".into(),
            dir: e.dir.clone(),
        })
        .unwrap()
    else {
        panic!()
    };
    let id = crear_terminal(&mut c, Some(p));
    c.pedir(Pedido::RenombrarTerminal {
        id,
        nombre: Some("  refactor login ".into()),
    })
    .unwrap();
    drop(c);
    e.apagar();
    assert!(!e.socket().exists(), "el socket se limpia al apagar");

    e.arrancar();
    let mut c = e.cliente();
    let i = estructura(&mut c);
    assert_eq!(i.proyectos.len(), 1);
    assert_eq!(i.proyectos[0].nombre, "api");
    let t = &i.terminales[0];
    assert_eq!(
        (t.id, t.proyecto, t.nombre.as_deref()),
        (id, Some(p), Some("refactor login"))
    );
    assert!(
        !matches!(t.estado, Estado::Terminada { .. }),
        "se reabre con un shell nuevo (V6)"
    );

    // Los ids nuevos no pisan a los restaurados.
    assert!(crear_terminal(&mut c, None) > id);

    // Un segundo daemon no arranca mientras hay uno.
    let segundo = Command::new(env!("CARGO_BIN_EXE_swarmd"))
        .env("SWARM_RUNTIME_DIR", e.dir.join("run"))
        .env("SWARM_STATE_DIR", e.dir.join("state"))
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!segundo.success());
}
