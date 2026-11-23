//! Ubicaciones en disco. `SWARM_RUNTIME_DIR` y `SWARM_STATE_DIR` las
//! sobreescriben (lo usan los tests).

use std::env;
use std::path::PathBuf;

/// Directorio del socket: `$XDG_RUNTIME_DIR/swarm`.
pub fn runtime_dir() -> PathBuf {
    if let Some(d) = env::var_os("SWARM_RUNTIME_DIR") {
        return d.into();
    }
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join(format!("swarm-{}", unsafe { libc::getuid() })));
    base.join("swarm")
}

pub fn socket() -> PathBuf {
    runtime_dir().join("swarmd.sock")
}

/// Directorio de estado persistente: `$XDG_STATE_HOME/swarm` (por defecto `~/.local/state/swarm`).
pub fn state_dir() -> PathBuf {
    if let Some(d) = env::var_os("SWARM_STATE_DIR") {
        return d.into();
    }
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"));
    base.join("swarm")
}

pub fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into())
}
