//! Running a command inside a world, through bubblewrap, without root.
//!
//! Ubuntu 24.04 and later block plain user namespaces for ordinary programs, but allow `bwrap`. So a cell is a
//! `bwrap` process, sealed: the computer read-only; the home folders, `/media` and `/mnt` empty; the world's overlay
//! mounted over its base folder's own path, so tools see the usual paths; `/run` and Arbor's own state hidden; a
//! cleared environment; no network, no new user namespaces and an empty session keyring; and a filter that refuses
//! the kernel calls sandbox escapes start from (see `seccomp`). Its one way out is a socket to the outbox, where it
//! asks for effects that a person may release (see `outbox`). A key (see `key`) opens more folders and files and
//! passes variables in; password files stay hidden even then. Where the user's systemd manager allows it, each cell
//! also runs in its own cgroup (see `cgroup`), with its limits, and its whole cost is read from there. A command
//! the cell would not find is refused before it starts, in plain words: where it is hidden, and the key option that
//! opens it (see `View::check`). Network through a filter comes later.

use std::env;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::ExitStatusExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cgroup::{self, Group, Limits, Usage};
use crate::error::{Context, Error, Result};
use crate::key::{self, Key};
use crate::ledger::Ledger;
use crate::outbox::{self, Outbox};
use crate::world::{self, Lock, State, World};
use crate::{paths, seccomp, sys, table, tree};

pub struct Options {
    /// The command will talk to the person at the terminal.
    pub interactive: bool,
    /// The limits it runs under: its key's and the command line's, the tighter of each.
    pub limits: Limits,
    /// The key that opens more than the world, checked by the caller.
    pub key: Option<Key>,
}

/// The variables a cell keeps from the environment of the arbor that starts it, besides `LC_*`. A key passes
/// more in by name.
const KEPT: [&str; 14] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "LANG",
    "LANGUAGE",
    "TZ",
    "EDITOR",
    "VISUAL",
    "PAGER",
];

fn kept(name: &OsStr, key: Option<&Key>) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    KEPT.contains(&name)
        || name.starts_with("LC_")
        || key.is_some_and(|k| k.env.iter().any(|v| v == name))
}

