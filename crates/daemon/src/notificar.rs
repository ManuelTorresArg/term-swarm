//! Notificaciones de escritorio (N1–N3). Las manda el daemon, así llegan
//! aunque la ventana de swarm esté cerrada.
//!
//! Cada Terminal tiene a lo sumo una notificación viva: una nueva reemplaza
//! a la anterior en vez de apilarse. Hacer clic lanza `swarm --mostrar <id>`;
//! como la app es de instancia única, eso enfoca la ventana existente.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use swarm_ipc::TerminalId;

pub struct Notificador {
    activo: bool,
    /// Terminal → id de su notificación viva (la que tiene un hilo esperando el clic).
    vivas: Arc<Mutex<HashMap<TerminalId, u32>>>,
}

impl Notificador {
    /// `SWARM_NO_NOTIFICAR=1` las desactiva (lo usan los tests).
    pub fn desde_entorno() -> Self {
        Notificador {
            activo: std::env::var_os("SWARM_NO_NOTIFICAR").is_none(),
            vivas: Arc::default(),
        }
    }

    pub fn enviar(&self, terminal: TerminalId, resumen: String, cuerpo: String) {
        if !self.activo {
            return;
        }
        let vivas = self.vivas.clone();
        // Las llamadas D-Bus son bloqueantes; que no frenen al actor.
        thread::spawn(move || {
            let mut n = notify_rust::Notification::new();
            n.appname("swarm")
                .summary(&resumen)
                .body(&cuerpo)
                .icon("dev.swarm.Swarm")
                // Así GNOME la asocia con la app instalada (nombre e ícono).
                .hint(notify_rust::Hint::DesktopEntry("dev.swarm.Swarm".into()))
                .action("default", "Mostrar");
            let previa = vivas.lock().unwrap().get(&terminal).copied();
            if let Some(id) = previa {
                // Ya hay un hilo esperando el clic de esa notificación: solo la reemplazamos.
                n.id(id);
                if let Err(e) = n.show() {
                    log::warn!("no se pudo mostrar la notificación: {e}");
                }
                return;
            }
            let handle = match n.show() {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("no se pudo mostrar la notificación: {e}");
                    return;
                }
            };
            let id = handle.id();
            vivas.lock().unwrap().insert(terminal, id);
            handle.wait_for_action(|accion| {
                if accion == "default" {
                    mostrar_en_app(terminal);
                }
            });
            let mut v = vivas.lock().unwrap();
            if v.get(&terminal) == Some(&id) {
                v.remove(&terminal);
            }
        });
    }
}

/// N3: abre (o enfoca) la app mostrando la Terminal.
fn mostrar_en_app(terminal: TerminalId) {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.join("swarm")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("swarm"));
    match Command::new(&exe)
        .arg("--mostrar")
        .arg(terminal.to_string())
        .stdin(Stdio::null())
        .spawn()
    {
        // Si es la primera instancia, la app vive mucho: esperarla en otro hilo
        // para no retener este (que tiene que liberar la notificación).
        Ok(mut hijo) => {
            thread::spawn(move || hijo.wait());
        }
        Err(e) => log::warn!("no se pudo abrir {}: {e}", exe.display()),
    }
}
