//! Notificaciones de escritorio (N1, N2). Las manda el daemon, así llegan
//! aunque la ventana de swarm esté cerrada.

use std::thread;

pub struct Notificador {
    activo: bool,
}

impl Notificador {
    /// `SWARM_NO_NOTIFICAR=1` las desactiva (lo usan los tests).
    pub fn desde_entorno() -> Self {
        Notificador {
            activo: std::env::var_os("SWARM_NO_NOTIFICAR").is_none(),
        }
    }

    pub fn enviar(&self, resumen: String, cuerpo: String) {
        if !self.activo {
            return;
        }
        // La llamada D-Bus es bloqueante; que no frene al actor.
        thread::spawn(move || {
            if let Err(e) = notify_rust::Notification::new()
                .appname("swarm")
                .summary(&resumen)
                .body(&cuerpo)
                .icon("utilities-terminal")
                .show()
            {
                log::warn!("no se pudo mostrar la notificación: {e}");
            }
        });
    }
}