/// Finds `name` on `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    env::var_os("PATH")
        .into_iter()
        .flat_map(|p| env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

pub fn bwrap() -> Result<PathBuf> {
    which("bwrap").ok_or_else(|| {
        Error::new("Arbor needs bubblewrap (the `bwrap` command) to run commands in a world; on Ubuntu: sudo apt install bubblewrap")
    })
}

/// The first bubblewrap that mounts overlays (`--overlay-src`).
const BWRAP_MIN: (u32, u32) = (0, 10);

/// Reads `bubblewrap 0.11.1` as (0, 11).
fn bwrap_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split_whitespace().nth(1)?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Checks once, when a world is made, that bubblewrap can mount its overlay, so an old one is named in plain words
/// instead of failing later with "Unknown option --overlay-src".
pub fn check_bwrap() -> Result<()> {
    let bwrap = bwrap()?;
    let out = Command::new(&bwrap)
        .arg("--version")
        .output()
        .ctx(|| format!("cannot run {}", bwrap.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match bwrap_version(&text) {
        Some(v) if v >= BWRAP_MIN => Ok(()),
        _ => Err(Error::new(format!(
            "Arbor needs bubblewrap {}.{} or later, for overlays, and {} is \"{}\". \
             Ubuntu 26.04 ships 0.11 (sudo apt install bubblewrap); Ubuntu 24.04's 0.9 is too old.",
            BWRAP_MIN.0,
            BWRAP_MIN.1,
            bwrap.display(),
            text.trim()
        ))),
    }
}

/// One mount of a cell's filesystem. bwrap makes them in order, each on top of the ones before it, so what a cell
/// sees at a path is what the last mount over that path shows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mount {
    /// The computer, read-only.
    Root,
    Dev,
    Proc,
    /// An empty folder whose writes vanish: a fresh `/tmp` or `/run`, the home folders, arbor's own state.
    Empty,
    /// An empty folder made inside an empty one, so the cell can stand in the home folder.
    Dir,
    /// The world: its changes over its base folder.
    World,
    /// A folder a key opens. Writes to it are thrown away, so tools that keep caches there still work.
    Folder,
    /// A file a key opens, read-only.
    File,
    /// A folder of password files, empty.
    Secrets,
    /// A password file, which reads empty.
    Secret,
}

/// What a cell is made of, apart from its way out to the outbox: its mounts in the order bwrap makes them.
pub struct View {
    mounts: Vec<(PathBuf, Mount)>,
    /// The home folder, resolved.
    home: Option<PathBuf>,
}

fn add(a: &mut Vec<OsString>, items: &[&dyn AsRef<OsStr>]) {
    a.extend(items.iter().map(|s| s.as_ref().to_os_string()));
}

impl View {
    /// The mounts of a cell in `world`, opened further by `key`.
    pub fn new(world: &World, key: Option<&Key>) -> Result<View> {
        let mut m: Vec<(PathBuf, Mount)> = vec![
            ("/".into(), Mount::Root),
            ("/dev".into(), Mount::Dev),
            ("/proc".into(), Mount::Proc),
            ("/tmp".into(), Mount::Empty),
            // /run holds the computer's and the session's sockets: Docker (which is root), D-Bus, systemd, snapd,
            // the session bus, Wayland and agents. Each is a way out of the cell.
            ("/run".into(), Mount::Empty),
        ];
        // The home folders and mounted drives are empty, and writes there vanish: only the world and what its key
        // opens show through.
        let home = paths::home()
            .and_then(|h| fs::canonicalize(h).ok())
            .filter(|h| h.is_dir() && h != Path::new("/"));
        let mut hidden: Vec<PathBuf> = ["/home", "/media", "/mnt"]
            .into_iter()
            .filter_map(|p| fs::canonicalize(p).ok())
            .chain(home.clone())
            .filter(|p| p.is_dir() && p != Path::new("/") && !paths::contains(&world.base, p))
            .collect();
        hidden.sort_by_key(|p| p.components().count());
        for (i, dir) in hidden.iter().enumerate() {
            if !hidden[..i].iter().any(|h| paths::contains(h, dir)) {
                m.push((dir.clone(), Mount::Empty));
            }
        }
        if let Some(home) = &home
            && hidden.iter().any(|h| paths::contains(h, home))
        {
            m.push((home.clone(), Mount::Dir));
        }
        // What shows through, outer paths first so inner ones land on top.
        let mut shown = vec![(world.base.clone(), Mount::World)];
        for p in key.iter().flat_map(|k| &k.read) {
            if let Ok(meta) = fs::metadata(p) {
                let how = if meta.is_dir() {
                    Mount::Folder
                } else {
                    Mount::File
                };
                shown.push((p.clone(), how));
            }
        }
        shown.sort_by_key(|(p, _)| p.components().count());
        // Password files stay hidden wherever the cell would see them, inside what shows through too, unless the world or
        // the key is that file or folder itself. What the key opens inside one of them goes on top.
        let shown_paths: Vec<PathBuf> = shown.iter().map(|(p, _)| p.clone()).collect();
        let masks = home
            .as_deref()
            .map_or_else(Vec::new, |h| key::secrets_in(h, &hidden, &shown_paths));
        let masked = |p: &Path| masks.iter().any(|(s, _)| paths::contains(s, p));
        for (p, how) in &shown {
            // What lies inside the world is in it already.
            let in_world = *how != Mount::World && paths::contains(&world.base, p);
            if !masked(p) && !in_world {
                m.push((p.clone(), *how));
            }
        }
        for (s, dir) in &masks {
            m.push((s.clone(), if *dir { Mount::Secrets } else { Mount::Secret }));
        }
        for (p, how) in &shown {
            if masked(p) {
                m.push((p.clone(), *how));
            }
        }
        // What the cell must not see: every world's changes, and the session's sockets if they live outside /run.
        m.push((fs::canonicalize(paths::state_dir()?)?, Mount::Empty));
        if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR")
            .and_then(|r| fs::canonicalize(r).ok())
            .filter(|r| r.is_dir() && !r.starts_with("/run"))
        {
            m.push((runtime, Mount::Empty));
        }
        Ok(View { mounts: m, home })
    }

    /// The `bwrap` arguments for a cell with these mounts in `world` that runs `cmd` in `cwd` with `path` as its
    /// `PATH`, with the seccomp filter read from the descriptor `filter` and its outbox socket at `socket`.
    #[allow(clippy::too_many_arguments)]
    fn args(
        &self,
        world: &World,
        cmd: &[OsString],
        cwd: &Path,
        path: &OsStr,
        filter: RawFd,
        socket: &Path,
        opts: &Options,
    ) -> Result<Vec<OsString>> {
        let mut a: Vec<OsString> = Vec::new();
        for (p, how) in &self.mounts {
            match how {
                Mount::Root | Mount::File => add(&mut a, &[&"--ro-bind", p, p]),
                Mount::Dev => add(&mut a, &[&"--dev", p]),
                Mount::Proc => add(&mut a, &[&"--proc", p]),
                Mount::Empty | Mount::Secrets => add(&mut a, &[&"--tmpfs", p]),
                Mount::Dir => add(&mut a, &[&"--dir", p]),
                Mount::World => add(
                    &mut a,
                    &[
                        &"--overlay-src",
                        &world.base,
                        &"--overlay",
                        &world.up(),
                        &world.work(),
                        &world.base,
                    ],
                ),
                Mount::Folder => add(&mut a, &[&"--overlay-src", p, &"--tmp-overlay", p]),
                // A plain bind would be `nodev`, and reading the null device there fails; this one reads empty.
                Mount::Secret => add(&mut a, &[&"--dev-bind", &"/dev/null", p]),
            }
        }
        // The cell's one way out: its socket to the outbox, and the arbor command that asks through it.
        let exe = env::current_exe().ctx(|| "cannot find the arbor command".to_string())?;
        let arbor = Path::new(outbox::CELL_BIN).join("arbor");
        add(
            &mut a,
            &[
                &"--bind",
                &socket,
                &outbox::CELL_SOCKET,
                &"--ro-bind",
                &exe,
                &arbor,
            ],
        );
        // No network, and no user namespaces of its own: those would reopen what bwrap closed. The computer's name is
        // the cell's own.
        add(
            &mut a,
            &[
                &"--unshare-all",
                &"--unshare-user",
                &"--disable-userns",
                &"--die-with-parent",
                &"--hostname",
                &"arbor",
            ],
        );
        // The kernel calls a cell may not make (see `seccomp`). bwrap loads the filter last, right before the command.
        add(&mut a, &[&"--seccomp", &filter.to_string()]);
        // A command run at a terminal shares the person's session, so shells keep job control; the filter keeps it from
        // typing into that terminal. Any other gets a session of its own.
        if !opts.interactive {
            add(&mut a, &[&"--new-session"]);
        }
        add(
            &mut a,
            &[
                &"--chdir",
                &cwd,
                &"--setenv",
                &"ARBOR_WORLD",
                &world.name.as_str(),
                &"--setenv",
                &outbox::SOCKET_VAR,
                &outbox::CELL_SOCKET,
                &"--setenv",
                &"PATH",
                &path,
                &"--",
            ],
        );
        a.extend(cmd.iter().cloned());
        Ok(a)
    }

    /// The last mount over `p`, absolute and without `.` or `..`: it decides what the cell sees there.
    fn over(&self, p: &Path) -> Mount {
        self.mounts
            .iter()
            .rev()
            .find(|(m, _)| paths::contains(m, p))
            .map_or(Mount::Root, |(_, how)| *how)
    }

    /// What the cell finds at `p`, an absolute path, following links as it would.
    fn at(&self, world: &World, p: &Path, hops: u32) -> At {
        let p = normal(p);
        if hops > 8 {
            return At::Unsure;
        }
        if p == Path::new(outbox::CELL_BIN).join("arbor") {
            return At::Runs(p);
        }
        let file = match self.over(&p) {
            Mount::Root | Mount::Folder | Mount::File => p.clone(),
            Mount::World => match world_file(world, &p) {
                Some(Some(file)) => file,
                Some(None) => return At::Absent,
                None => return At::Unsure,
            },
            // A fresh /dev or /proc has what the computer's has not, and the other way round.
            Mount::Dev | Mount::Proc => return At::Unsure,
            Mount::Empty | Mount::Dir | Mount::Secrets | Mount::Secret => {
                // The cell sees nothing here; the computer may have it, and where the file really is.
                return match fs::metadata(&p) {
                    Ok(meta) if meta.is_file() => {
                        let mut places = vec![p.clone()];
                        if let Ok(real) = fs::canonicalize(&p)
                            && real != p
                            && !matches!(self.at(world, &real, hops + 1), At::Runs(_))
                        {
                            places.push(real);
                        }
                        At::Hidden(places)
                    }
                    _ => At::Absent,
                };
            }
        };
        // A folder on the way may be a link, which the cell follows to wherever it points.
        if file == p {
            let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else {
                return At::Absent;
            };
            match fs::canonicalize(parent) {
                Ok(real) if real != parent => return self.at(world, &real.join(name), hops + 1),
                Ok(_) => {}
                Err(_) => return At::Absent,
            }
        }
        match fs::symlink_metadata(&file) {
            Ok(meta) if meta.file_type().is_symlink() => match fs::read_link(&file) {
                Ok(to) => self.at(world, &p.parent().unwrap_or(&p).join(to), hops + 1),
                Err(_) => At::Unsure,
            },
            // One it may not run makes `execvp` look further and fail otherwise: bwrap says so then.
            Ok(meta) if meta.is_file() && meta.mode() & 0o111 == 0 => At::Unsure,
            Ok(meta) if meta.is_file() => At::Runs(file),
            _ => At::Absent,
        }
    }

    /// What the cell runs for `name`, as `execvp` finds it: at that path from `cwd` when it holds a `/`, else in the
    /// first folder on `path` that has it.
    fn find(&self, world: &World, name: &OsStr, cwd: &Path, path: &OsStr) -> At {
        if name.as_bytes().contains(&b'/') {
            return self.at(world, &cwd.join(name), 0);
        }
        let mut hidden = None;
        for dir in env::split_paths(path) {
            match self.at(world, &cwd.join(dir).join(name), 0) {
                At::Hidden(places) => {
                    hidden.get_or_insert(places);
                }
                At::Absent => {}
                found => return found,
            }
        }
        hidden.map_or(At::Absent, At::Hidden)
    }

    /// Refuses, in arbor's words, a command the cell would not find: one only in places it hides, with the key that
    /// would open them, or one it has nowhere. A script whose interpreter is hidden is refused the same way.
    fn check(
        &self,
        world: &World,
        cmd: &[OsString],
        cwd: &Path,
        path: &OsStr,
        key: Option<&Key>,
    ) -> Result<()> {
        let Some(name) = cmd.first() else {
            return Ok(());
        };
        let quoted = |n: &OsStr| format!("`{}`", shown(&[n]));
        let (what, places) = match self.find(world, name, cwd, path) {
            At::Runs(file) => match interpreter(&file) {
                Some(by) => match self.find(world, &by, cwd, path) {
                    At::Hidden(places) => (
                        format!(
                            "{} needs {}, and {} is",
                            quoted(name),
                            quoted(&by),
                            quoted(&by)
                        ),
                        places,
                    ),
                    _ => return Ok(()),
                },
                None => return Ok(()),
            },
            At::Hidden(places) => (format!("{} is", quoted(name)), places),
            At::Absent => {
                return Err(Error::not_found(format!(
                    "world {} has no {}",
                    world.name,
                    if name.as_bytes().contains(&b'/') {
                        paths::tilde(&normal(&cwd.join(name)))
                    } else {
                        format!("command {}", quoted(name))
                    }
                )));
            }
            At::Unsure => return Ok(()),
        };
        let folders: Vec<String> = places
            .iter()
            .map(|p| paths::tilde(p.parent().unwrap_or(p)))
            .collect();
        let mut msg = format!("{what} in {}, which the world hides", folders.join(" and "));
        if let Some(k) = key {
            msg += &format!(" and key '{}' does not open", k.name);
        }
        match openers(&places, self.home.as_deref(), &key::toolchains()) {
            Some(opens) => {
                let name = ["dev", "tools"]
                    .into_iter()
                    .map(String::from)
                    .chain((2..).map(|n| format!("tools{n}")))
                    .find(|n| !key::exists(n))
                    .unwrap_or_default();
                msg += &format!(
                    ". A key opens it: `arbor key new {name} {}`, then add `--key {name}`.",
                    opens.join(" ")
                );
            }
            None => msg += ", and a key cannot open it.",
        }
        Err(Error::not_found(msg))
    }
}

/// What a cell finds at a path.
#[derive(Debug, PartialEq)]
enum At {
    /// A file it can run, at this path of the computer, or of the world's changes or base.
    Runs(PathBuf),
    /// Nothing it can see; the computer has the file in these places, which the cell does not see.
    Hidden(Vec<PathBuf>),
    Absent,
    /// arbor cannot tell for sure, so the command runs and bwrap says what went wrong, if anything.
    Unsure,
}

/// `p` with `.` and `..` worked out from its text alone.
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in p.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            part => out.push(part),
        }
    }
    out
}

