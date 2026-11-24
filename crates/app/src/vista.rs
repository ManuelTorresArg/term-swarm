//! Una Terminal en pantalla: un widget VTE sin pseudo-terminal propia. La
//! salida llega del daemon con `feed` y lo que se tipea vuelve como Entrada.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, pango};
use swarm_ipc::{Estado, Pedido, Terminal, TerminalId};
use vte::prelude::*;

use crate::conexion::Conexion;

pub struct Vista {
    pub raiz: gtk::Box,
    pub term: vte::Terminal,
    barra: gtk::Box,
    barra_texto: gtk::Label,
    /// Rótulo de la Terminal; solo se ve cuando comparte la ventana con otras.
    pub cabecera: gtk::Label,
    /// Último tamaño (cols, rows) que le informamos al daemon.
    pub tam_informado: Cell<(i64, i64)>,
}

impl Vista {
    pub fn nueva(
        id: TerminalId,
        conexion: &Rc<Conexion>,
        al_cerrar: impl Fn() + 'static,
    ) -> Rc<Vista> {
        let term = vte::Terminal::new();
        term.set_hexpand(true);
        term.set_vexpand(true);
        term.set_scrollback_lines(10_000);
        term.set_mouse_autohide(true);
        term.set_bold_is_bright(true);
        if let Some(f) = fuente_monoespaciada() {
            term.set_font(Some(&pango::FontDescription::from_string(&f)));
        }
        aplicar_paleta(&term);

        let c = conexion.clone();
        term.connect_commit(move |_, texto, _| {
            c.enviar(Pedido::Entrada {
                id,
                datos: texto.as_bytes().to_vec(),
            });
        });

        // I6: la Terminada sigue mostrando su salida, con una barra arriba.
        let barra_texto = gtk::Label::new(None);
        barra_texto.set_hexpand(true);
        barra_texto.set_xalign(0.0);
        let cerrar = gtk::Button::with_label("Cerrar Terminal");
        cerrar.connect_clicked(move |_| al_cerrar());
        let barra = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        barra.add_css_class("barra-terminada");
        barra.append(&barra_texto);
        barra.append(&cerrar);
        barra.set_visible(false);

        let barra_desplazamiento =
            gtk::Scrollbar::new(gtk::Orientation::Vertical, term.vadjustment().as_ref());
        let fila = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fila.append(&term);
        fila.append(&barra_desplazamiento);

        let cabecera = gtk::Label::new(None);
        cabecera.set_xalign(0.0);
        cabecera.set_ellipsize(pango::EllipsizeMode::End);
        cabecera.add_css_class("cabecera");
        cabecera.set_visible(false);

        let raiz = gtk::Box::new(gtk::Orientation::Vertical, 0);
        raiz.add_css_class("vista");
        raiz.append(&cabecera);
        raiz.append(&barra);
        raiz.append(&fila);

        Rc::new(Vista {
            raiz,
            term,
            barra,
            barra_texto,
            cabecera,
            tam_informado: Cell::new((0, 0)),
        })
    }

    pub fn actualizar(&self, t: &Terminal) {
        self.cabecera.set_text(&crate::ui::rotulo(t));
        match t.estado {
            Estado::Terminada { codigo } => {
                self.barra_texto
                    .set_text(&format!("Terminada · código de salida {codigo}"));
                self.barra.set_visible(true);
                self.term.set_input_enabled(false);
            }
            _ => {
                self.barra.set_visible(false);
                self.term.set_input_enabled(true);
            }
        }
    }

    /// Tamaño actual de la grilla, si ya fue calculado.
    pub fn tam(&self) -> Option<(i64, i64)> {
        let t = (self.term.column_count(), self.term.row_count());
        (t.0 > 0 && t.1 > 0).then_some(t)
    }
}

/// La fuente monoespaciada del sistema (la misma que usa GNOME Terminal por defecto).
fn fuente_monoespaciada() -> Option<String> {
    const ESQUEMA: &str = "org.gnome.desktop.interface";
    gio::SettingsSchemaSource::default()?.lookup(ESQUEMA, true)?;
    Some(
        gio::Settings::new(ESQUEMA)
            .string("monospace-font-name")
            .to_string(),
    )
}

fn aplicar_paleta(term: &vte::Terminal) {
    let rgba = |s: &str| gdk::RGBA::parse(s).expect("color válido");
    let paleta: Vec<gdk::RGBA> = [
        "#171421", "#c01c28", "#26a269", "#a2734c", "#12488b", "#a347ba", "#2aa1b3", "#d0cfcc",
        "#5e5c64", "#f66151", "#33d17a", "#e9ad0c", "#2a7bde", "#c061cb", "#33c7de", "#ffffff",
    ]
    .into_iter()
    .map(rgba)
    .collect();
    let refs: Vec<&gdk::RGBA> = paleta.iter().collect();
    term.set_colors(Some(&rgba("#d0cfcc")), Some(&rgba("#171421")), &refs);
}
