//! A world's changes are exactly what its overlay wrote into `up`: changed and new files, a whiteout (a 0:0
//! character device) for each deleted path, and opaque folders that replace a base folder whole. This module
//! reads them (diff), applies them to the base (keep) and takes them back (undo).
//!
//! Keep moves each change into the base with an atomic rename where it can, and moves every original it replaces
//! or deletes into the world's `undo/files`, writing one journal line per step. Undo replays the journal backwards
//! and restores the world's `up` exactly, whiteouts and opaque folders included, so a kept world can be kept again.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Context, Error, Result};
use crate::world::{self, State, World};
use crate::{paths, sys};

const OPAQUE: &str = "user.overlay.opaque";
/// overlayfs features Arbor does not apply. They need privileged mounts, so a world should never hold them.
const UNSUPPORTED: [&str; 2] = ["user.overlay.redirect", "user.overlay.metacopy"];

/// One path the world changes, as `up` records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// New in the world; nothing at this path in the base.
    Add { path: PathBuf, dir: bool },
    /// A file or link in the world replaces a file or link in the base.
    Replace { path: PathBuf },
    /// Deleted in the world.
    Remove { path: PathBuf, dir: bool },
    /// The world's version replaces the base's whole: a folder recreated empty (opaque), or a change of type.
    Swap {
        path: PathBuf,
        dir: bool,
        base_dir: bool,
    },
    /// A pipe, socket or device made in the world; never applied.
    Special { path: PathBuf },
}

impl Entry {
    pub fn path(&self) -> &Path {
        match self {
            Entry::Add { path, .. }
            | Entry::Replace { path }
            | Entry::Remove { path, .. }
            | Entry::Swap { path, .. }
            | Entry::Special { path } => path,
        }
    }
}

/// One line of the change list: `A`dded, `M`odified, `D`eleted, or `?` for a special file that keep skips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub mark: char,
    pub path: String,
}

