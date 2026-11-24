//! La ventana: sidebar con Proyectos y Terminales (I2–I5) y la Terminal visible.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use vte::prelude::*;
use gtk::{gio, glib};
use swarm_ipc::{Estado, Evento, Instantanea, Pedido, ProyectoId, Terminal, TerminalId, Valor};

use crate::conexion::{Conexion, Recepcion};
use crate::dialogos;
use crate::vista::Vista;

/// Un grupo del sidebar: un Proyecto, o Sueltas (`None`).
type Grupo = Option<ProyectoId>;

type AccionTerminal = fn(&Rc<Ui>, TerminalId);

enum Fila {
    Grupo(Grupo),
    Terminal(TerminalId),
}

struct FilaTerminal {
    row: gtk::ListBoxRow,
    indicador: gtk::Label,
    linea1: gtk::Label,
    linea2: gtk::Label,
}

#[derive(Default)]
struct Estado_ {
    inst: Instantanea,
    colapsados: HashSet<Grupo>,
    /// Paralelo a las filas del ListBox.
    filas: Vec<Fila>,
    filas_terminal: HashMap<TerminalId, FilaTerminal>,
    contadores: HashMap<Grupo, gtk::Label>,
    vistas: HashMap<TerminalId, Rc<Vista>>,
    visible: Option<TerminalId>,
    /// Último `Mirando` enviado al daemon.
    mirando: Option<Option<TerminalId>>,
    /// Mientras se reconstruye el sidebar, ignorar `row-selected`.
    reconstruyendo: bool,
    /// `--mostrar` que llegó antes de tener la Estructura.
    mostrar_pendiente: Option<TerminalId>,
    cargada: bool,
    saliendo: bool,
}

pub struct Ui {
    app: gtk::Application,
    ventana: gtk::ApplicationWindow,
    conexion: Rc<Conexion>,
    lista: gtk::ListBox,
    pila: gtk::Stack,
    st: RefCell<Estado_>,
}

