use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub type ProyectoId = u64;
pub type TerminalId = u64;

/// Versión del protocolo; cambia cuando un mensaje cambia de forma incompatible.
pub const VERSION_PROTOCOLO: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proyecto {
    pub id: ProyectoId,
    pub nombre: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "estado", rename_all = "snake_case")]
pub enum Estado {
    Activa,
    Quieta,
    PideAtencion,
    Terminada { codigo: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terminal {
    pub id: TerminalId,
    /// `None` = pertenece a Sueltas.
    pub proyecto: Option<ProyectoId>,
    pub nombre: Option<String>,
    pub titulo: Option<String>,
    pub cwd: PathBuf,
    pub estado: Estado,
    /// Terminada y todavía no Mirada desde que terminó.
    pub terminada_sin_mirar: bool,
    /// Comando que se escribe en el shell al reabrir la Terminal (R1).
    #[serde(default)]
    pub reanudacion: Option<String>,
}

/// La Estructura completa más el estado vivo de cada Terminal.
/// `terminales` viene en orden de presentación (I5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instantanea {
    pub proyectos: Vec<Proyecto>,
    pub terminales: Vec<Terminal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Pedido {
    Hola {
        version: u32,
    },
    Estructura,
    CrearProyecto {
        nombre: String,
        dir: PathBuf,
    },
    /// Cierra también todas sus Terminales (P3).
    BorrarProyecto {
        id: ProyectoId,
    },
    CrearTerminal {
        proyecto: Option<ProyectoId>,
        cols: u16,
        rows: u16,
    },
    CerrarTerminal {
        id: TerminalId,
    },
    RenombrarTerminal {
        id: TerminalId,
        nombre: Option<String>,
    },
    /// Pone o borra la Reanudación de una Terminal (R1–R4).
    FijarReanudacion {
        id: TerminalId,
        comando: Option<String>,
    },
    /// Posición dentro de su grupo (Proyecto o Sueltas).
    MoverTerminal {
        id: TerminalId,
        posicion: usize,
    },
    /// Empieza a recibir la Salida de la Terminal; primero llega lo repetido del buffer.
    Adjuntar {
        id: TerminalId,
        cols: u16,
        rows: u16,
    },
    Soltar {
        id: TerminalId,
    },
    Entrada {
        id: TerminalId,
        #[serde(with = "b64")]
        datos: Vec<u8>,
    },
    Redimensionar {
        id: TerminalId,
        cols: u16,
        rows: u16,
    },
    /// La Terminal que este cliente está Mirando (visible + ventana enfocada), o ninguna.
    Mirando {
        id: Option<TerminalId>,
    },
    /// Mata todos los procesos y apaga el daemon (V5). La Estructura se conserva.
    Apagar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MensajeCliente {
    pub req: u64,
    #[serde(flatten)]
    pub pedido: Pedido,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "valor", rename_all = "snake_case")]
pub enum Valor {
    Nada,
    Hola { version: u32 },
    Estructura(Instantanea),
    ProyectoCreado { id: ProyectoId },
    TerminalCreada { id: TerminalId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "evento", rename_all = "snake_case")]
pub enum Evento {
    /// Cambió la Estructura (alta, baja, renombre u orden).
    Estructura(Instantanea),
    /// Cambió el estado o el Título de una Terminal.
    TerminalActualizada(Terminal),
    /// Bytes de una Terminal adjuntada.
    Salida {
        id: TerminalId,
        #[serde(with = "b64")]
        datos: Vec<u8>,
    },
    /// La Terminal fue cerrada mientras estaba adjuntada.
    Cerrada {
        id: TerminalId,
    },
    Apagando,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum MensajeServidor {
    Respuesta {
        req: u64,
        resultado: Result<Valor, String>,
    },
    Evento {
        evento: Evento,
    },
}

mod b64 {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(datos: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(datos))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD.decode(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_y_vuelta() {
        let m = MensajeCliente {
            req: 7,
            pedido: Pedido::Entrada {
                id: 3,
                datos: b"ls\r\x07".to_vec(),
            },
        };
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<MensajeCliente>(&json).unwrap(), m);

        let s = MensajeServidor::Respuesta {
            req: 1,
            resultado: Ok(Valor::TerminalCreada { id: 2 }),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<MensajeServidor>(&json).unwrap(), s);
    }
}
