//! A world is a copy-on-write copy of one folder (its base). Commands change the world, never the base, until a
//! person keeps the result. On disk a world is a folder in Arbor's state: `world.json`, a `lock` file, the
//! overlay's `up` (every change) and `work` folders, and `undo` once it has been kept.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Context, Error, Result};
use crate::{paths, sys};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Changes live only in the world.
    Open,
    /// The changes were applied to the base; `undo` can still take them back.
    Kept,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub name: String,
    /// The real folder, absolute and resolved.
    pub base: PathBuf,
    /// The last moment the base matched what the world builds on, in nanoseconds since 1970 (see `stamp_now`):
    /// set when the world is made and again when undo puts the base back. Changes to the base after it are
    /// conflicts for keep.
    pub since_ns: u64,
    pub state: State,
    /// When the world was last kept, on the same scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_ns: Option<u64>,
    /// Where the world lives; not stored, set when loading.
    #[serde(skip)]
    pub dir: PathBuf,
}

/// Held while a command runs in a world or while it is kept, undone or discarded; released when dropped.
pub struct Lock {
    _file: File,
}

pub fn worlds_dir() -> Result<PathBuf> {
    Ok(paths::state_dir()?.join("worlds"))
}

fn clock_ns(clock: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(clock, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

pub fn now_ns() -> u64 {
    clock_ns(libc::CLOCK_REALTIME)
}

/// A moment that file timestamps order exactly: every change before it is stamped at or before it, every change
/// after it is stamped later. Files are stamped from a coarse clock (up to a few ms behind) or, since Linux 6.13 on
/// some filesystems, the fine one; waiting until the coarse clock passes the fine reading covers both.
pub fn stamp_now() -> u64 {
    let t = now_ns();
    while clock_ns(libc::CLOCK_REALTIME_COARSE) <= t {
        std::thread::sleep(std::time::Duration::from_micros(250));
    }
    t
}

pub fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c))
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(Error::usage(format!(
            "'{name}' is not a usable world name: use lowercase letters, digits, '-', '_' or '.', \
             starting with a letter or digit, at most 64 characters"
        )))
    }
}

/// Turns a folder name into a world name: `My Shop` becomes `my-shop`.
fn name_from(base: &Path) -> String {
    let raw = base
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    name = name
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string();
    name.truncate(56);
    if name.is_empty() {
        "world".to_string()
    } else {
        name
    }
}

impl World {
    pub fn up(&self) -> PathBuf {
        self.dir.join("up")
    }

    pub fn work(&self) -> PathBuf {
        self.dir.join("work")
    }

    pub fn undo_dir(&self) -> PathBuf {
        self.dir.join("undo")
    }

    /// Makes a world over `from`. Nothing is copied: the world starts empty and fills only with changes.
    pub fn create(name: Option<&str>, from: &Path) -> Result<World> {
        let base = fs::canonicalize(from).ctx(|| format!("cannot use {}", from.display()))?;
        if !base.is_dir() {
            return Err(Error::usage(format!(
                "{} is not a folder",
                paths::tilde(&base)
            )));
        }
        if base == Path::new("/") {
            return Err(Error::usage(
                "a world of the whole computer is not supported yet; pick a folder",
            ));
        }
        let worlds = worlds_dir()?;
        fs::create_dir_all(&worlds).ctx(|| format!("cannot create {}", worlds.display()))?;
        let worlds = fs::canonicalize(&worlds)?;
        // overlayfs: no layer may contain another, so Arbor's own folder and the base must stay apart.
        if paths::contains(&base, &worlds) {
            return Err(Error::usage(format!(
                "{} holds Arbor's own folder ({}); make the world from a folder inside it instead",
                paths::tilde(&base),
                paths::tilde(&worlds)
            )));
        }
        if paths::contains(&worlds, &base) {
            return Err(Error::usage(
                "a world cannot be made from Arbor's own folder",
            ));
        }

        let (name, dir) = match name {
            Some(n) => {
                check_name(n)?;
                let dir = worlds.join(n);
                match fs::create_dir(&dir) {
                    Ok(()) => (n.to_string(), dir),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                        return Err(Error::refused(format!(
                            "a world named '{n}' already exists"
                        )));
                    }
                    Err(e) => return Err(e).ctx(|| format!("cannot create {}", dir.display())),
                }
            }
            None => Self::claim_free_name(&worlds, &name_from(&base))?,
        };