/// Where the world keeps its version of `p`, a path inside its base folder: `Some(Some(file))` in its changes or its
/// base, `Some(None)` where it deleted `p` or replaced a folder on the way whole, and `None` where arbor cannot tell,
/// as for a link on the way among its changes.
fn world_file(world: &World, p: &Path) -> Option<Option<PathBuf>> {
    let rel = p.strip_prefix(&world.base).ok()?;
    let changed = world.up().join(rel);
    let mut at = world.up();
    let mut replaced = false;
    for part in rel.components() {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(meta) if sys::is_whiteout(&meta) => return Some(None),
            Ok(_) if at == changed => return Some(Some(at)),
            Ok(meta) if meta.file_type().is_symlink() => return None,
            Ok(meta) if meta.is_dir() => {
                replaced |= sys::get_xattr(&at, tree::OPAQUE)
                    .ok()
                    .flatten()
                    .is_some_and(|v| v == b"y");
            }
            Ok(_) => return Some(None),
            Err(_) => break,
        }
    }
    Some((!replaced).then(|| p.to_path_buf()))
}

/// The interpreter a script names on its first line, as `#!/bin/sh` or `#!/usr/bin/env node` do: a path, or a
/// name to find on `PATH`.
fn interpreter(file: &Path) -> Option<OsString> {
    let mut head = [0u8; 256];
    let n = File::open(file).and_then(|mut f| f.read(&mut head)).ok()?;
    let line = head[..n].strip_prefix(b"#!")?;
    let line = &line[..line.iter().position(|&b| b == b'\n').unwrap_or(line.len())];
    let mut words = line
        .split(|b| b.is_ascii_whitespace())
        .filter(|w| !w.is_empty());
    let program = words.next()?;
    if program.ends_with(b"/env") {
        let name = words.next().filter(|w| !w.starts_with(b"-"))?;
        return Some(OsStr::from_bytes(name).to_os_string());
    }
    Some(OsStr::from_bytes(program).to_os_string())
}