impl Ui {
    pub fn nueva(app: &gtk::Application, conexion: Rc<Conexion>, recepcion: Recepcion) -> Rc<Ui> {
        cargar_css();
        let ventana = gtk::ApplicationWindow::builder()
            .application(app)
            .title("swarm")
            .default_width(1200)
            .default_height(760)
            .build();

        let lista = gtk::ListBox::new();
        lista.set_selection_mode(gtk::SelectionMode::Single);
        lista.add_css_class("navigation-sidebar");
        let lateral = gtk::ScrolledWindow::builder()
            .child(&lista)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(240)
            .build();

        let pila = gtk::Stack::new();
        let vacio = gtk::Label::new(Some(
            "Elegí una Terminal del costado\no creá una con Ctrl+Shift+T",
        ));
        vacio.set_justify(gtk::Justification::Center);
        vacio.add_css_class("dim-label");
        pila.add_named(&vacio, Some("vacio"));

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&lateral));
        paned.set_end_child(Some(&pila));
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);
        paned.set_position(260);
        ventana.set_child(Some(&paned));

        let ui = Rc::new(Ui {
            app: app.clone(),
            ventana,
            conexion: conexion.clone(),
            lista,
            pila,
            st: RefCell::default(),
        });
        ui.armar_barra();
        ui.armar_acciones();
        ui.conectar_senales();

        conexion.escuchar(
            recepcion,
            glib::clone!(
                #[weak]
                ui,
                move |e| ui.evento(e)
            ),
            glib::clone!(
                #[weak]
                ui,
                move || ui.conexion_perdida()
            ),
        );
        conexion.pedir(
            Pedido::Estructura,
            glib::clone!(
                #[weak]
                ui,
                move |r| {
                    if let Ok(Valor::Estructura(i)) = r {
                        ui.aplicar_estructura(i);
                        ui.st.borrow_mut().cargada = true;
                        let pendiente = ui.st.borrow_mut().mostrar_pendiente.take();
                        if let Some(id) = pendiente {
                            ui.mostrar(id);
                        }
                    }
                }
            ),
        );
        ui
    }

    pub fn presentar(&self) {
        self.ventana.present();
    }

    /// `swarm-term --mostrar <id>` (N3).
    pub fn pedir_mostrar(self: &Rc<Self>, id: TerminalId) {
        if self.st.borrow().cargada {
            self.mostrar(id);
        } else {
            self.st.borrow_mut().mostrar_pendiente = Some(id);
        }
    }

    // ---- Armado ----

    fn armar_barra(self: &Rc<Self>) {
        let barra = gtk::HeaderBar::new();

        let proyecto = gtk::Button::from_icon_name("folder-new-symbolic");
        proyecto.set_tooltip_text(Some("Nuevo Proyecto"));
        proyecto.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.nuevo_proyecto()
        ));

        let terminal = gtk::Button::from_icon_name("tab-new-symbolic");
        terminal.set_tooltip_text(Some("Nueva Terminal (Ctrl+Shift+T)"));
        terminal.set_action_name(Some("win.nueva-terminal"));

        let atencion = gtk::Button::from_icon_name("go-jump-symbolic");
        atencion.set_tooltip_text(Some("Ir a la próxima que pide atención (Ctrl+Shift+A)"));
        atencion.set_action_name(Some("win.siguiente-atencion"));

        let salir = gtk::Button::with_label("Salir y cerrar todas las Terminales");
        salir.add_css_class("flat");
        salir.add_css_class("destructive-action");
        let menu = gtk::Popover::new();
        menu.set_child(Some(&salir));
        salir.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            #[weak]
            menu,
            move |_| {
                menu.popdown();
                ui.salir_y_cerrar_todo();
            }
        ));
        let boton_menu = gtk::MenuButton::new();
        boton_menu.set_icon_name("open-menu-symbolic");
        boton_menu.set_popover(Some(&menu));

        barra.pack_start(&proyecto);
        barra.pack_start(&terminal);
        barra.pack_start(&atencion);
        barra.pack_end(&boton_menu);
        self.ventana.set_titlebar(Some(&barra));
    }

    fn armar_acciones(self: &Rc<Self>) {
        let accion = |nombre: &str, teclas: &[&str], f: fn(&Rc<Ui>)| {
            let a = gio::SimpleAction::new(nombre, None);
            a.connect_activate(glib::clone!(
                #[weak(rename_to = ui)]
                self,
                move |_, _| f(&ui)
            ));
            self.ventana.add_action(&a);
            self.app.set_accels_for_action(&format!("win.{nombre}"), teclas);
        };
        accion("siguiente-atencion", &["<Control><Shift>a"], |ui| ui.siguiente_atencion());
        accion("nueva-terminal", &["<Control><Shift>t"], |ui| {
            let g = ui.grupo_actual();
            ui.nueva_terminal(g)
        });
        accion("siguiente", &["<Control>Page_Down"], |ui| ui.saltar(1));
        accion("anterior", &["<Control>Page_Up"], |ui| ui.saltar(-1));
        accion("copiar", &["<Control><Shift>c"], |ui| {
            if let Some(v) = ui.vista_visible()
                && v.term.has_selection()
            {
                v.term.copy_clipboard_format(vte::Format::Text);
            }
        });
        accion("pegar", &["<Control><Shift>v"], |ui| {
            if let Some(v) = ui.vista_visible() {
                v.term.paste_clipboard();
            }
        });
    }

    fn conectar_senales(self: &Rc<Self>) {
        self.lista.connect_row_selected(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_, row| {
                let Some(row) = row else { return };
                let id = {
                    let st = ui.st.borrow();
                    if st.reconstruyendo {
                        return;
                    }
                    match st.filas.get(row.index() as usize) {
                        Some(Fila::Terminal(id)) => *id,
                        _ => return,
                    }
                };
                ui.mostrar(id);
            }
        ));
        self.lista.connect_row_activated(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_, row| {
                let grupo = match ui.st.borrow().filas.get(row.index() as usize) {
                    Some(Fila::Grupo(g)) => *g,
                    _ => return,
                };
                {
                    let mut st = ui.st.borrow_mut();
                    if !st.colapsados.remove(&grupo) {
                        st.colapsados.insert(grupo);
                    }
                }
                ui.reconstruir_lista();
            }
        ));
        // E2: Mirar = visible + ventana enfocada.
        self.ventana.connect_is_active_notify(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.actualizar_mirando()
        ));
        // El VTE no tiene PTY propia: informamos su tamaño al daemon cuando cambia.
        glib::timeout_add_local(
            Duration::from_millis(150),
            glib::clone!(
                #[weak(rename_to = ui)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    ui.informar_tamano();
                    glib::ControlFlow::Continue
                }
            ),
        );
    }

    // ---- Eventos del daemon ----

    fn evento(self: &Rc<Self>, e: Evento) {
        match e {
            Evento::Estructura(i) => self.aplicar_estructura(i),
            Evento::TerminalActualizada(t) => self.aplicar_terminal(t),
            Evento::Salida { id, datos } => {
                let v = self.st.borrow().vistas.get(&id).cloned();
                if let Some(v) = v {
                    v.term.feed(&datos);
                }
            }
            Evento::Cerrada { .. } => {} // llega también la Estructura nueva
            Evento::Apagando => {
                self.st.borrow_mut().saliendo = true;
                self.app.quit();
            }
        }
    }

    fn conexion_perdida(self: &Rc<Self>) {
        if self.st.borrow().saliendo {
            return;
        }
        let app = self.app.clone();
        dialogos::avisar(
            Some(self.ventana.upcast_ref()),
            "Se perdió la conexión con swarmd",
            "Las Terminales siguen en la Estructura; al volver a abrir swarm se reconecta o se reabren.",
            move || app.quit(),
        );
    }

    fn aplicar_estructura(self: &Rc<Self>, inst: Instantanea) {
        let quitadas: Vec<Rc<Vista>> = {
            let mut st = self.st.borrow_mut();
            let vivas: HashSet<TerminalId> = inst.terminales.iter().map(|t| t.id).collect();
            let ids: Vec<TerminalId> = st.vistas.keys().copied().filter(|id| !vivas.contains(id)).collect();
            let quitadas = ids.iter().filter_map(|id| st.vistas.remove(id)).collect();
            if st.visible.is_some_and(|v| !vivas.contains(&v)) {
                st.visible = None;
            }
            let proyectos: HashSet<Grupo> = inst.proyectos.iter().map(|p| Some(p.id)).collect();
            st.colapsados.retain(|g| g.is_none() || proyectos.contains(g));
            st.inst = inst;
            quitadas
        };
        for v in quitadas {
            self.pila.remove(&v.raiz);
        }
        if self.st.borrow().visible.is_none() {
            self.pila.set_visible_child_name("vacio");
        }
        // Las vistas existentes pueden haber cambiado de estado mientras no había ventana.
        let pares: Vec<(Rc<Vista>, Terminal)> = {
            let st = self.st.borrow();
            st.inst
                .terminales
                .iter()
                .filter_map(|t| Some((st.vistas.get(&t.id)?.clone(), t.clone())))
                .collect()
        };
        for (v, t) in pares {
            v.actualizar(&t);
        }
        self.reconstruir_lista();
        self.actualizar_mirando();
    }

    fn aplicar_terminal(self: &Rc<Self>, t: Terminal) {
        let vista = {
            let mut st = self.st.borrow_mut();
            let Some(pos) = st.inst.terminales.iter().position(|x| x.id == t.id) else {
                return;
            };
            st.inst.terminales[pos] = t.clone();
            if let Some(f) = st.filas_terminal.get(&t.id) {
                pintar_fila(f, &t);
            }
            pintar_contador(&st, t.proyecto);
            st.vistas.get(&t.id).cloned()
        };
        if let Some(v) = vista {
            v.actualizar(&t);
        }
    }

    // ---- Sidebar ----

    fn reconstruir_lista(self: &Rc<Self>) {
        self.st.borrow_mut().reconstruyendo = true;
        while let Some(hijo) = self.lista.first_child() {
            self.lista.remove(&hijo);
        }
        let (grupos, colapsados) = {
            let mut st = self.st.borrow_mut();
            st.filas.clear();
            st.filas_terminal.clear();
            st.contadores.clear();
            let mut grupos: Vec<(Grupo, String, Option<PathBuf>)> = st
                .inst
                .proyectos
                .iter()
                .map(|p| (Some(p.id), p.nombre.clone(), Some(p.dir.clone())))
                .collect();
            grupos.push((None, "Sueltas".into(), None));
            (grupos, st.colapsados.clone())
        };

        for (grupo, nombre, dir) in grupos {
            let colapsado = colapsados.contains(&grupo);
            let row = self.fila_grupo(grupo, &nombre, dir, colapsado);
            self.lista.append(&row);
            self.st.borrow_mut().filas.push(Fila::Grupo(grupo));
            if colapsado {
                continue;
            }
            let suyas: Vec<Terminal> = self
                .st
                .borrow()
                .inst
                .terminales
                .iter()
                .filter(|t| t.proyecto == grupo)
                .cloned()
                .collect();
            for t in suyas {
                let f = self.fila_terminal(&t);
                self.lista.append(&f.row);
                let mut st = self.st.borrow_mut();
                st.filas.push(Fila::Terminal(t.id));
                st.filas_terminal.insert(t.id, f);
            }
        }

        let visible = {
            let st = self.st.borrow();
            for g in st.contadores.keys() {
                pintar_contador(&st, *g);
            }
            st.visible.and_then(|v| st.filas_terminal.get(&v).map(|f| f.row.clone()))
        };
        self.lista.select_row(visible.as_ref());
        self.st.borrow_mut().reconstruyendo = false;
    }

    fn fila_grupo(self: &Rc<Self>, grupo: Grupo, nombre: &str, dir: Option<PathBuf>, colapsado: bool) -> gtk::ListBoxRow {
        let flecha = gtk::Label::new(Some(if colapsado { "▸" } else { "▾" }));
        flecha.add_css_class("dim-label");
        let titulo = gtk::Label::new(Some(nombre));
        titulo.set_xalign(0.0);
        titulo.set_hexpand(true);
        titulo.set_ellipsize(gtk::pango::EllipsizeMode::End);
        titulo.add_css_class("grupo");
        let contador = gtk::Label::new(None);
        contador.add_css_class("contador");
        contador.set_visible(false);

        let mas = gtk::Button::from_icon_name("list-add-symbolic");
        mas.add_css_class("flat");
        mas.set_tooltip_text(Some(&format!("Nueva Terminal en {nombre}")));
        mas.connect_clicked(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            move |_| ui.nueva_terminal(grupo)
        ));

        let caja = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        caja.set_margin_top(4);
        caja.append(&flecha);
        caja.append(&titulo);
        caja.append(&contador);
        caja.append(&mas);
        if let Some(p) = grupo {
            let borrar = gtk::Button::from_icon_name("user-trash-symbolic");
            borrar.add_css_class("flat");
            borrar.set_tooltip_text(Some("Borrar Proyecto"));
            borrar.connect_clicked(glib::clone!(
                #[weak(rename_to = ui)]
                self,
                move |_| ui.borrar_proyecto(p)
            ));
            caja.append(&borrar);
        }

        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&caja));
        row.set_selectable(false);
        if let Some(d) = dir {
            row.set_tooltip_text(Some(&d.display().to_string()));
        }
        self.st.borrow_mut().contadores.insert(grupo, contador);
        row
    }

    fn fila_terminal(self: &Rc<Self>, t: &Terminal) -> FilaTerminal {
        let indicador = gtk::Label::new(None);
        indicador.add_css_class("indicador");
        let linea1 = gtk::Label::new(None);
        linea1.set_xalign(0.0);
        linea1.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let linea2 = gtk::Label::new(None);
        linea2.set_xalign(0.0);
        linea2.set_ellipsize(gtk::pango::EllipsizeMode::End);
        linea2.add_css_class("dim-label");
        linea2.add_css_class("caption");

        let textos = gtk::Box::new(gtk::Orientation::Vertical, 0);
        textos.set_hexpand(true);
        textos.append(&linea1);
        textos.append(&linea2);
        let caja = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        caja.set_margin_start(14);
        caja.append(&indicador);
        caja.append(&textos);

        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&caja));
        row.set_tooltip_text(Some(&t.cwd.display().to_string()));

        let id = t.id;
        let clic = gtk::GestureClick::new();
        clic.set_button(3);
        clic.connect_pressed(glib::clone!(
            #[weak(rename_to = ui)]
            self,
            #[weak]
            row,
            move |_, _, x, y| ui.menu_terminal(&row, id, x, y)
        ));
        row.add_controller(clic);

        let f = FilaTerminal {
            row,
            indicador,
            linea1,
            linea2,
        };
        pintar_fila(&f, t);
        f
    }

    fn menu_terminal(self: &Rc<Self>, row: &gtk::ListBoxRow, id: TerminalId, x: f64, y: f64) {
        let menu = gtk::Popover::new();
        menu.set_has_arrow(false);
        menu.set_parent(row);
        menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        let caja = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let opciones: [(&str, AccionTerminal); 4] = [
            ("Renombrar…", |ui, id| ui.renombrar(id)),
            ("Subir", |ui, id| ui.mover(id, -1)),
            ("Bajar", |ui, id| ui.mover(id, 1)),
            ("Cerrar", |ui, id| ui.cerrar_terminal(id)),
        ];
        for (texto, f) in opciones {
            let b = gtk::Button::with_label(texto);
            b.add_css_class("flat");
            b.child().and_downcast::<gtk::Label>().inspect(|l| l.set_xalign(0.0));
            b.connect_clicked(glib::clone!(
                #[weak(rename_to = ui)]
                self,
                #[weak]
                menu,
                move |_| {
                    menu.popdown();
                    f(&ui, id);
                }
            ));
            caja.append(&b);
        }
        menu.set_child(Some(&caja));
        menu.connect_closed(|m| {
            // Desenganchar después de que termine la animación de cierre.
            let m = m.clone();
            glib::idle_add_local_once(move || m.unparent());
        });
        menu.popup();
    }

    // ---- Mostrar y Mirar ----

    fn mostrar(self: &Rc<Self>, id: TerminalId) {
        let (grupo, t) = {
            let st = self.st.borrow();
            let Some(t) = st.inst.terminales.iter().find(|t| t.id == id) else {
                return;
            };
            (t.proyecto, t.clone())
        };
        // Si estaba en un Proyecto colapsado, desplegarlo.
        if self.st.borrow_mut().colapsados.remove(&grupo) {
            self.reconstruir_lista();
        }

        let existente = self.st.borrow().vistas.get(&id).cloned();
        let vista = match existente {
            Some(v) => v,
            None => {
                let (cols, rows) = self.tam_estimado();
                let v = Vista::nueva(
                    id,
                    &self.conexion,
                    glib::clone!(
                        #[weak(rename_to = ui)]
                        self,
                        move || ui.cerrar_terminal(id)
                    ),
                );
                v.actualizar(&t);
                self.pila.add_named(&v.raiz, Some(&format!("t{id}")));
                v.tam_informado.set((cols, rows));
                self.conexion.enviar(Pedido::Adjuntar {
                    id,
                    cols: cols as u16,
                    rows: rows as u16,
                });
                self.st.borrow_mut().vistas.insert(id, v.clone());
                v
            }
        };

        self.st.borrow_mut().visible = Some(id);
        self.pila.set_visible_child(&vista.raiz);
        vista.term.grab_focus();
        // Un clic en el sidebar le devuelve el foco a la fila al terminar de
        // procesarse; recuperarlo después para que el teclado vaya a la Terminal.
        let term = vista.term.clone();
        glib::idle_add_local_once(move || {
            term.grab_focus();
        });

        let fila = self.st.borrow().filas_terminal.get(&id).map(|f| f.row.clone());
        if let Some(fila) = fila {
            self.st.borrow_mut().reconstruyendo = true;
            self.lista.select_row(Some(&fila));
            self.st.borrow_mut().reconstruyendo = false;
        }
        self.actualizar_mirando();
    }

    fn actualizar_mirando(&self) {
        let m = if self.ventana.is_active() {
            self.st.borrow().visible
        } else {
            None
        };
        let mut st = self.st.borrow_mut();
        if st.mirando != Some(m) {
            st.mirando = Some(m);
            drop(st);
            self.conexion.enviar(Pedido::Mirando { id: m });
        }
    }

    fn vista_visible(&self) -> Option<Rc<Vista>> {
        let st = self.st.borrow();
        st.vistas.get(&st.visible?).cloned()
    }

    fn informar_tamano(&self) {
        let (Some(id), Some(v)) = (self.st.borrow().visible, self.vista_visible()) else {
            return;
        };
        if let Some(tam) = v.tam()
            && tam != v.tam_informado.get()
        {
            v.tam_informado.set(tam);
            self.conexion.enviar(Pedido::Redimensionar {
                id,
                cols: tam.0 as u16,
                rows: tam.1 as u16,
            });
        }
    }

    /// Tamaño para una Terminal nueva: el de la visible (todas comparten el área), o uno razonable.
    fn tam_estimado(&self) -> (i64, i64) {
        let otra = self.st.borrow().vistas.values().find_map(|v| v.tam());
        otra.unwrap_or((80, 24))
    }

    /// Terminales en orden del sidebar.
    fn orden(&self) -> Vec<Terminal> {
        let st = self.st.borrow();
        let grupos = st.inst.proyectos.iter().map(|p| Some(p.id)).chain([None]);
        grupos
            .flat_map(|g| st.inst.terminales.iter().filter(move |t| t.proyecto == g).cloned())
            .collect()
    }

    /// A1: la próxima que Pide atención; si no hay, la próxima Terminada sin mirar.
    fn siguiente_atencion(self: &Rc<Self>) {
        let orden = self.orden();
        let visible = self.st.borrow().visible;
        let inicio = visible
            .and_then(|v| orden.iter().position(|t| t.id == v))
            .map(|i| i + 1)
            .unwrap_or(0);
        let rotado = orden.iter().cycle().skip(inicio).take(orden.len());
        let candidatos: Vec<&Terminal> = rotado.collect();
        let elegida = candidatos
            .iter()
            .find(|t| t.estado == Estado::PideAtencion)
            .or_else(|| candidatos.iter().find(|t| t.terminada_sin_mirar));
        if let Some(t) = elegida {
            self.mostrar(t.id);
        }
    }

    fn saltar(self: &Rc<Self>, delta: isize) {
        let orden = self.orden();
        if orden.is_empty() {
            return;
        }
        let actual = self.st.borrow().visible.and_then(|v| orden.iter().position(|t| t.id == v));
        let n = orden.len() as isize;
        let i = match actual {
            Some(i) => (i as isize + delta).rem_euclid(n),
            None => 0,
        };
        self.mostrar(orden[i as usize].id);
    }

    fn grupo_actual(&self) -> Grupo {
        let st = self.st.borrow();
        st.visible
            .and_then(|v| st.inst.terminales.iter().find(|t| t.id == v))
            .and_then(|t| t.proyecto)
    }

    // ---- Acciones ----

    fn nueva_terminal(self: &Rc<Self>, proyecto: Grupo) {
        let (cols, rows) = self.tam_estimado();
        self.conexion.pedir(
            Pedido::CrearTerminal {
                proyecto,
                cols: cols as u16,
                rows: rows as u16,
            },
            glib::clone!(
                #[weak(rename_to = ui)]
                self,
                move |r| match r {
                    // La Estructura nueva llega antes que esta respuesta: la fila ya existe.
                    Ok(Valor::TerminalCreada { id }) => ui.mostrar(id),
                    Ok(_) => {}
                    Err(e) => ui.error("No se pudo crear la Terminal", &e),
                }
            ),
        );
    }

    fn nuevo_proyecto(self: &Rc<Self>) {
        let d = gtk::FileDialog::builder()
            .title("Elegí el directorio del Proyecto")
            .modal(true)
            .build();
        d.select_folder(
            Some(&self.ventana),
            None::<&gio::Cancellable>,
            glib::clone!(
                #[weak(rename_to = ui)]
                self,
                move |r| {
                    let Some(dir) = r.ok().and_then(|f| f.path()) else {
                        return;
                    };
                    let sugerido = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    dialogos::pedir_texto(
                        &ui.ventana,
                        "Nuevo Proyecto",
                        &format!("Nombre del Proyecto en {}", dir.display()),
                        &sugerido,
                        "Crear",
                        glib::clone!(
                            #[weak]
                            ui,
                            move |nombre| {
                                ui.conexion.pedir(
                                    Pedido::CrearProyecto {
                                        nombre,
                                        dir: dir.clone(),
                                    },
                                    glib::clone!(
                                        #[weak]
                                        ui,
                                        move |r| {
                                            if let Err(e) = r {
                                                ui.error("No se pudo crear el Proyecto", &e);
                                            }
                                        }
                                    ),
                                );
                            }
                        ),
                    );
                }
            ),
        );
    }

    fn borrar_proyecto(self: &Rc<Self>, id: ProyectoId) {
        let (nombre, n) = {
            let st = self.st.borrow();
            let Some(p) = st.inst.proyectos.iter().find(|p| p.id == id) else {
                return;
            };
            let n = st.inst.terminales.iter().filter(|t| t.proyecto == Some(id)).count();
            (p.nombre.clone(), n)
        };
        let detalle = match n {
            0 => "No tiene Terminales.".to_string(),
            1 => "Se va a cerrar su Terminal y terminar su proceso.".to_string(),
            n => format!("Se van a cerrar sus {n} Terminales y terminar sus procesos."),
        };
        let c = self.conexion.clone();
        dialogos::confirmar(
            &self.ventana,
            &format!("¿Borrar el Proyecto «{nombre}»?"),
            &detalle,
            "Borrar",
            move || c.enviar(Pedido::BorrarProyecto { id }),
        );
    }

    /// T6/T7: cerrar una viva pide confirmación; una Terminada no.
    fn cerrar_terminal(self: &Rc<Self>, id: TerminalId) {
        let Some(t) = self.st.borrow().inst.terminales.iter().find(|t| t.id == id).cloned() else {
            return;
        };
        if matches!(t.estado, Estado::Terminada { .. }) {
            self.conexion.enviar(Pedido::CerrarTerminal { id });
            return;
        }
        let c = self.conexion.clone();
        dialogos::confirmar(
            &self.ventana,
            &format!("¿Cerrar «{}»?", rotulo(&t)),
            "Su proceso sigue corriendo y se va a terminar.",
            "Cerrar",
            move || c.enviar(Pedido::CerrarTerminal { id }),
        );
    }

    fn renombrar(self: &Rc<Self>, id: TerminalId) {
        let actual = self
            .st
            .borrow()
            .inst
            .terminales
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.nombre.clone())
            .unwrap_or_default();
        let c = self.conexion.clone();
        dialogos::pedir_texto(
            &self.ventana,
            "Renombrar Terminal",
            "Nombre (vacío para mostrar solo el Título)",
            &actual,
            "Renombrar",
            move |nombre| {
                c.enviar(Pedido::RenombrarTerminal {
                    id,
                    nombre: Some(nombre).filter(|n| !n.is_empty()),
                })
            },
        );
    }

    fn mover(&self, id: TerminalId, delta: isize) {
        let posicion = {
            let st = self.st.borrow();
            let Some(t) = st.inst.terminales.iter().find(|t| t.id == id) else {
                return;
            };
            let grupo: Vec<TerminalId> = st
                .inst
                .terminales
                .iter()
                .filter(|o| o.proyecto == t.proyecto)
                .map(|o| o.id)
                .collect();
            let i = grupo.iter().position(|&o| o == id).unwrap_or(0) as isize;
            let nueva = i + delta;
            if nueva < 0 || nueva >= grupo.len() as isize {
                return;
            }
            nueva as usize
        };
        self.conexion.enviar(Pedido::MoverTerminal { id, posicion });
    }

    fn salir_y_cerrar_todo(self: &Rc<Self>) {
        let vivas = self
            .st
            .borrow()
            .inst
            .terminales
            .iter()
            .filter(|t| !matches!(t.estado, Estado::Terminada { .. }))
            .count();
        let c = self.conexion.clone();
        dialogos::confirmar(
            &self.ventana,
            "¿Salir y cerrar todas las Terminales?",
            &format!(
                "Se van a terminar {vivas} procesos. Los Proyectos y Terminales se conservan y se reabren con un shell nuevo la próxima vez."
            ),
            "Salir y cerrar todo",
            move || c.enviar(Pedido::Apagar),
        );
    }

    fn error(&self, mensaje: &str, detalle: &str) {
        dialogos::avisar(Some(self.ventana.upcast_ref()), mensaje, detalle, || {});
    }
}

