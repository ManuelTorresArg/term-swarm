//! Detección de las señales estándar de terminal en la salida (ADR 0002):
//! BEL y notificaciones OSC 9 / OSC 777 → Pide atención; OSC 0 / OSC 2 → Título.

use vte::{Parser, Perform};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Senales {
    /// `Some(mensaje)` si el programa pidió atención; el mensaje puede venir vacío (BEL).
    pub atencion: Option<Option<String>>,
    /// Último Título declarado en este tramo; `Some(None)` = lo borró.
    pub titulo: Option<Option<String>>,
}

/// Parser con estado: una secuencia puede llegar partida entre dos lecturas.
#[derive(Default)]
pub struct Detector {
    parser: Parser,
}

impl Detector {
    pub fn procesar(&mut self, datos: &[u8]) -> Senales {
        let mut s = Senales::default();
        self.parser.advance(&mut Recolector(&mut s), datos);
        s
    }
}

struct Recolector<'a>(&'a mut Senales);

impl Recolector<'_> {
    fn atencion(&mut self, mensaje: Option<String>) {
        // Si en un mismo tramo hay un BEL y una notificación con texto, gana el texto.
        match &mut self.0.atencion {
            Some(m @ None) => *m = mensaje,
            Some(Some(_)) => {}
            n @ None => *n = Some(mensaje),
        }
    }
}

fn texto(partes: &[&[u8]]) -> Option<String> {
    let t = partes
        .iter()
        .map(|p| String::from_utf8_lossy(p))
        .collect::<Vec<_>>()
        .join(";");
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

impl Perform for Recolector<'_> {
    fn execute(&mut self, byte: u8) {
        // Un BEL que cierra una OSC no pasa por acá: lo consume el parser.
        if byte == 0x07 {
            self.atencion(None);
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let Some((&cmd, resto)) = params.split_first() else {
            return;
        };
        match cmd {
            b"0" | b"2" => self.0.titulo = Some(texto(resto)),
            // OSC 9;4 es la barra de progreso de ConEmu/Windows Terminal, no una notificación.
            b"9" if resto.first() == Some(&&b"4"[..]) => {}
            b"9" => self.atencion(texto(resto)),
            b"777" if resto.first() == Some(&&b"notify"[..]) => {
                let titulo = resto.get(1).and_then(|t| texto(&[t]));
                let cuerpo = resto.get(2..).and_then(texto);
                let mensaje = match (titulo, cuerpo) {
                    (Some(t), Some(c)) => Some(format!("{t}: {c}")),
                    (t, c) => t.or(c),
                };
                self.atencion(mensaje);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(datos: &[u8]) -> Senales {
        Detector::default().procesar(datos)
    }

    #[test]
    fn bel_suelto_pide_atencion() {
        assert_eq!(s(b"hola\x07").atencion, Some(None));
    }

    #[test]
    fn bel_que_cierra_osc_no_pide_atencion() {
        let r = s(b"\x1b]0;mi tarea\x07");
        assert_eq!(r.atencion, None);
        assert_eq!(r.titulo, Some(Some("mi tarea".into())));
    }

    #[test]
    fn titulo_con_st_y_con_punto_y_coma() {
        assert_eq!(s(b"\x1b]2;a;b\x1b\\").titulo, Some(Some("a;b".into())));
        assert_eq!(s(b"\x1b]2;\x07").titulo, Some(None));
    }

    #[test]
    fn osc9_y_osc777() {
        assert_eq!(
            s(b"\x1b]9;Listo para revisar\x07").atencion,
            Some(Some("Listo para revisar".into()))
        );
        assert_eq!(
            s(b"\x1b]777;notify;Claude;Necesito permiso\x1b\\").atencion,
            Some(Some("Claude: Necesito permiso".into()))
        );
    }

    #[test]
    fn osc9_4_es_progreso() {
        assert_eq!(s(b"\x1b]9;4;1;50\x07").atencion, None);
    }

    #[test]
    fn secuencia_partida_entre_lecturas() {
        let mut d = Detector::default();
        assert_eq!(d.procesar(b"\x1b]0;mit"), Senales::default());
        assert_eq!(d.procesar(b"ad\x07").titulo, Some(Some("mitad".into())));
    }
}