fn lstat(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn sorted_names(dir: &Path) -> io::Result<Vec<std::ffi::OsString>> {
    let mut names = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

/// Reads the world's changes, in path order.
pub fn entries(world: &World) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    walk(&world.up(), &world.base, Path::new(""), &mut out)
        .ctx(|| format!("cannot read world '{}'", world.name))?;
    Ok(out)
}

fn walk(up: &Path, base: &Path, rel: &Path, out: &mut Vec<Entry>) -> io::Result<()> {
    for name in sorted_names(&up.join(rel))? {
        let path = rel.join(&name);
        let u = up.join(&path);
        let um = fs::symlink_metadata(&u)?;
        if !um.file_type().is_symlink() {
            for attr in sys::list_xattrs(&u)? {
                if UNSUPPORTED.contains(&attr.as_str()) {
                    return Err(io::Error::other(format!(
                        "{} uses an overlayfs feature Arbor cannot apply ({attr})",
                        path.display()
                    )));
                }
            }
        }
        let bm = lstat(&base.join(&path))?;
        if sys::is_whiteout(&um) {
            // A whiteout over nothing (the base lost the path meanwhile) changes nothing.
            if let Some(bm) = bm {
                out.push(Entry::Remove {
                    path,
                    dir: bm.is_dir(),
                });
            }
        } else if um.is_dir() {
            let opaque = sys::get_xattr(&u, OPAQUE)?.is_some_and(|v| v == b"y");
            match bm {
                Some(bm) if bm.is_dir() && !opaque => walk(up, base, &path, out)?,
                Some(bm) => out.push(Entry::Swap {
                    path,
                    dir: true,
                    base_dir: bm.is_dir(),
                }),
                None => out.push(Entry::Add { path, dir: true }),
            }
        } else if um.is_file() || um.file_type().is_symlink() {
            match bm {
                Some(bm) if !bm.is_dir() => out.push(Entry::Replace { path }),
                Some(_) => out.push(Entry::Swap {
                    path,
                    dir: false,
                    base_dir: true,
                }),
                None => out.push(Entry::Add { path, dir: false }),
            }
        } else {
            out.push(Entry::Special { path });
        }
    }
    Ok(())
}

/// Every file (and empty folder) under `dir`, relative to it.
fn files_under(dir: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let names = sorted_names(&dir.join(rel))?;
    if names.is_empty() && !rel.as_os_str().is_empty() {
        out.push(rel.to_path_buf());
    }
    for name in names {
        let path = rel.join(name);
        if fs::symlink_metadata(dir.join(&path))?.is_dir() {
            files_under(dir, &path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn show(path: &Path, dir: bool) -> String {
    let s = path.display().to_string();
    if dir { format!("{s}/") } else { s }
}

/// The change list a person or an agent reads: files, not folders, except a folder deleted or left empty.
pub fn changes(world: &World, entries: &[Entry]) -> Result<Vec<Change>> {
    let (up, base) = (world.up(), &world.base);
    let mut out = Vec::new();
    let mut push = |mark: char, path: String| out.push(Change { mark, path });
    for e in entries {
        match e {
            Entry::Add { path, dir: false } => push('A', show(path, false)),
            Entry::Replace { path } => push('M', show(path, false)),
            Entry::Remove { path, dir } => push('D', show(path, *dir)),
            Entry::Special { path } => push('?', show(path, false)),
            Entry::Add { path, dir: true } => {
                let mut files = Vec::new();
                files_under(&up.join(path), Path::new(""), &mut files)?;
                if files.is_empty() {
                    push('A', show(path, true));
                }
                for f in files {
                    let dir = fs::symlink_metadata(up.join(path).join(&f))?.is_dir();
                    push('A', show(&path.join(f), dir));
                }
            }
            Entry::Swap {
                path,
                dir,
                base_dir,
            } => {
                // Compare the two versions file by file.
                let (mut ours, mut theirs) = (Vec::new(), Vec::new());
                if *dir {
                    files_under(&up.join(path), Path::new(""), &mut ours)?;
                }
                if *base_dir {
                    files_under(&base.join(path), Path::new(""), &mut theirs)?;
                }
                let (ours_set, theirs_set): (BTreeSet<_>, BTreeSet<_>) =
                    (ours.iter().collect(), theirs.iter().collect());
                if !*base_dir {
                    push('D', show(path, false));
                }
                for f in &theirs {
                    if !ours_set.contains(f) {
                        push('D', show(&path.join(f), false));
                    }
                }
                if !*dir {
                    push('A', show(path, false));
                }
                for f in &ours {
                    push(
                        if theirs_set.contains(f) { 'M' } else { 'A' },
                        show(&path.join(f), false),
                    );
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

// ---- keep and undo ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Op {
    Add,
    Replace,
    Remove,
    Swap,
}

/// One journal line: what keep did at `path`, so undo can do the opposite.
#[derive(Debug, Serialize, Deserialize)]
struct Step {
    op: Op,
    path: PathBuf,
    /// For a swapped folder: the world's folder hid a base folder, so undo marks it opaque again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    opaque: bool,
}

/// The most recent change to a path itself: its content (mtime) or its metadata, links and name (ctime).
fn stamp(m: &fs::Metadata) -> u64 {
    let ns = |s: i64, n: i64| (s.max(0) as u64) * 1_000_000_000 + n.max(0) as u64;
    ns(m.mtime(), m.mtime_nsec()).max(ns(m.ctime(), m.ctime_nsec()))
}

/// The first path at or under `path` that changed after `since` (a `world::stamp_now` reading).
fn changed_since(path: &Path, since: u64) -> io::Result<Option<PathBuf>> {
    let Some(m) = lstat(path)? else {
        return Ok(None);
    };
    if stamp(&m) > since {
        return Ok(Some(path.to_path_buf()));
    }
    if m.is_dir() {
        for name in sorted_names(path)? {
            if let Some(p) = changed_since(&path.join(name), since)? {
                return Ok(Some(p));
            }
        }
    }
    Ok(None)
}

/// What keep must refuse: base paths that changed after the world was made, so the world never overwrites
/// work it has not seen. overlayfs leaves a base that changes under an open world undefined, so this is checked
/// rather than assumed.
pub fn conflicts(world: &World, entries: &[Entry]) -> Result<Vec<String>> {
    let base = &world.base;
    let since = world.since_ns;
    let mut out = Vec::new();
    for e in entries {
        let path = e.path();
        let b = base.join(path);
        let why = match e {
            Entry::Special { .. } => None,
            // A new file in a base folder whose entries changed since: the base may have deleted or renamed it.
            Entry::Add { .. } => match b.parent().map(lstat).transpose()?.flatten() {
                Some(pm) if stamp(&pm) > since => {
                    Some("its folder in the base changed since the world was made")
                }
                None => Some("its folder is gone from the base"),
                _ => None,
            },
            Entry::Replace { .. } | Entry::Swap { .. } if lstat(&b)?.is_none() => {
                Some("deleted from the base")
            }
            Entry::Replace { .. } | Entry::Remove { .. } | Entry::Swap { .. } => {
                changed_since(&b, since)?.map(|_| "changed in the base since the world was made")
            }
        };
        if let Some(why) = why {
            out.push(format!("{}: {why}", path.display()));
        }
    }
    Ok(out)
}

fn exdev(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::EXDEV)
}

/// A temporary name next to `path`, on its filesystem.
fn beside(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(
        ".{name}.arbor-{}-{}",
        std::process::id(),
        world::now_ns() % 1_000_000_007
    ))
}

/// Copies a file, link or folder tree, keeping modes and modification times. Pipes and sockets are skipped.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(from)?;
    if m.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(from)?, to)?;
    } else if m.is_dir() {
        fs::create_dir(to)?;
        for name in sorted_names(from)? {
            copy_tree(&from.join(&name), &to.join(&name))?;
        }
        fs::set_permissions(to, fs::Permissions::from_mode(m.mode()))?;
    } else if m.is_file() {
        fs::copy(from, to)?;
        File::options()
            .write(true)
            .open(to)?
            .set_modified(m.modified()?)?;
    }
    Ok(())
}

fn remove_any(path: &Path) -> io::Result<()> {
    match lstat(path)? {
        Some(m) if m.is_dir() => world::remove_tree(path),
        Some(_) => fs::remove_file(path),
        None => Ok(()),
    }
}

fn ensure_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(p) => fs::create_dir_all(p),
        None => Ok(()),
    }
}

/// Moves `from` to `to`, which must not exist: one rename on the same filesystem, else a copy beside `to`
/// renamed into place, then the source removed.
fn move_into(from: &Path, to: &Path) -> io::Result<()> {
    ensure_parent(to)?;
    match sys::rename_noreplace(from, to) {
        Err(e) if exdev(&e) => {
            let tmp = beside(to);
            copy_tree(from, &tmp)?;
            sys::rename_noreplace(&tmp, to)?;
            remove_any(from)
        }
        other => other,
    }
}

/// Puts `new` at `dst`, which exists, in one atomic swap, and moves what was at `dst` to `old_to`.
fn replace_with(new: &Path, dst: &Path, old_to: &Path) -> io::Result<()> {
    match sys::rename_exchange(new, dst) {
        Ok(()) => move_into(new, old_to),
        Err(e) if exdev(&e) => {
            let tmp = beside(dst);
            copy_tree(new, &tmp)?;
            sys::rename_exchange(&tmp, dst)?;
            move_into(&tmp, old_to)?;
            remove_any(new)
        }
        Err(e) => Err(e),
    }
}

/// Removes overlayfs's own attributes from a path moved into the base, so they never leak into real files.
fn strip_overlay_attrs(path: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if m.file_type().is_symlink() {
        return Ok(());
    }
    for attr in sys::list_xattrs(path)? {
        if attr.starts_with("user.overlay.") {
            sys::remove_xattr(path, &attr)?;
        }
    }
    if m.is_dir() {
        for name in sorted_names(path)? {
            strip_overlay_attrs(&path.join(name))?;
        }
    }
    Ok(())
}

/// Removes now-empty folders left in `up` after keep, deepest first. `up` itself stays.
fn prune_empty(dir: &Path, top: bool) -> io::Result<()> {
    for name in sorted_names(dir)? {
        let p = dir.join(name);
        if fs::symlink_metadata(&p)?.is_dir() {
            prune_empty(&p, false)?;
        }
    }
    if !top && fs::read_dir(dir)?.next().is_none() {
        fs::remove_dir(dir)?;
    }
    Ok(())
}

pub struct Kept {
    pub changes: Vec<Change>,
    pub skipped: usize,
}

/// Applies the world to its base. The caller holds the world's lock.
pub fn keep(world: &mut World, force: bool) -> Result<Kept> {
    if world.state != State::Open {
        return Err(Error::refused(format!(
            "world '{}' was already kept",
            world.name
        )));
    }
    if !world.base.is_dir() {
        return Err(Error::new(format!(
            "the base folder {} is gone",
            paths::tilde(&world.base)
        )));
    }
    let entries = entries(world)?;
    let found = conflicts(world, &entries)?;
    if !found.is_empty() && !force {
        return Err(Error::refused(format!(
            "nothing was kept: {} path(s) changed in {} after world '{}' was made:\n  {}\n\
             Look at them, then run `arbor keep {} --force` to apply the world anyway \
             (the base's versions go to the undo folder), or `arbor discard {}`.",
            found.len(),
            paths::tilde(&world.base),
            world.name,
            found.join("\n  "),
            world.name,
            world.name
        )));
    }
    let changes = changes(world, &entries)?;
    let (up, base, files) = (
        world.up(),
        world.base.clone(),
        world.undo_dir().join("files"),
    );
    fs::create_dir_all(&files)?;
    let mut journal = OpenOptions::new()
        .create(true)
        .append(true)
        .open(world.undo_dir().join("journal.jsonl"))?;
    // From the first step on the world counts as kept, so a failure part-way can still be undone.
    world.state = State::Kept;
    world.kept_ns = Some(world::stamp_now());
    world.save()?;

    let mut skipped = 0;
    let mut failure = None;
    for e in &entries {
        match apply_entry(e, &up, &base, &files, &mut journal) {
            Ok(true) => {}
            Ok(false) => skipped += 1,
            Err(err) => {
                failure = Some(format!(
                    "keep stopped at {}: {err}\nWhat was applied so far is in the journal; `arbor undo {}` puts it back.",
                    e.path().display(),
                    world.name
                ));
                break;
            }
        }
    }
    journal.sync_all()?;
    // Stamped after the last step, so undo can tell keep's own renames from later edits.
    world.kept_ns = Some(world::stamp_now());
    world.save()?;
    if let Some(msg) = failure {
        return Err(Error::new(msg));
    }
    prune_empty(&up, true)?;
    Ok(Kept { changes, skipped })
}

/// Applies one entry and writes its journal line. Returns false for a special file, which is skipped.
fn apply_entry(
    e: &Entry,
    up: &Path,
    base: &Path,
    files: &Path,
    journal: &mut File,
) -> io::Result<bool> {
    let path = e.path();
    let (u, b, f) = (up.join(path), base.join(path), files.join(path));
    let step = |op, opaque| Step {
        op,
        path: path.into(),
        opaque,
    };
    let step = match e {
        Entry::Special { .. } => return Ok(false),
        Entry::Add { .. } => {
            move_into(&u, &b)?;
            strip_overlay_attrs(&b)?;
            step(Op::Add, false)
        }
        Entry::Replace { .. } => {
            replace_with(&u, &b, &f)?;
            strip_overlay_attrs(&b)?;
            step(Op::Replace, false)
        }
        Entry::Remove { .. } => {
            move_into(&b, &f)?;
            fs::remove_file(&u)?;
            step(Op::Remove, false)
        }
        Entry::Swap { dir, base_dir, .. } => {
            move_into(&b, &f)?;
            move_into(&u, &b)?;
            strip_overlay_attrs(&b)?;
            step(Op::Swap, *dir && *base_dir)
        }
    };
    writeln!(
        journal,
        "{}",
        serde_json::to_string(&step).map_err(io::Error::other)?
    )?;
    Ok(true)
}

fn read_journal(world: &World) -> Result<Vec<Step>> {
    let path = world.undo_dir().join("journal.jsonl");
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).ctx(|| format!("cannot read {}", path.display())),
    };
    let mut steps = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if !line.trim().is_empty() {
            steps.push(
                serde_json::from_str(&line)
                    .ctx(|| format!("damaged journal line in {}", path.display()))?,
            );
        }
    }
    Ok(steps)
}

