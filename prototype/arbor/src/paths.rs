//! Where Arbor keeps its state, and how paths are shown.

use std::env;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The folder that holds every world: `$ARBOR_HOME`, else `$XDG_DATA_HOME/arbor`, else `~/.local/share/arbor`.
pub fn state_dir() -> Result<PathBuf> {
    if let Some(dir) = env::var_os("ARBOR_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join("arbor"));
    }
    home()
        .map(|h| h.join(".local/share/arbor"))
        .ok_or_else(|| Error::new("cannot find the home folder: set HOME or ARBOR_HOME"))
}

pub fn home() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The home folder, resolved, when it is a folder other than `/`: the one a cell hides.
pub fn resolved_home() -> Option<PathBuf> {
    home()
        .and_then(|h| std::fs::canonicalize(h).ok())
        .filter(|h| h.is_dir() && h != Path::new("/"))
}

/// Shows a path under the home folder as `~/…`: shorter to read, and shorter for a model to read.
pub fn tilde(path: &Path) -> String {
    if let Some(home) = home()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}

/// True when `inner` is `outer` or lies inside it. Both paths must already be absolute and resolved.
pub fn contains(outer: &Path, inner: &Path) -> bool {
    inner.starts_with(outer)
}
