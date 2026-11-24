use gtk::prelude::*;
use gtk::{gio, glib};

/// Pide confirmación; `si` corre solo si el usuario elige `accion`.
pub fn confirmar(padre: &impl IsA<gtk::Window>, mensaje: &str, detalle: &str, accion: &str, si: impl FnOnce() + 'static) {
    let d = gtk::AlertDialog::builder()
        .message(mensaje)
        .detail(detalle)
        .buttons(["Cancelar", accion])
        .cancel_button(0)
        .default_button(0)
        .modal(true)
        .build();
    d.choose(Some(padre), None::<&gio::Cancellable>, move |r| {
        if r == Ok(1) {
            si();
        }
    });
}

pub fn avisar(padre: Option<&gtk::Window>, mensaje: &str, detalle: &str, despues: impl FnOnce() + 'static) {
    let d = gtk::AlertDialog::builder()
        .message(mensaje)
        .detail(detalle)
        .modal(true)
        .build();
    d.choose(padre, None::<&gio::Cancellable>, move |_| despues());
}

/// Pide un texto en un diálogo chico. `ok` recibe el texto sin espacios a los costados (puede venir vacío).
pub fn pedir_texto(
    padre: &impl IsA<gtk::Window>,
    titulo: &str,
    etiqueta: &str,
    inicial: &str,
    boton: &str,
    ok: impl Fn(String) + 'static,
) {
    let ventana = gtk::Window::builder()
        .title(titulo)
        .transient_for(padre)
        .modal(true)
        .resizable(false)
        .default_width(360)
        .build();

    let entrada = gtk::Entry::builder().text(inicial).activates_default(true).build();
    let cancelar = gtk::Button::with_label("Cancelar");
    let aceptar = gtk::Button::with_label(boton);
    aceptar.add_css_class("suggested-action");

    let botones = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    botones.set_halign(gtk::Align::End);
    botones.append(&cancelar);
    botones.append(&aceptar);

    let caja = gtk::Box::new(gtk::Orientation::Vertical, 12);
    caja.set_margin_top(18);
    caja.set_margin_bottom(18);
    caja.set_margin_start(18);
    caja.set_margin_end(18);
    caja.append(&gtk::Label::builder().label(etiqueta).xalign(0.0).build());
    caja.append(&entrada);
    caja.append(&botones);
    ventana.set_child(Some(&caja));
    ventana.set_default_widget(Some(&aceptar));

    cancelar.connect_clicked(glib::clone!(
        #[weak]
        ventana,
        move |_| ventana.close()
    ));
    aceptar.connect_clicked(glib::clone!(
        #[weak]
        ventana,
        #[weak]
        entrada,
        move |_| {
            ok(entrada.text().trim().to_string());
            ventana.close();
        }
    ));
    let esc = gtk::EventControllerKey::new();
    esc.connect_key_pressed(glib::clone!(
        #[weak]
        ventana,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, tecla, _, _| {
            if tecla == gtk::gdk::Key::Escape {
                ventana.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    ));
    ventana.add_controller(esc);
    ventana.present();
    entrada.select_region(0, -1);
    entrada.grab_focus();
}