/// Folders of the home folder that hold many programs' files, each in a folder of its own: a key opens that one.
const SHARED: [&str; 6] = [
    ".local",
    ".local/share",
    ".local/state",
    ".config",
    ".cache",
    ".var/app",
];

/// The `arbor key new` options that open `places` to a cell: `--toolchains` for a folder among `toolchains`, else
/// `--read` with the folder of the home folder that holds it, or else its own folder. None when a key may not open
/// one of them.
fn openers(places: &[PathBuf], home: Option<&Path>, toolchains: &[PathBuf]) -> Option<Vec<String>> {
    let mut opens: Vec<String> = Vec::new();
    for place in places {
        // Where the file is, through linked folders on the way; a link itself stays where it is.
        let real = match (place.parent().map(fs::canonicalize), place.file_name()) {
            (Some(Ok(dir)), Some(name)) => dir.join(name),
            _ => place.clone(),
        };
        let open = if toolchains.iter().any(|t| paths::contains(t, &real)) {
            "--toolchains".to_string()
        } else {
            let folder = match home.and_then(|h| Some((h, real.strip_prefix(h).ok()?))) {
                Some((h, rel)) => {
                    let mut folder = h.to_path_buf();
                    for part in rel.components() {
                        folder.push(part);
                        let r = folder.strip_prefix(h).unwrap_or(&folder);
                        if !SHARED.iter().any(|s| Path::new(s) == r) {
                            break;
                        }
                    }
                    folder
                }
                None => real.parent().unwrap_or(&real).to_path_buf(),
            };
            key::check_grant(&folder).ok()?;
            format!("--read {}", paths::tilde(&folder))
        };
        if !opens.contains(&open) {
            opens.push(open);
        }
    }
    opens.sort_by_key(|o| o != "--toolchains");
    Some(opens)
}

/// The cell's `PATH`: first the folder of the arbor command that asks for effects, then the person's.
fn cell_path() -> OsString {
    let mut path = OsString::from(outbox::CELL_BIN);
    path.push(":");
    path.push(env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into()));
    path
}

/// Where the command starts: the same folder when the person stands inside the base, else the base itself.
pub fn start_dir(world: &World) -> PathBuf {
    match env::current_dir()
        .ok()
        .and_then(|d| fs::canonicalize(d).ok())
    {
        Some(dir) if paths::contains(&world.base, &dir) => dir,
        _ => world.base.clone(),
    }
}

/// A command as a shell would take it back: arguments with spaces or quotes are quoted, and those with characters a
/// terminal would act on or not show, such as escape sequences, newlines and direction marks, are spelled out in
/// `$'…'`, so what is shown is what runs.
pub fn shown<S: AsRef<OsStr>>(cmd: &[S]) -> String {
    let word = |a: &S| {
        let bytes = a.as_ref().as_bytes();
        let plain = !bytes.is_empty()
            && bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(b));
        match std::str::from_utf8(bytes) {
            Ok(a) if plain => a.to_string(),
            Ok(a) if !a.chars().any(table::unseen) => format!("'{}'", a.replace('\'', "'\\''")),
            _ => table::ansi_quoted(bytes),
        }
    };
    cmd.iter().map(word).collect::<Vec<_>>().join(" ")
}

/// A running cell, as `arbor ps` lists it. Each has a file in the state folder's `cells`, locked by the arbor
/// that runs it, so a file nobody holds was left by a crash.
#[derive(Debug, Serialize, Deserialize)]
pub struct Running {
    pub id: String,
    pub world: String,
    pub what: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub limits: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The arbor that runs it, which `stop` asks to end it.
    pub pid: libc::pid_t,
    /// When it started, in nanoseconds since 1970.
    pub started_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cgroup: Option<PathBuf>,
}

/// A running cell's file, removed when dropped.
struct Entry {
    path: PathBuf,
    _file: File,
}

