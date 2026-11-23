//! swarmd: dueño de los procesos de las Terminales (ADR 0001).

mod daemon;
mod estructura;
mod notificar;
mod pty;
mod senales;

use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use swarm_ipc::{MensajeCliente, MensajeServidor, paths};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use crate::daemon::{Cmd, Daemon};

/// Mensajes pendientes por cliente antes de considerarlo trabado.
const COLA_CLIENTE: usize = 4096;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(correr())
}

async fn correr() -> Result<()> {
    let runtime = paths::runtime_dir();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&runtime)?;
    let socket = paths::socket();

    // Una sola instancia: si alguien atiende el socket, ya hay un swarmd.
    if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        bail!("ya hay un swarmd corriendo en {}", socket.display());
    }
    let _ = fs::remove_file(&socket);
    let listener =
        UnixListener::bind(&socket).with_context(|| format!("bind {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    log::info!("escuchando en {}", socket.display());

    let (tx, rx) = mpsc::channel::<Cmd>(1024);
    let daemon = Daemon::nuevo(tx.clone(), &paths::state_dir());

    let tx_tick = tx.clone();
    tokio::spawn(async move {
        let mut intervalo = tokio::time::interval(Duration::from_millis(250));
        loop {
            intervalo.tick().await;
            if tx_tick.send(Cmd::Tick).await.is_err() {
                break;
            }
        }
    });

    let tx_senal = tx.clone();
    tokio::spawn(async move {
        let (Ok(mut term), Ok(mut int)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        let _ = tx_senal.send(Cmd::Apagar).await;
    });

    let tx_acepta = tx.clone();
    tokio::spawn(async move {
        let mut proximo: u64 = 1;
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let cliente = proximo;
                    proximo += 1;
                    atender(cliente, stream, tx_acepta.clone()).await;
                }
                Err(e) => log::warn!("accept: {e}"),
            }
        }
    });

    drop(tx);
    daemon.correr(rx).await;

    // Primero liberar el socket: un cliente que llegue ahora tiene que poder
    // arrancar un daemon nuevo sin que este se lo borre después.
    let _ = fs::remove_file(&socket);
    // Dar un momento a que salgan las últimas respuestas (p. ej. la de Apagar).
    tokio::time::sleep(Duration::from_millis(100)).await;
    log::info!("chau");
    Ok(())
}

async fn atender(cliente: u64, stream: UnixStream, tx: mpsc::Sender<Cmd>) {
    let (lectura, escritura) = stream.into_split();
    let (tx_cliente, mut rx_cliente) = mpsc::channel::<MensajeServidor>(COLA_CLIENTE);
    if tx
        .send(Cmd::Conectar {
            cliente,
            tx: tx_cliente,
        })
        .await
        .is_err()
    {
        return;
    }

    tokio::spawn(async move {
        let mut w = BufWriter::new(escritura);
        while let Some(m) = rx_cliente.recv().await {
            let Ok(mut linea) = serde_json::to_vec(&m) else {
                continue;
            };
            linea.push(b'\n');
            if w.write_all(&linea).await.is_err() {
                break;
            }
            // Vaciar solo cuando no hay más en cola: agrupa ráfagas de salida.
            if rx_cliente.is_empty() && w.flush().await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        let mut lineas = BufReader::new(lectura).lines();
        while let Ok(Some(linea)) = lineas.next_line().await {
            match serde_json::from_str::<MensajeCliente>(&linea) {
                Ok(MensajeCliente { req, pedido }) => {
                    if tx
                        .send(Cmd::Pedido {
                            cliente,
                            req,
                            pedido,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(e) => log::warn!("cliente {cliente}: mensaje inválido: {e}"),
            }
        }
        let _ = tx.send(Cmd::Desconectar { cliente }).await;
    });
}
