//! swarm: la app de escritorio. Es un cliente delgado de swarmd (ADR 0004).

mod conexion;
mod dialogos;
mod ui;
mod vista;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::conexion::Conexion;
use crate::ui::Ui;

const APP_ID: &str = "dev.swarm.Swarm";

fn main() -> glib::ExitCode {
    // I1: una sola instancia. `swarm --mostrar <id>` (N3) se reenvía a la que ya corre.
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|app| {
        gtk::Window::set_default_icon_name(APP_ID);
        aplicar_tema(app);
    });
    let ui: Rc<RefCell<Option<Rc<Ui>>>> = Rc::default();

    app.connect_command_line(move |app, linea| {
        let args: Vec<String> = linea
            .arguments()
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        let mostrar = args
            .iter()
            .position(|a| a == "--mostrar")
            .and_then(|i| args.get(i + 1)?.parse().ok());

        let existente = ui.borrow().clone();
        let u = match existente {
            Some(u) => u,
            None => match Conexion::conectar() {
                Ok((conexion, recepcion)) => {
                    let u = Ui::nueva(app, conexion, recepcion);
                    *ui.borrow_mut() = Some(u.clone());
                    u
                }
                Err(e) => {
                    let app = app.clone();
                    let guarda = app.hold();
                    dialogos::avisar(None, "No se pudo conectar con swarmd", &format!("{e:#}"), move || {
                        drop(guarda);
                        app.quit();
                    });
                    return glib::ExitCode::FAILURE;
                }
            },
        };
        u.presentar();
        if let Some(id) = mostrar {
            u.pedir_mostrar(id);
        }
        glib::ExitCode::SUCCESS
    });

    app.run()
}

/// Usa el tema interno de GTK y los íconos de Adwaita, siguiendo la
/// preferencia clara/oscura del sistema, como las apps de GNOME. El tema GTK
/// del usuario puede no tener variante oscura para GTK4 (p. ej. Greybird con
/// `prefer-dark`), y eso deja texto e íconos claros sobre fondos claros.
fn aplicar_tema(app: &gtk::Application) {
    let Some(ajustes) = gtk::Settings::default() else {
        return;
    };
    ajustes.set_gtk_theme_name(Some("Default"));
    ajustes.set_gtk_icon_theme_name(Some("Adwaita"));

    const ESQUEMA: &str = "org.gnome.desktop.interface";
    let hay_esquema = gio::SettingsSchemaSource::default()
        .and_then(|s| s.lookup(ESQUEMA, true))
        .is_some();
    if !hay_esquema {
        return;
    }
    let sistema = gio::Settings::new(ESQUEMA);
    #[allow(deprecated)] // gtk-interface-color-scheme recién existe desde GTK 4.20
    let seguir = move |s: &gio::Settings| {
        ajustes.set_gtk_application_prefer_dark_theme(s.string("color-scheme") == "prefer-dark");
    };
    seguir(&sistema);
    sistema.connect_changed(Some("color-scheme"), move |s, _| seguir(s));
    // Mantener vivo el objeto para seguir recibiendo cambios.
    app.connect_shutdown(move |_| {
        let _ = &sistema;
    });
}
