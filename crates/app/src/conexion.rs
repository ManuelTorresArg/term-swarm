//! Puente entre el cliente IPC (síncrono, con su hilo lector) y el loop de GTK.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::thread;

use anyhow::Result;
use gtk::glib;
use swarm_ipc::{Client, Evento, MensajeServidor, Pedido, Sender, Valor};

type Respuesta = Box<dyn FnOnce(Result<Valor, String>)>;

pub struct Conexion {
    sender: Sender,
    pendientes: RefCell<HashMap<u64, Respuesta>>,
}

/// Lo que llega del daemon, antes de empezar a escucharlo.
pub struct Recepcion {
    previos: Vec<Evento>,
    rx: Receiver<MensajeServidor>,
}

impl Conexion {
    /// Se conecta al daemon, arrancándolo si hace falta (V2).
    pub fn conectar() -> Result<(Rc<Conexion>, Recepcion)> {
        let (sender, previos, rx) = Client::conectar_o_arrancar(None)?.en_partes();
        let c = Rc::new(Conexion {
            sender,
            pendientes: RefCell::default(),
        });
        Ok((c, Recepcion { previos, rx }))
    }

    /// Empieza a despachar en el loop de GTK. `al_cerrar` se llama si se pierde la conexión.
    pub fn escuchar(
        self: &Rc<Self>,
        r: Recepcion,
        al_evento: impl Fn(Evento) + 'static,
        al_cerrar: impl FnOnce() + 'static,
    ) {
        let (tx, rx) = async_channel::unbounded();
        let Recepcion { previos, rx: rx_hilo } = r;
        thread::Builder::new()
            .name("swarm-puente".into())
            .spawn(move || {
                for m in rx_hilo {
                    if tx.send_blocking(m).is_err() {
                        break;
                    }
                }
            })
            .expect("no se pudo crear el hilo puente");

        let yo = self.clone();
        glib::spawn_future_local(async move {
            for e in previos {
                al_evento(e);
            }
            while let Ok(m) = rx.recv().await {
                match m {
                    MensajeServidor::Evento { evento } => al_evento(evento),
                    MensajeServidor::Respuesta { req, resultado } => {
                        let cb = yo.pendientes.borrow_mut().remove(&req);
                        match (cb, resultado) {
                            (Some(cb), r) => cb(r),
                            (None, Err(e)) => eprintln!("swarm: el daemon respondió con error: {e}"),
                            (None, Ok(_)) => {}
                        }
                    }
                }
            }
            al_cerrar();
        });
    }

    /// Manda un Pedido sin esperar respuesta (los errores van a stderr).
    pub fn enviar(&self, p: Pedido) {
        if let Err(e) = self.sender.enviar(p) {
            eprintln!("swarm: no se pudo hablar con el daemon: {e}");
        }
    }

    /// Manda un Pedido y llama a `cb` con la Respuesta, en el loop de GTK.
    pub fn pedir(&self, p: Pedido, cb: impl FnOnce(Result<Valor, String>) + 'static) {
        match self.sender.enviar(p) {
            // La respuesta se procesa en este mismo hilo, así que no puede ganarle al insert.
            Ok(req) => {
                self.pendientes.borrow_mut().insert(req, Box::new(cb));
            }
            Err(e) => cb(Err(e.to_string())),
        }
    }
}