/// Takes a kept world's changes back out of the base and puts them back in the world. The caller holds the
/// world's lock. Refuses when the base changed since the keep, unless `force`, which moves those later
/// versions into the world instead of losing them.
pub fn undo(world: &mut World, force: bool) -> Result<usize> {
    if world.state != State::Kept {
        return Err(Error::refused(format!(
            "world '{}' has not been kept; there is nothing to undo",
            world.name
        )));
    }
    let steps = read_journal(world)?;
    let since = world.kept_ns.unwrap_or(0);
    let (up, base, files) = (
        world.up(),
        world.base.clone(),
        world.undo_dir().join("files"),
    );
    let mut problems = Vec::new();
    for s in &steps {
        let b = base.join(&s.path);
        let present = lstat(&b)?.is_some();
        let why = match s.op {
            Op::Remove if present => Some("made again in the base since the keep"),
            Op::Remove => None,
            _ if !present => Some("deleted from the base since the keep"),
            _ if !force && changed_since(&b, since)?.is_some() => {
                Some("changed in the base since the keep")
            }
            _ => None,
        };
        if let Some(why) = why {
            problems.push(format!("{}: {why}", s.path.display()));
        }
    }
    if !problems.is_empty() {
        return Err(Error::refused(format!(
            "nothing was undone: {} path(s) in {} no longer match what keep left:\n  {}{}",
            problems.len(),
            paths::tilde(&base),
            problems.join("\n  "),
            if force {
                ""
            } else {
                "\nTo move later edits into the world instead, run undo with --force."
            }
        )));
    }
    for s in steps.iter().rev() {
        let (u, b, f) = (up.join(&s.path), base.join(&s.path), files.join(&s.path));
        let res = (|| -> io::Result<()> {
            match s.op {
                Op::Add => move_into(&b, &u),
                Op::Replace => {
                    ensure_parent(&u)?;
                    replace_with(&f, &b, &u)
                }
                Op::Remove => {
                    move_into(&f, &b)?;
                    ensure_parent(&u)?;
                    sys::make_whiteout(&u)
                }
                Op::Swap => {
                    move_into(&b, &u)?;
                    if s.opaque {
                        sys::set_xattr(&u, OPAQUE, b"y")?;
                    }
                    move_into(&f, &b)
                }
            }
        })();
        res.map_err(|e| Error::new(format!("undo stopped at {}: {e}", s.path.display())))?;
    }
    world::remove_tree(&world.undo_dir())?;
    world.state = State::Open;
    world.kept_ns = None;
    // The base is back to what the world builds on; undo's own renames must not read as conflicts later.
    world.since_ns = world::stamp_now();
    world.save()?;
    Ok(steps.len())
}