impl Drop for Entry {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn running_dir() -> Result<PathBuf> {
    Ok(paths::state_dir()?.join("cells"))
}

/// Writes the cell's file under a temporary name, locks it, then moves it into place, so `arbor ps` never finds
/// it unlocked while the cell runs.
fn register(cell: &Running) -> Result<Entry> {
    let dir = running_dir()?;
    fs::create_dir_all(&dir).ctx(|| format!("cannot create {}", dir.display()))?;
    let tmp = dir.join(format!(".{}.tmp", cell.id));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .ctx(|| format!("cannot create {}", tmp.display()))?;
    let path = dir.join(format!("{}.json", cell.id));
    let placed = sys::flock(&file, true, false)
        .and_then(|()| file.write_all(&serde_json::to_vec(cell).map_err(io::Error::other)?))
        .and_then(|()| sys::rename_noreplace(&tmp, &path));
    if let Err(e) = placed {
        let _ = fs::remove_file(&tmp);
        return Err(e).ctx(|| format!("cannot write {}", path.display()));
    }
    Ok(Entry { path, _file: file })
}

/// The running cells, oldest first. Files left by a crash are removed, with the cgroup they name.
pub fn running() -> Result<Vec<Running>> {
    let dir = running_dir()?;
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).ctx(|| format!("cannot read {}", dir.display())),
    };
    let mut cells = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Ok(file) = File::open(&path) else {
            continue; // it ended meanwhile
        };
        let cell: Option<Running> = serde_json::from_reader(&file).ok();
        match sys::flock(&file, false, false) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => cells.extend(cell),
            Err(e) => return Err(e.into()),
            Ok(()) => {
                // Its arbor is gone, and the cell died with it; make sure, then clean up.
                if let Some(group) = cell
                    .and_then(|c| c.cgroup)
                    .filter(|g| g.starts_with(cgroup::cells_dir()))
                {
                    cgroup::kill_dir(&group);
                    cgroup::remove_dir(&group);
                }
                let _ = fs::remove_file(path.with_extension("sock"));
                let _ = fs::remove_file(&path);
            }
        }
    }
    cells.sort_by_key(|c| c.started_ns);
    Ok(cells)
}

/// Stops a running cell: asks the arbor that runs it to end it, so that arbor records how it ended, and waits up to
/// `grace` for that arbor to finish. Where it has not by then, the cell is killed directly: its cgroup, or else that
/// arbor, which takes the cell with it. Returns false when the cell had already ended.
pub fn stop(cell: &Running, grace: Duration) -> Result<bool> {
    let path = running_dir()?.join(format!("{}.json", cell.id));
    let Ok(file) = File::open(&path) else {
        return Ok(false);
    };
    let Ok(arbor) = sys::pidfd_open(cell.pid) else {
        return Ok(false);
    };
    // The file stays locked while its arbor runs the cell, so the process the descriptor names is that arbor.
    match sys::flock(&file, false, false) {
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
        Ok(()) => return Ok(false),
        Err(e) => return Err(e.into()),
    }
    drop(file);
    match sys::pidfd_send_signal(&arbor, libc::SIGTERM) {
        Err(e) if e.raw_os_error() == Some(libc::ESRCH) => return Ok(false),
        other => other?,
    }
    if !sys::readable_within(arbor.as_fd(), grace)? {
        match cell
            .cgroup
            .as_ref()
            .filter(|g| g.starts_with(cgroup::cells_dir()))
        {
            Some(group) => cgroup::kill_dir(group),
            None => {
                let _ = sys::pidfd_send_signal(&arbor, libc::SIGKILL);
            }
        }
    }
    Ok(true)
}

/// How a command in a cell ended.
pub struct Ran {
    /// The cell's name, as `arbor ps` and the ledger show it.
    pub id: String,
    /// Its exit code, or 128 + the signal that killed it.
    pub code: i32,
    /// How it ended, for the ledger: `exit N`, or what stopped it, such as `killed: time limit`.
    pub out: String,
    /// A line for the person when a limit, a key or a request stopped it.
    pub warning: Option<String>,
    /// CPU time of all its processes.
    pub cpu_ms: u64,
    /// Peak memory in KiB: the whole cell's when it had its own cgroup, else its largest process's.
    pub mem_kib: u64,
    /// Peak number of processes and threads, when it had its own cgroup.
    pub pids: Option<u64>,
}

/// The cell's own cgroup, where this session can have one. Memory, CPU and process limits need it; a cell
/// without them runs without one when it cannot have one.
fn cell_group(limits: &Limits) -> Result<Option<Group>> {
    if let Some(why) = cgroup::unavailable() {
        return if limits.need_cgroup() {
            Err(Error::refused(format!(
                "--mem, --cpu and --pids need a cgroup, and {why}; --time works without one"
            )))
        } else {
            Ok(None)
        };
    }
    match cgroup::ensure_service().and_then(|cells| Group::create(&cells, limits)) {
        Ok(group) => Ok(Some(group)),
        Err(e) if limits.need_cgroup() => Err(e),
        Err(_) => Ok(None),
    }
}

fn c_string(s: &OsStr) -> Result<CString> {
    CString::new(s.as_bytes()).map_err(|_| Error::usage("an argument holds a NUL byte"))
}

/// The pointer array `execve` takes, ending in null. It points into `items`, which must outlive it.
fn pointers(items: &[CString]) -> Vec<*const libc::c_char> {
    items
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect()
}

