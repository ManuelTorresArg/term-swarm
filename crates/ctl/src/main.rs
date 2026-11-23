//! swarmctl: CLI para probar y manejar swarmd mientras no existe la app.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use swarm_ipc::{Client, Estado, Evento, Instantanea, Pedido, Terminal, Valor};

#[derive(Parser)]
#[command(about = "Maneja el daemon de swarm")]
struct Cli {
    #[command(subcommand)]
    cmd: Comando,
}

#[derive(Subcommand)]
enum Comando {
    /// Lista Proyectos y Terminales con su estado.
    Ls,
    /// Crea un Proyecto (por defecto en el directorio actual).
    Proyecto {
        nombre: String,
        dir: Option<PathBuf>,
    },
    /// Borra un Proyecto y cierra todas sus Terminales.
    BorrarProyecto { id: u64 },
    /// Abre una Terminal nueva (en Sueltas si no se indica Proyecto).
    Nueva {
        #[arg(short, long)]
        proyecto: Option<u64>,
    },
    /// Cierra una Terminal, matando su proceso.
    Cerrar { id: u64 },
    /// Pone o borra el Nombre de una Terminal.
    Renombrar { id: u64, nombre: Option<String> },
    /// Mueve una Terminal a una posición dentro de su grupo.
    Mover { id: u64, posicion: usize },
    /// Se adjunta a una Terminal. Ctrl+] para soltarla.
    Adjuntar { id: u64 },
    /// Muestra los eventos del daemon a medida que llegan.
    Eventos,
    /// Mata todos los procesos y apaga el daemon (la Estructura se conserva).
    Apagar,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Comando::Apagar = cli.cmd {
        let Ok(mut c) = Client::conectar() else {
            println!("swarmd no está corriendo");
            return Ok(());
        };
        c.pedir(Pedido::Apagar)?;
        return Ok(());
    }

    let mut c = Client::conectar_o_arrancar(None)?;
    match cli.cmd {
        Comando::Ls => imprimir(&instantanea(&mut c)?),
        Comando::Proyecto { nombre, dir } => {
            let dir = match dir {
                Some(d) => std::path::absolute(d)?,
                None => std::env::current_dir()?,
            };
            if let Valor::ProyectoCreado { id } = c.pedir(Pedido::CrearProyecto { nombre, dir })? {
                println!("{id}");
            }
        }
        Comando::BorrarProyecto { id } => {
            c.pedir(Pedido::BorrarProyecto { id })?;
        }
        Comando::Nueva { proyecto } => {
            let (cols, rows) = tamano_terminal().unwrap_or((80, 24));
            if let Valor::TerminalCreada { id } = c.pedir(Pedido::CrearTerminal {
                proyecto,
                cols,
                rows,
            })? {
                println!("{id}");
            }
        }
        Comando::Cerrar { id } => {
            c.pedir(Pedido::CerrarTerminal { id })?;
        }
        Comando::Renombrar { id, nombre } => {
            c.pedir(Pedido::RenombrarTerminal { id, nombre })?;
        }
        Comando::Mover { id, posicion } => {
            c.pedir(Pedido::MoverTerminal { id, posicion })?;
        }
        Comando::Adjuntar { id } => adjuntar(c, id)?,
        Comando::Eventos => {
            while let Some(ev) = c.proximo_evento(None) {
                match ev {
                    Evento::Salida { id, datos } => println!("salida {id}: {} bytes", datos.len()),
                    otro => println!("{otro:?}"),
                }
            }
        }
        Comando::Apagar => unreachable!(),
    }
    Ok(())
}

fn instantanea(c: &mut Client) -> Result<Instantanea> {
    match c.pedir(Pedido::Estructura)? {
        Valor::Estructura(i) => Ok(i),
        otro => bail!("respuesta inesperada: {otro:?}"),
    }
}