fn rotulo(t: &Terminal) -> String {
    t.nombre
        .clone()
        .or_else(|| t.titulo.clone())
        .unwrap_or_else(|| format!("Terminal {}", t.id))
}

/// I3: indicador de estado, Nombre arriba y Título abajo (o solo el Título).
fn pintar_fila(f: &FilaTerminal, t: &Terminal) {
    let (simbolo, clase, texto_estado) = match t.estado {
        Estado::Activa => ("●", "activa", "Activa"),
        Estado::Quieta => ("○", "quieta", "Quieta"),
        Estado::PideAtencion => ("!", "atencion", "Pide atención"),
        Estado::Terminada { codigo: 0 } => ("✓", "terminada-ok", "Terminada"),
        Estado::Terminada { .. } => ("✕", "terminada", "Terminada"),
    };
    f.indicador.set_text(simbolo);
    f.indicador.set_css_classes(&["indicador", clase]);
    f.indicador.set_tooltip_text(Some(texto_estado));

    let (l1, mut l2) = match (&t.nombre, &t.titulo) {
        (Some(n), Some(ti)) => (n.clone(), ti.clone()),
        (Some(n), None) => (n.clone(), String::new()),
        (None, Some(ti)) => (ti.clone(), String::new()),
        (None, None) => (format!("Terminal {}", t.id), String::new()),
    };
    if let Estado::Terminada { codigo } = t.estado {
        let fin = format!("código {codigo}");
        l2 = if l2.is_empty() { fin } else { format!("{fin} · {l2}") };
    }
    f.linea1.set_text(&l1);
    f.linea2.set_text(&l2);
    f.linea2.set_visible(!l2.is_empty());

    let destacada = t.estado == Estado::PideAtencion || t.terminada_sin_mirar;
    if destacada {
        f.linea1.add_css_class("destacada");
    } else {
        f.linea1.remove_css_class("destacada");
    }
    if t.estado == Estado::PideAtencion {
        f.row.add_css_class("fila-atencion");
    } else {
        f.row.remove_css_class("fila-atencion");
    }
}

/// I4: en un grupo colapsado, cuántas Piden atención o están Terminadas sin mirar.
fn pintar_contador(st: &Estado_, grupo: Grupo) {
    let Some(c) = st.contadores.get(&grupo) else { return };
    let n = st
        .inst
        .terminales
        .iter()
        .filter(|t| t.proyecto == grupo && (t.estado == Estado::PideAtencion || t.terminada_sin_mirar))
        .count();
    c.set_text(&n.to_string());
    c.set_visible(n > 0 && st.colapsados.contains(&grupo));
}

fn cargar_css() {
    let css = gtk::CssProvider::new();
    css.load_from_string(include_str!("estilo.css"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
