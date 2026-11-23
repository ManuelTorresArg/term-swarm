//! Persistencia de la Estructura (V6): Proyectos y Terminales sin sus procesos.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use swarm_ipc::{Proyecto, ProyectoId, TerminalId};

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guardada {
    pub proximo_id: u64,
    pub proyectos: Vec<Proyecto>,
    /// En orden de presentación.
    pub terminales: Vec<TerminalGuardada>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalGuardada {
    pub id: TerminalId,
    pub proyecto: Option<ProyectoId>,
    pub nombre: Option<String>,
    pub cwd: PathBuf,
}

/// Carga la Estructura. Si el archivo no existe arranca vacía; si está
/// corrupto lo aparta (no lo pisa) y arranca vacía.
pub fn cargar(ruta: &Path) -> Guardada {
    let datos = match fs::read(ruta) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Guardada::default(),
        Err(e) => {
            log::error!(
                "no se pudo leer {}: {e}; arranco sin Estructura",
                ruta.display()
            );
            return Guardada::default();
        }
    };
    match serde_json::from_slice(&datos) {
        Ok(g) => g,
        Err(e) => {
            let apartado = ruta.with_extension("json.corrupto");
            log::error!(
                "{} está corrupto ({e}); lo muevo a {}",
                ruta.display(),
                apartado.display()
            );
            let _ = fs::rename(ruta, apartado);
            Guardada::default()
        }
    }
}

/// Escritura atómica: archivo temporal + rename.
pub fn guardar(ruta: &Path, g: &Guardada) -> Result<()> {
    let dir = ruta.parent().context("ruta sin directorio")?;
    fs::create_dir_all(dir)?;
    let tmp = ruta.with_extension("json.tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(&serde_json::to_vec_pretty(g)?)?;
    f.sync_all()?;
    fs::rename(&tmp, ruta)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ida_y_vuelta_y_corrupto() {
        let dir = std::env::temp_dir().join(format!("swarm-estructura-{}", std::process::id()));
        let ruta = dir.join("estructura.json");
        let g = Guardada {
            proximo_id: 3,
            proyectos: vec![Proyecto {
                id: 1,
                nombre: "web".into(),
                dir: "/tmp".into(),
            }],
            terminales: vec![TerminalGuardada {
                id: 2,
                proyecto: Some(1),
                nombre: None,
                cwd: "/tmp".into(),
            }],
        };
        guardar(&ruta, &g).unwrap();
        assert_eq!(cargar(&ruta), g);

        fs::write(&ruta, b"{no es json").unwrap();
        assert_eq!(cargar(&ruta), Guardada::default());
        assert!(ruta.with_extension("json.corrupto").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