fn imprimir(i: &Instantanea) {
    let grupos = i
        .proyectos
        .iter()
        .map(|p| {
            (
                Some(p.id),
                format!("{} [{}]  {}", p.nombre, p.id, p.dir.display()),
            )
        })
        .chain([(None, "Sueltas".to_string())]);
    for (proyecto, cabecera) in grupos {
        let suyas: Vec<&Terminal> = i
            .terminales
            .iter()
            .filter(|t| t.proyecto == proyecto)
            .collect();
        if proyecto.is_none() && suyas.is_empty() {
            continue;
        }
        println!("{cabecera}");
        for t in suyas {
            let estado = match t.estado {
                Estado::Activa => "● Activa".to_string(),
                Estado::Quieta => "○ Quieta".to_string(),
                Estado::PideAtencion => "! Pide atención".to_string(),
                Estado::Terminada { codigo } => {
                    format!(
                        "✕ Terminada ({codigo}){}",
                        if t.terminada_sin_mirar { " *" } else { "" }
                    )
                }
            };
            let rotulo = match (&t.nombre, &t.titulo) {
                (Some(n), Some(ti)) => format!("{n} — {ti}"),
                (Some(n), None) => n.clone(),
                (None, Some(ti)) => ti.clone(),
                (None, None) => String::new(),
            };
            println!("  {:>4}  {estado:<20} {rotulo}", t.id);
        }
    }
}

// ---- Adjuntar ----

const SOLTAR: u8 = 0x1d; // Ctrl+]

static ORIGINAL: OnceLock<libc::termios> = OnceLock::new();

fn adjuntar(mut c: Client, id: u64) -> Result<()> {
    let (cols, rows) = tamano_terminal().unwrap_or((80, 24));
    c.pedir(Pedido::Adjuntar { id, cols, rows })?;
    let tx = c.sender();
    // Esta terminal tiene el foco mientras estamos adjuntados: cuenta como Mirar.
    tx.enviar(Pedido::Mirando { id: Some(id) })?;
    modo_crudo()?;

    thread::spawn(move || {
        while let Some(ev) = c.proximo_evento(None) {
            match ev {
                Evento::Salida { id: i, datos } if i == id => {
                    // Tomar el lock por cada escritura: `salir` también escribe en stdout.
                    let mut out = io::stdout().lock();
                    let _ = out.write_all(&datos);
                    let _ = out.flush();
                }
                Evento::Cerrada { id: i } if i == id => {
                    salir("\r\n[swarmctl: la Terminal fue cerrada]\r\n")
                }
                Evento::Apagando => salir("\r\n[swarmctl: swarmd se apagó]\r\n"),
                _ => {}
            }
        }
        salir("\r\n[swarmctl: se perdió la conexión con swarmd]\r\n");
    });

    let tx_tam = tx.clone();
    thread::spawn(move || {
        let mut ultimo = (cols, rows);
        loop {
            thread::sleep(Duration::from_millis(250));
            if let Some(t) = tamano_terminal()
                && t != ultimo
            {
                ultimo = t;
                let _ = tx_tam.enviar(Pedido::Redimensionar {
                    id,
                    cols: t.0,
                    rows: t.1,
                });
            }
        }
    });

    let mut buf = [0u8; 4096];
    let mut stdin = io::stdin().lock();
    loop {
        let n = stdin.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let datos = &buf[..n];
        if let Some(p) = datos.iter().position(|&b| b == SOLTAR) {
            if p > 0 {
                tx.enviar(Pedido::Entrada {
                    id,
                    datos: datos[..p].to_vec(),
                })?;
            }
            break;
        }
        tx.enviar(Pedido::Entrada {
            id,
            datos: datos.to_vec(),
        })?;
    }
    let _ = tx.enviar(Pedido::Mirando { id: None });
    let _ = tx.enviar(Pedido::Soltar { id });
    thread::sleep(Duration::from_millis(50));
    salir("\r\n[swarmctl: soltada]\r\n");
}

fn modo_crudo() -> Result<()> {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) != 0 {
            bail!("stdin no es una terminal");
        }
        let _ = ORIGINAL.set(t);
        libc::cfmakeraw(&mut t);
        libc::tcsetattr(0, libc::TCSANOW, &t);
    }
    Ok(())
}

fn salir(mensaje: &str) -> ! {
    if let Some(t) = ORIGINAL.get() {
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, t);
        }
    }
    // Volver a la pantalla normal y mostrar el cursor por si el programa los cambió.
    print!("\x1b[?1049l\x1b[?25h{mensaje}");
    let _ = io::stdout().flush();
    std::process::exit(0);
}

fn tamano_terminal() -> Option<(u16, u16)> {
    unsafe {
        let mut w: libc::winsize = std::mem::zeroed();
        (libc::ioctl(1, libc::TIOCGWINSZ, &mut w) == 0 && w.ws_col > 0)
            .then_some((w.ws_col, w.ws_row))
    }
}
