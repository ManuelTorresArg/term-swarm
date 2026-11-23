//! Lanzamiento del proceso de una Terminal sobre una pseudo-terminal propia.
//! Cada Terminal usa tres hilos: lectura, escritura y espera del proceso.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc as std_mpsc;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use swarm_ipc::TerminalId;
use tokio::sync::mpsc;

use crate::daemon::Cmd;

pub struct Proceso {
    master: Box<dyn MasterPty + Send>,
    entrada: std_mpsc::Sender<Vec<u8>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

pub fn tamano(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Abre el shell por defecto del usuario en `cwd` (T1). La salida y el fin del
/// proceso llegan al actor como `Cmd::Salida` y `Cmd::Termino`.
pub fn lanzar(id: TerminalId, cwd: &Path, tam: PtySize, tx: mpsc::Sender<Cmd>) -> Result<Proceso> {
    let par = native_pty_system().openpty(tam).context("openpty")?;

    let mut cmd = CommandBuilder::new_default_prog();
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("SWARM_TERMINAL", id.to_string());
    let mut hijo = par
        .slave
        .spawn_command(cmd)
        .context("no se pudo lanzar el shell")?;
    drop(par.slave);

    let mut lector = par.master.try_clone_reader()?;
    let mut escritor = par.master.take_writer()?;
    let killer = hijo.clone_killer();

    let tx_salida = tx.clone();
    thread::Builder::new()
        .name(format!("pty-{id}-lee"))
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match lector.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    // blocking_send da contrapresión: si el actor va atrasado, dejamos de leer la PTY.
                    Ok(n) => {
                        if tx_salida
                            .blocking_send(Cmd::Salida {
                                id,
                                datos: buf[..n].to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })?;

    let (entrada, rx_entrada) = std_mpsc::channel::<Vec<u8>>();
    thread::Builder::new()
        .name(format!("pty-{id}-escribe"))
        .spawn(move || {
            // Si el programa no lee su stdin, este hilo se bloquea y el actor no.
            while let Ok(datos) = rx_entrada.recv() {
                if escritor
                    .write_all(&datos)
                    .and_then(|_| escritor.flush())
                    .is_err()
                {
                    break;
                }
            }
        })?;

    thread::Builder::new()
        .name(format!("pty-{id}-espera"))
        .spawn(move || {
            let codigo = match hijo.wait() {
                Ok(s) => s.exit_code(),
                Err(e) => {
                    log::warn!("terminal {id}: error esperando el proceso: {e}");
                    1
                }
            };
            let _ = tx.blocking_send(Cmd::Termino { id, codigo });
        })?;

    Ok(Proceso {
        master: par.master,
        entrada,
        killer,
    })
}

impl Proceso {
    pub fn escribir(&self, datos: Vec<u8>) {
        let _ = self.entrada.send(datos);
    }

    pub fn tamano(&self) -> Option<PtySize> {
        self.master.get_size().ok()
    }

    pub fn redimensionar(&self, tam: PtySize) {
        if let Err(e) = self.master.resize(tam) {
            log::warn!("resize falló: {e}");
        }
    }

    /// Fuerza un redibujado sin cambiar el tamaño: SIGWINCH al grupo de primer plano (V4).
    pub fn forzar_redibujado(&self) {
        if let Some(pgid) = self.master.process_group_leader()
            && pgid > 0
        {
            unsafe {
                libc::killpg(pgid, libc::SIGWINCH);
            }
        }
    }

    pub fn matar(&mut self) {
        let _ = self.killer.kill();
    }
}