/// Starts `program` with `argv` and `envp`, inside the cell's cgroup when it has one, with the session keyring
/// left behind, death tied to arbor's, and `filter` left open for it. Returns its process ID and a descriptor that
/// becomes readable when it exits.
fn spawn(
    program: &CStr,
    argv: &[*const libc::c_char],
    envp: &[*const libc::c_char],
    group: Option<&Group>,
    filter: BorrowedFd,
) -> io::Result<(libc::pid_t, OwnedFd)> {
    let (report, told) = sys::pipe()?;
    let parent = unsafe { libc::getpid() };
    match unsafe { sys::fork_into(group.map(Group::folder))? } {
        sys::Forked::Child { placed } => {
            // Between fork and exec: only calls that are safe there, then exec or `_exit`.
            let fail = |err: libc::c_int| -> ! {
                unsafe {
                    libc::write(told.as_raw_fd(), (&raw const err).cast(), 4);
                    libc::_exit(127)
                }
            };
            let errno = || unsafe { *libc::__errno_location() };
            unsafe {
                let mut none: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut none);
                libc::pthread_sigmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
            }
            if !placed
                && let Some(g) = group
                && cgroup::join(g.procs()).is_err()
            {
                fail(errno());
            }
            // The login session's keyring stays outside. Where keyctl is blocked (as in Docker's default
            // profile), the cell cannot reach a keyring either, so a failure is ignored.
            let _ = sys::new_session_keyring();
            // A cell never outlives the arbor that holds its world's lock.
            if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } != 0
                || unsafe { libc::getppid() } != parent
            {
                fail(libc::ESRCH);
            }
            // bwrap reads the seccomp filter from this descriptor, so it stays open across exec.
            if unsafe { libc::fcntl(filter.as_raw_fd(), libc::F_SETFD, 0) } != 0 {
                fail(errno());
            }
            unsafe { libc::execve(program.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
            fail(errno())
        }
        sys::Forked::Parent { pid, exited } => {
            drop(told);
            // Four bytes are an errno from the child; none means it reached exec.
            let mut err = [0u8; 4];
            let n = loop {
                let n = unsafe { libc::read(report.as_raw_fd(), err.as_mut_ptr().cast(), 4) };
                if n >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    break n;
                }
            };
            if n == 4 {
                unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) };
                return Err(io::Error::from_raw_os_error(i32::from_ne_bytes(err)));
            }
            Ok((pid, exited))
        }
    }
}

/// Why arbor ended a cell itself.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ended {
    TimeLimit,
    KeyExpired,
    /// Someone asked: `arbor stop`, a closed terminal or a signal to arbor.
    Stopped,
    KeyTakenBack,
}