        let world = World {
            name,
            base,
            since_ns: stamp_now(),
            state: State::Open,
            kept_ns: None,
            dir,
        };
        fs::create_dir(world.up())?;
        fs::create_dir(world.work())?;
        File::create(world.dir.join("lock"))?;
        world.save()?;
        Ok(world)
    }

    fn claim_free_name(worlds: &Path, stem: &str) -> Result<(String, PathBuf)> {
        for i in 1..1000 {
            let name = if i == 1 {
                stem.to_string()
            } else {
                format!("{stem}-{i}")
            };
            let dir = worlds.join(&name);
            match fs::create_dir(&dir) {
                Ok(()) => return Ok((name, dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e).ctx(|| format!("cannot create {}", dir.display())),
            }
        }
        Err(Error::new(format!(
            "no free world name starting with '{stem}'"
        )))
    }

    pub fn load(name: &str) -> Result<World> {
        check_name(name)?;
        let dir = worlds_dir()?.join(name);
        let text = match fs::read_to_string(dir.join("world.json")) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::usage(format!(
                    "there is no world named '{name}' (see `arbor worlds`)"
                )));
            }
            Err(e) => return Err(e).ctx(|| format!("cannot read world '{name}'")),
        };
        let mut world: World = serde_json::from_str(&text)
            .ctx(|| format!("world '{name}' has a damaged world.json"))?;
        world.dir = dir;
        Ok(world)
    }

    /// Every world, sorted by name. Folders without a readable `world.json` are skipped.
    pub fn list() -> Result<Vec<World>> {
        let dir = worlds_dir()?;
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).ctx(|| format!("cannot read {}", dir.display())),
        };
        let mut worlds = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            if let Some(name) = name.to_str()
                && let Ok(world) = World::load(name)
            {
                worlds.push(world);
            }
        }
        worlds.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(worlds)
    }

    /// Writes `world.json` through a temporary file, so a crash leaves the old or the new version, never half.
    pub fn save(&self) -> Result<()> {
        let tmp = self.dir.join("world.json.tmp");
        let mut f = File::create(&tmp)?;
        f.write_all(
            serde_json::to_string_pretty(self)
                .map_err(|e| Error::new(e.to_string()))?
                .as_bytes(),
        )?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        fs::rename(&tmp, self.dir.join("world.json"))?;
        Ok(())
    }

    /// One thing at a time per world: a running command, a keep, an undo or a discard.
    pub fn lock(&self) -> Result<Lock> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join("lock"))?;
        match sys::flock(&file, true, false) {
            Ok(()) => Ok(Lock { _file: file }),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Err(Error::refused(format!(
                "world '{}' is busy: a command is still running in it, or it is being kept",
                self.name
            ))),
            Err(e) => Err(e.into()),
        }
    }

    /// Deletes the world and everything it holds. Its base is not touched.
    pub fn discard(self, _lock: &Lock) -> Result<()> {
        remove_tree(&self.dir).ctx(|| format!("cannot remove world '{}'", self.name))
    }
}

/// Removes a folder tree that may hold overlayfs leftovers: the kernel leaves `work/work` with mode 000, which
/// a plain recursive delete cannot enter.
pub fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => {
            make_writable(path)?;
            fs::remove_dir_all(path)
        }
    }
}

fn make_writable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(meta.permissions().mode() | 0o700),
        )?;
        for entry in fs::read_dir(path)? {
            make_writable(&entry?.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(check_name("demo").is_ok());
        assert!(check_name("try-2.fast_path").is_ok());
        assert!(check_name("").is_err());
        assert!(check_name("-x").is_err());
        assert!(check_name("Has Caps").is_err());
        assert!(check_name("../x").is_err());
        assert_eq!(name_from(Path::new("/x/My Shop")), "my-shop");
        assert_eq!(name_from(Path::new("/x/__")), "world");
    }
}
