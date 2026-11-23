//! Protocolo entre `swarmd` y sus clientes (la app y `swarmctl`).
//!
//! Transporte: un socket Unix con un mensaje JSON por línea. Los bytes de
//! terminal viajan en base64.

pub mod client;
pub mod paths;
pub mod proto;

pub use client::{Client, Sender};
pub use proto::*;