/// Waits for the cell's first process. At the deadline, or when arbor is asked to stop (SIGTERM, SIGHUP), it kills
/// the whole cell and says why.
fn wait(
    pid: libc::pid_t,
    exited: &OwnedFd,
    deadline: Option<(Instant, Ended)>,
    group: Option<&Group>,
    stops: &sys::Interrupts,
) -> io::Result<(ExitStatus, Option<Ended>)> {
    let ended = loop {
        if stops.interrupted() {
            break Some(Ended::Stopped);
        }
        let left = deadline.map(|(at, why)| (at.saturating_duration_since(Instant::now()), why));
        if let Some((left, why)) = left
            && left.is_zero()
        {
            break Some(why);
        }
        if stops.poll(exited.as_fd(), left.map(|(left, _)| left))? {
            break None;
        }
    };
    if ended.is_some() {
        match group {
            Some(g) => g.kill(),
            // Not reaped yet, so its process ID still names it.
            None => unsafe {
                libc::kill(pid, libc::SIGKILL);
            },
        }
    }
    let mut status = 0;
    while unsafe { libc::waitpid(pid, &mut status, 0) } < 0 {
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
    Ok((ExitStatus::from_raw(status), ended))
}

/// What the ledger says about how a cell ended, and a line for the person when something stopped it.
fn outcome(
    code: i32,
    ended: Option<Ended>,
    usage: Option<&Usage>,
    limits: &Limits,
    key: Option<&Key>,
) -> (String, Option<String>) {
    let key_name = key.map(|k| k.name.as_str()).unwrap_or_default();
    let said = |out: &str, line: String| (out.to_string(), Some(line));
    match ended {
        Some(Ended::TimeLimit) => {
            let limit = limits.time.map(cgroup::duration).unwrap_or_default();
            return said(
                "killed: time limit",
                format!("the command reached its time limit ({limit}) and was stopped"),
            );
        }
        Some(Ended::KeyExpired) => {
            let at = key
                .map(|k| key::clock(k.until_ns, world::now_ns()))
                .unwrap_or_default();
            return said(
                "killed: key expired",
                format!("key '{key_name}' ended at {at}, and the command was stopped"),
            );
        }
        Some(Ended::Stopped) => return said("stopped", "the command was stopped".into()),
        Some(Ended::KeyTakenBack) => {
            return said(
                "stopped: key taken back",
                format!("key '{key_name}' was taken back, and the command was stopped"),
            );
        }
        None => {}
    }
    let oom = usage.is_some_and(|u| u.oom_kills > 0);
    if oom && let Some(mem) = limits.mem {
        return said(
            "killed: memory limit",
            format!(
                "the command reached its memory limit ({}) and was stopped",
                cgroup::size(mem)
            ),
        );
    }
    let mut out = format!("exit {code}");
    let mut warning = None;
    if oom {
        out += ", out of memory";
    }
    if let Some(pids) = limits.pids
        && usage.is_some_and(|u| u.pids_refused > 0)
    {
        out += ", process limit hit";
        warning = Some(format!(
            "the command reached its process limit ({pids}); new processes were refused"
        ));
    }
    (out, warning)
}

/// Runs `cmd` in the world. The caller holds the world's lock and loaded the world after taking it, and checked the
/// key; `started` is told the cell's name once it runs.
pub fn run(
    world: &World,
    _lock: &Lock,
    cmd: &[OsString],
    opts: &Options,
    started: &mut dyn FnMut(&str),
) -> Result<Ran> {
    if world.state != State::Open {
        return Err(Error::refused(format!(
            "world '{}' was kept; run `arbor undo {}` to reopen it",
            world.name, world.name
        )));
    }
    if !world.base.is_dir() {
        return Err(Error::new(format!(
            "the base folder {} is gone",
            paths::tilde(&world.base)
        )));
    }
    let bwrap = bwrap()?;
    let limits = &opts.limits;
    let key = opts.key.as_ref();
    let view = View::new(world, key)?;
    let cwd = start_dir(world);
    let path = cell_path();
    view.check(world, cmd, &cwd, &path, key)?;
    let group = cell_group(limits)?;
    let id = group
        .as_ref()
        .map_or_else(cgroup::new_id, |g| g.id().to_string());
    // From before `arbor stop` can find the cell, asking arbor to stop only raises a flag that the wait reads.
    let stops = sys::defer(&[libc::SIGTERM, libc::SIGHUP]);
    let _entry = register(&Running {
        id: id.clone(),
        world: world.name.clone(),
        what: shown(cmd),
        limits: limits.describe(),
        key: key.map(|k| k.name.clone()),
        pid: unsafe { libc::getpid() },
        started_ns: world::now_ns(),
        cgroup: group.as_ref().map(|g| g.dir().to_path_buf()),
    })?;
    // A key taken back after the caller checked it, but before the cell was listed, is caught here.
    if let Some(k) = key
        && !key::exists(&k.name)
    {
        return Err(Error::refused(format!("key '{}' was taken back", k.name)));
    }
    // Who asks through the cell's socket is known here, never taken from what the cell sends.
    let asker = outbox::Asker {
        world: world.name.clone(),
        base: world.base.clone(),
        cell: id.clone(),
        keys: key.map(Key::chain).transpose()?.unwrap_or_default(),
    };
    let socket = outbox::Socket::bind(&running_dir()?.join(format!("{id}.sock")))?;
    // bwrap reads the filter from a pipe until it closes. The filter fits in a pipe's buffer, so it is written whole
    // now and that end closed.
    let (filter, to_filter) = sys::pipe()?;
    File::from(to_filter).write_all(&seccomp::bytes())?;
    let args = view.args(
        world,
        cmd,
        &cwd,
        &path,
        filter.as_raw_fd(),
        socket.path(),
        opts,
    )?;
    // Everything exec needs is made before the fork: the child may not allocate.
    let program = c_string(bwrap.as_os_str())?;
    let argv = std::iter::once(OsStr::new("bwrap"))
        .chain(args.iter().map(OsString::as_os_str))
        .map(c_string)
        .collect::<Result<Vec<_>>>()?;
    let envp = env::vars_os()
        .filter(|(name, _)| kept(name, key))
        .map(|(mut name, value)| {
            name.push("=");
            name.push(value);
            c_string(&name)
        })
        .collect::<Result<Vec<_>>>()?;
    // bwrap's outer process exits as soon as the command does, without waiting for the cell's init, and the
    // cell's CPU time and memory would go with it. As a subreaper, arbor adopts that init and waits for it below,
    // so they count.
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) };
    let (cpu_before, _) = sys::children_usage();
    // The cell stops at its time limit or when its key ends, whichever comes first.
    let now = Instant::now();
    let deadline = [
        limits.time.map(|t| (now + t, Ended::TimeLimit)),
        key.map(|k| {
            let left = k.until_ns.saturating_sub(world::now_ns());
            (now + Duration::from_nanos(left), Ended::KeyExpired)
        }),
    ]
    .into_iter()
    .flatten()
    .min_by_key(|(at, _)| *at);
    let (pid, exited) = spawn(
        &program,
        &pointers(&argv),
        &pointers(&envp),
        group.as_ref(),
        filter.as_fd(),
    )
    .ctx(|| "cannot start the cell".to_string())?;
    drop(filter);
    started(&id);
    // Until the server answers, what the cell asks waits; without one, it is refused.
    let server = Outbox::open()
        .and_then(|o| Ok((o, Ledger::open()?)))
        .and_then(|(o, l)| socket.serve(asker, o, l));
    let server = match server {
        Ok(server) => Some(server),
        Err(e) => {
            eprintln!("arbor: warning: this command cannot ask for effects: {e}");
            None
        }
    };
    // Ctrl-C belongs to the command inside: Arbor waits for it instead of dying first and leaving it behind.
    let (old_int, old_quit) = unsafe {
        (
            libc::signal(libc::SIGINT, libc::SIG_IGN),
            libc::signal(libc::SIGQUIT, libc::SIG_IGN),
        )
    };
    let waited = wait(pid, &exited, deadline, group.as_ref(), &stops);
    // The adopted init dies with bwrap (--die-with-parent) and takes the cell's last processes with it.
    while unsafe { libc::waitpid(-1, std::ptr::null_mut(), 0) } > 0
        || io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
    {}
    unsafe {
        libc::signal(libc::SIGINT, old_int);
        libc::signal(libc::SIGQUIT, old_quit);
    }
    // Every process of the cell has ended, so its requests have too.
    drop(server);
    drop(stops);
    let (status, ended) = waited?;
    let code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    // Only arbor's own kill counts: a command that ended on its own a moment before stays as it ended.
    let ended = match ended.filter(|_| status.signal() == Some(libc::SIGKILL)) {
        Some(Ended::Stopped) if key.is_some_and(|k| !key::exists(&k.name)) => {
            Some(Ended::KeyTakenBack)
        }
        e => e,
    };
    let (cpu, max_rss) = sys::children_usage();
    let usage = group.as_ref().map(Group::usage);
    let (out, warning) = outcome(code, ended, usage.as_ref(), limits, key);
    Ok(Ran {
        id,
        code,
        out,
        warning,
        // systemd may have removed the cgroup first (`systemctl --user stop arbor`); getrusage still counts.
        cpu_ms: usage
            .and_then(|u| u.cpu_us)
            .map_or(cpu - cpu_before, |us| us / 1000),
        mem_kib: usage
            .and_then(|u| u.mem_peak)
            .map_or(max_rss, |b| b.div_ceil(1024)),
        pids: usage.and_then(|u| u.pids_peak),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bwrap_versions() {
        assert_eq!(bwrap_version("bubblewrap 0.11.1\n"), Some((0, 11)));
        assert_eq!(bwrap_version("bubblewrap 0.9.0"), Some((0, 9)));
        assert!(bwrap_version("bubblewrap 0.9.0") < Some(BWRAP_MIN));
        assert_eq!(bwrap_version("garbage"), None);
    }

    #[test]
    fn names_how_a_cell_ended() {
        let limits = Limits {
            mem: Some(64 << 20),
            pids: Some(8),
            time: Some(std::time::Duration::from_secs(2)),
            ..Limits::default()
        };
        let used = |oom_kills, pids_refused| Usage {
            oom_kills,
            pids_refused,
            ..Usage::default()
        };
        assert_eq!(
            outcome(0, None, Some(&used(0, 0)), &limits, None),
            ("exit 0".into(), None)
        );
        let (out, warning) = outcome(137, Some(Ended::TimeLimit), None, &limits, None);
        assert_eq!(out, "killed: time limit");
        assert!(warning.unwrap().contains("(2s)"));
        assert_eq!(
            outcome(137, None, Some(&used(1, 0)), &limits, None).0,
            "killed: memory limit"
        );
        assert_eq!(
            outcome(2, None, Some(&used(0, 3)), &limits, None).0,
            "exit 2, process limit hit"
        );
        assert_eq!(
            outcome(137, None, Some(&used(1, 0)), &Limits::default(), None).0,
            "exit 137, out of memory"
        );
        for (ended, out) in [
            (Ended::KeyExpired, "killed: key expired"),
            (Ended::Stopped, "stopped"),
            (Ended::KeyTakenBack, "stopped: key taken back"),
        ] {
            assert_eq!(outcome(137, Some(ended), None, &limits, None).0, out);
        }
    }

    #[test]
    fn the_last_mount_over_a_path_decides_what_a_cell_sees() {
        let p = PathBuf::from;
        let view = View {
            mounts: vec![
                (p("/"), Mount::Root),
                (p("/proc"), Mount::Proc),
                (p("/tmp"), Mount::Empty),
                (p("/home"), Mount::Empty),
                (p("/home/u"), Mount::Dir),
                (p("/home/u/shop"), Mount::World),
                (p("/home/u/.cargo"), Mount::Folder),
                (p("/home/u/.cargo/credentials.toml"), Mount::Secret),
                (p("/home/u/shop/.state"), Mount::Empty),
            ],
            home: Some(p("/home/u")),
        };
        for (path, seen) in [
            ("/usr/bin/env", Mount::Root),
            ("/proc/self/exe", Mount::Proc),
            ("/tmp/x", Mount::Empty),
            ("/home/v/bin/tool", Mount::Empty),
            ("/home/u/bin/tool", Mount::Dir),
            ("/home/u/shop/node_modules/.bin/jest", Mount::World),
            ("/home/u/shopping/x", Mount::Dir),
            ("/home/u/.cargo/bin/cargo", Mount::Folder),
            ("/home/u/.cargo/credentials.toml", Mount::Secret),
            ("/home/u/shop/.state/worlds", Mount::Empty),
        ] {
            assert_eq!(view.over(Path::new(path)), seen, "{path}");
        }
    }

    #[test]
    fn reads_paths_and_the_interpreters_scripts_name() {
        assert_eq!(normal(Path::new("/a/./b/../c")), Path::new("/a/c"));
        assert_eq!(normal(Path::new("/../x")), Path::new("/x"));
        let dir = env::temp_dir().join(format!("arbor-unit-interp-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for (script, by) in [
            ("#!/bin/sh\necho", Some("/bin/sh")),
            ("#! /usr/bin/env node --flag\n", Some("node")),
            ("#!/usr/bin/env -S node --flag\n", None),
            ("#!/usr/bin/python3 -u", Some("/usr/bin/python3")),
            ("\x7fELF", None),
            ("", None),
        ] {
            let file = dir.join("script");
            fs::write(&file, script).unwrap();
            assert_eq!(interpreter(&file), by.map(OsString::from), "{script:?}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_the_key_options_that_open_hidden_places() {
        let home = Path::new("/nowhere/u");
        let toolchains = [home.join(".cargo"), home.join(".local/bin")];
        let opens = |places: &[&str]| {
            let places: Vec<PathBuf> = places.iter().map(|p| home.join(p)).collect();
            openers(&places, Some(home), &toolchains)
        };
        let read = |rel: &str| format!("--read {}", paths::tilde(&home.join(rel)));
        assert_eq!(
            opens(&[".cargo/bin/cargo"]),
            Some(vec!["--toolchains".into()])
        );
        // A link in a toolchain's folder to a program's own folder: both open.
        assert_eq!(
            opens(&[".local/share/claude/versions/2/claude", ".local/bin/claude"]),
            Some(vec!["--toolchains".into(), read(".local/share/claude")])
        );
        assert_eq!(opens(&["bin/tool"]), Some(vec![read("bin")]));
        assert_eq!(opens(&["tool"]), Some(vec![read("tool")]));
        // Outside the home folder, the file's own folder; never what a key may not open.
        assert_eq!(
            openers(
                &[PathBuf::from("/mnt/disk/bin/tool")],
                Some(home),
                &toolchains
            ),
            Some(vec!["--read /mnt/disk/bin".into()])
        );
        assert_eq!(
            openers(
                &[PathBuf::from("/run/wrappers/bin/sudo")],
                Some(home),
                &toolchains
            ),
            None
        );
    }

    #[test]
    fn keeps_only_plain_variables_and_those_a_key_passes() {
        let mut key = Key {
            name: "k".into(),
            within: None,
            worlds: Vec::new(),
            read: Vec::new(),
            env: vec!["CARGO_HOME".into()],
            limits: Limits::default(),
            spend: None,
            made_ns: 0,
            until_ns: 1,
        };
        for name in ["PATH", "HOME", "LC_ALL", "TERM"] {
            assert!(kept(OsStr::new(name), None), "{name}");
        }
        for name in ["CARGO_HOME", "GITHUB_TOKEN", "SSH_AUTH_SOCK", "ARBOR_SCOPE"] {
            assert!(!kept(OsStr::new(name), None), "{name}");
        }
        assert!(kept(OsStr::new("CARGO_HOME"), Some(&key)));
        key.env.clear();
        assert!(!kept(OsStr::new("CARGO_HOME"), Some(&key)));
    }
}