/// Prints a unified diff of the world's text changes, through the system's `diff`.
pub fn patch(world: &World, out: &mut impl Write) -> Result<()> {
    for c in changes(world, &entries(world)?)? {
        if c.path.ends_with('/') || c.mark == '?' {
            continue;
        }
        let (ours, theirs) = (world.up().join(&c.path), world.base.join(&c.path));
        let (a, b) = match c.mark {
            'A' => (PathBuf::from("/dev/null"), ours),
            'D' => (theirs, PathBuf::from("/dev/null")),
            _ => (theirs, ours),
        };
        let la = if c.mark == 'A' {
            "/dev/null".to_string()
        } else {
            format!("a/{}", c.path)
        };
        let lb = if c.mark == 'D' {
            "/dev/null".to_string()
        } else {
            format!("b/{}", c.path)
        };
        let res = std::process::Command::new("diff")
            .args(["-u", "--label", &la, "--label", &lb])
            .arg(&a)
            .arg(&b)
            .output()
            .ctx(|| "cannot run diff".to_string())?;
        out.write_all(&res.stdout)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! These build a world's `up` folder by hand (whiteouts and opaque folders included), so they test diff, keep
    //! and undo without bubblewrap.
    use super::*;
    use std::os::unix::fs::{FileTypeExt, symlink};

    /// A temporary folder removed when dropped, even with overlay-style permissions inside.
    pub struct Temp(pub PathBuf);
    impl Temp {
        pub fn new(tag: &str) -> Temp {
            let dir = std::env::temp_dir().join(format!(
                "arbor-test-{tag}-{}-{}",
                std::process::id(),
                world::now_ns()
            ));
            fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = world::remove_tree(&self.0);
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A snapshot of a tree: every path with its kind and content, for exact before/after comparisons.
    fn snapshot(root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        fn go(root: &Path, rel: &Path, out: &mut Vec<String>) {
            for name in sorted_names(&root.join(rel)).unwrap() {
                let p = rel.join(name);
                let full = root.join(&p);
                let m = fs::symlink_metadata(&full).unwrap();
                let what = if m.file_type().is_symlink() {
                    format!("-> {}", fs::read_link(&full).unwrap().display())
                } else if m.is_dir() {
                    let opaque = sys::get_xattr(&full, OPAQUE).unwrap().is_some();
                    format!("dir{}", if opaque { " opaque" } else { "" })
                } else if m.file_type().is_char_device() {
                    format!("whiteout={}", sys::is_whiteout(&m))
                } else {
                    format!(
                        "file {:?} {:o}",
                        fs::read_to_string(&full).unwrap(),
                        m.mode() & 0o777
                    )
                };
                out.push(format!("{} {what}", p.display()));
                if m.is_dir() {
                    go(root, &p, out);
                }
            }
        }
        go(root, Path::new(""), &mut out);
        out
    }

    /// A base and a world over it, with `up` holding one change of every kind.
    fn fixture(t: &Temp) -> World {
        let base = t.0.join("base");
        write(&base.join("src/a"), "one");
        write(&base.join("src/gone"), "bye");
        write(&base.join("old/x"), "o");
        write(&base.join("keepdir/k1"), "k1");
        write(&base.join("dir2file/f"), "f");
        symlink("src/a", base.join("link")).unwrap();
        let dir = t.0.join("state/worlds/demo");
        fs::create_dir_all(dir.join("up")).unwrap();
        fs::create_dir_all(dir.join("work")).unwrap();
        let world = World {
            name: "demo".into(),
            base,
            since_ns: world::stamp_now(),
            state: State::Open,
            kept_ns: None,
            dir,
        };
        let up = world.up();
        write(&up.join("src/a"), "two");
        write(&up.join("src/b"), "new");
        sys::make_whiteout(&up.join("src/gone")).unwrap();
        sys::make_whiteout(&up.join("old")).unwrap();
        write(&up.join("keepdir/k2"), "k2");
        sys::set_xattr(&up.join("keepdir"), OPAQUE, b"y").unwrap();
        write(&up.join("deep/er/z"), "z");
        write(&up.join("dir2file"), "now a file");
        symlink("src/b", up.join("link")).unwrap();
        world
    }

    fn marks(world: &World) -> Vec<String> {
        changes(world, &entries(world).unwrap())
            .unwrap()
            .iter()
            .map(|c| format!("{} {}", c.mark, c.path))
            .collect()
    }

    const EXPECTED: [&str; 10] = [
        "A deep/er/z",
        "D dir2file/f",
        "A dir2file",
        "D keepdir/k1",
        "A keepdir/k2",
        "M link",
        "D old/",
        "M src/a",
        "A src/b",
        "D src/gone",
    ];

    #[test]
    fn diff_reads_every_kind_of_change() {
        let t = Temp::new("diff");
        let world = fixture(&t);
        let mut got = marks(&world);
        got.sort();
        let mut want: Vec<String> = EXPECTED.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn keep_then_undo_restores_base_and_world_exactly() {
        let t = Temp::new("keep");
        let mut world = fixture(&t);
        let (base_before, up_before) = (snapshot(&world.base), snapshot(&world.up()));

        let kept = keep(&mut world, false).unwrap();
        assert_eq!(kept.skipped, 0);
        assert_eq!(world.state, State::Kept);
        let base = &world.base;
        assert_eq!(fs::read_to_string(base.join("src/a")).unwrap(), "two");
        assert_eq!(fs::read_to_string(base.join("src/b")).unwrap(), "new");
        assert!(!base.join("src/gone").exists() && !base.join("old").exists());
        assert!(!base.join("keepdir/k1").exists());
        assert_eq!(fs::read_to_string(base.join("keepdir/k2")).unwrap(), "k2");
        assert_eq!(
            sys::get_xattr(&base.join("keepdir"), OPAQUE).unwrap(),
            None,
            "overlay attributes leaked"
        );
        assert_eq!(
            fs::read_to_string(base.join("dir2file")).unwrap(),
            "now a file"
        );
        assert_eq!(
            fs::read_link(base.join("link")).unwrap(),
            Path::new("src/b")
        );
        assert_eq!(
            fs::read_to_string(world.undo_dir().join("files/src/a")).unwrap(),
            "one"
        );
        assert!(
            sorted_names(&world.up()).unwrap().is_empty(),
            "keep left changes in the world"
        );
        let base_kept = snapshot(&world.base);

        undo(&mut world, false).unwrap();
        assert_eq!(world.state, State::Open);
        assert_eq!(
            snapshot(&world.base),
            base_before,
            "undo did not restore the base"
        );
        assert_eq!(
            snapshot(&world.up()),
            up_before,
            "undo did not restore the world"
        );

        // Undo leaves the world ready to be kept again, with the same result.
        keep(&mut world, false).unwrap();
        assert_eq!(snapshot(&world.base), base_kept);
    }

    #[test]
    fn keep_refuses_a_base_changed_since_and_force_saves_the_base_version() {
        let t = Temp::new("conflict");
        let mut world = fixture(&t);
        write(&world.base.join("src/a"), "edited outside");
        let err = keep(&mut world, false).err().expect("keep should refuse");
        assert_eq!(err.code, crate::error::REFUSED);
        assert!(
            err.msg.contains("src/a: changed in the base"),
            "{}",
            err.msg
        );
        assert_eq!(
            world.state,
            State::Open,
            "a refused keep must change nothing"
        );
        assert_eq!(
            fs::read_to_string(world.base.join("src/a")).unwrap(),
            "edited outside"
        );

        keep(&mut world, true).unwrap();
        assert_eq!(fs::read_to_string(world.base.join("src/a")).unwrap(), "two");
        undo(&mut world, false).unwrap();
        assert_eq!(
            fs::read_to_string(world.base.join("src/a")).unwrap(),
            "edited outside"
        );
    }

    #[test]
    fn undo_refuses_edits_made_after_keep_unless_forced() {
        let t = Temp::new("undo");
        let mut world = fixture(&t);
        keep(&mut world, false).unwrap();
        write(&world.base.join("src/b"), "edited after keep");
        let err = undo(&mut world, false).expect_err("undo should refuse");
        assert!(
            err.msg
                .contains("src/b: changed in the base since the keep"),
            "{}",
            err.msg
        );
        undo(&mut world, true).unwrap();
        // The later edit is not lost: it moved into the world.
        assert_eq!(
            fs::read_to_string(world.up().join("src/b")).unwrap(),
            "edited after keep"
        );
        assert!(!world.base.join("src/b").exists());
    }

    #[test]
    fn keep_across_filesystems_copies_instead_of_renaming() {
        // /tmp is often tmpfs while the build folder is not; skip where they share a filesystem.
        let other = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/test-tmp");
        fs::create_dir_all(&other).unwrap();
        let t = Temp::new("xdev");
        let far = Temp(other.join(format!("xdev-{}-{}", std::process::id(), world::now_ns())));
        fs::create_dir_all(&far.0).unwrap();
        if fs::metadata(&t.0).unwrap().dev() == fs::metadata(&far.0).unwrap().dev() {
            eprintln!(
                "skipped: {} and {} share a filesystem",
                t.0.display(),
                far.0.display()
            );
            return;
        }
        let mut world = fixture(&t);
        // Move the world's state to the other filesystem; the base stays on this one.
        let moved = far.0.join("demo");
        copy_tree(&world.dir, &moved).unwrap();
        world.dir = moved;
        let base_before = snapshot(&world.base);
        keep(&mut world, false).unwrap();
        assert_eq!(
            fs::read_to_string(world.base.join("keepdir/k2")).unwrap(),
            "k2"
        );
        assert_eq!(
            fs::read_to_string(world.undo_dir().join("files/src/a")).unwrap(),
            "one"
        );
        undo(&mut world, false).unwrap();
        assert_eq!(snapshot(&world.base), base_before);
    }
}
