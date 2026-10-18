//! Running a command inside a world, through bubblewrap, without root.
//!
//! Ubuntu 24.04 and later block plain user namespaces for ordinary programs, but allow `bwrap`. So a cell is a
//! `bwrap` process, sealed: the computer read-only; the home folders, `/media` and `/mnt` empty; the world's overlay
//! mounted over its base folder's own path, so tools see the usual paths; `/run` and Arbor's own state hidden; a
//! cleared environment; no network, no new user namespaces and an empty session keyring; and a filter that refuses
//! the kernel calls sandbox escapes start from (see `seccomp`). Its one way out is a socket to the outbox, where it
//! asks for effects that a person may release (see `outbox`). A key (see `key`) opens more folders and files and
//! passes variables in; password files and agents' records of your sessions stay hidden even then, and those missing
//! when the cell starts stay missing (see `Layer`). A key can give the cell a home folder of its own, kept between
//! runs, in place of the empty one. A key can also open the network to the hosts it names: the cell's first process is
//! then arbor itself, which hands the arbor outside a port on the cell's loopback to answer as a proxy, and becomes
//! the command (see `net`). Where the user's systemd manager allows it, each cell also runs in its own cgroup (see
//! `cgroup`), with its limits, and its whole cost is read from there. A command the cell would not find is refused
//! before it starts, in plain words: where it is hidden, and the key option that opens it (see `View::check`).

use std::collections::HashSet;
use std::env;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
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
use crate::{net, paths, seccomp, sys, table, tree};

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

/// Becomes `cmd`, found as `execvp` finds it, as bwrap would have run it. Returns only when it cannot, after
/// saying why, with the code a shell gives: 127 for a command not found, 126 for one that cannot run.
pub fn exec(cmd: &[OsString]) -> i32 {
    let Ok(args) = cmd
        .iter()
        .map(|a| CString::new(a.as_bytes()))
        .collect::<std::result::Result<Vec<_>, _>>()
    else {
        eprintln!("arbor: cannot run {}: it holds a NUL byte", shown(cmd));
        return 126;
    };
    let Some(program) = args.first() else {
        return 126;
    };
    let argv = pointers(&args);
    unsafe { libc::execvp(program.as_ptr(), argv.as_ptr()) };
    let e = io::Error::last_os_error();
    eprintln!("arbor: cannot run {}: {e}", shown(&cmd[..1]));
    if e.kind() == io::ErrorKind::NotFound {
        127
    } else {
        126
    }
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
    /// The home folder of the key, read-write, in place of `Dir`: what is written there stays for the next run.
    Home,
    /// The world: its changes over its base folder.
    World,
    /// A folder a key opens. Writes to it are thrown away, so tools that keep caches there still work. In a world, the
    /// folder that holds an agent's database is one too, over the world, so that what the agent writes there never
    /// reaches the world's changes.
    Folder,
    /// A file a key opens, read-only.
    File,
    /// A folder of password files or agents' records, empty.
    Secrets,
    /// A password file or an agent's record, which reads empty.
    Secret,
    /// A folder of the computer that holds one of those, or where one is missing, read-only as `Root` shows it, but
    /// through an overlay of its own. The kernel takes a mount off an entry of the computer's when a program renames a
    /// new file over it or deletes it, which shows what replaced it, but leaves one on an overlay's entry, as in a
    /// world; and only an overlay can take a layer that keeps a missing one missing (see `Layer`).
    Holder,
}

/// What a cell is made of, apart from its way out to the outbox: its mounts in the order bwrap makes them.
pub struct View {
    mounts: Vec<(PathBuf, Mount)>,
    /// The home folder, resolved.
    home: Option<PathBuf>,
    /// The key's home folder, which the cell sees at `home` (`Mount::Home`).
    key_home: Option<PathBuf>,
    /// Where a password file or record is missing, the first part of its way that is, where the world, a folder the
    /// key opens or the computer itself would show what a program outside makes there while the cell runs, and each
    /// agent's database there, with the files beside it. A layer of whiteouts keeps each missing (see `Layer`).
    gaps: Vec<PathBuf>,
}

fn add(a: &mut Vec<OsString>, items: &[&dyn AsRef<OsStr>]) {
    a.extend(items.iter().map(|s| s.as_ref().to_os_string()));
}

impl View {
    /// The mounts of a cell in `world`, opened further by `key`, for a command that starts in `cwd`.
    pub fn new(world: &World, key: Option<&Key>, cwd: &Path) -> Result<View> {
        let mut m: Vec<(PathBuf, Mount)> = vec![
            ("/".into(), Mount::Root),
            ("/dev".into(), Mount::Dev),
            ("/proc".into(), Mount::Proc),
            ("/tmp".into(), Mount::Empty),
            // /run holds the computer's and the session's sockets: Docker (which is root), D-Bus, systemd, snapd,
            // the session bus, Wayland and agents. Each is a way out of the cell.
            ("/run".into(), Mount::Empty),
        ];
        let fresh = m.len();
        // The home folders and mounted drives are empty, and writes there vanish: only the world and what its key
        // opens show through. A key may give the cell a home folder of its own instead of the empty one, which stays.
        let home = paths::resolved_home();
        let key_home = key
            .map(Key::home_dir)
            .transpose()?
            .flatten()
            .map(|d| std::path::absolute(&d).unwrap_or(d));
        if let Some(k) = key.filter(|_| key_home.is_some()) {
            k.check_home(home.as_deref())?;
            match &home {
                None => {
                    return Err(Error::refused(format!(
                        "key '{}' has a home folder, but HOME names no folder to put it in",
                        k.name
                    )));
                }
                // The world shows the home folder itself there.
                Some(h) if paths::contains(&world.base, h) => {
                    return Err(Error::refused(format!(
                        "key '{}' has a home folder, so it cannot run in world {}, whose folder {} {} your home folder",
                        k.name,
                        world.name,
                        paths::tilde(&world.base),
                        if &world.base == h { "is" } else { "holds" }
                    )));
                }
                Some(_) => {}
            }
        }
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
            let how = if key_home.is_some() {
                Mount::Home
            } else {
                Mount::Dir
            };
            m.push((home.clone(), how));
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
        // Password files and agents' records stay hidden wherever the cell would see them, inside what shows through
        // too, unless the world or the key is that file or folder itself, and so do the records agents keep in a
        // project's folder. What the key opens inside one of them goes on top.
        let shown_paths: Vec<PathBuf> = shown.iter().map(|(p, _)| p.clone()).collect();
        let in_projects = project_records(world, &shown, cwd);
        let key::Hidden {
            masks,
            missing,
            databases,
        } = key::secrets_in(home.as_deref(), &hidden, &shown_paths, &in_projects);
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
        // SQLite cannot work with an agent's database hidden as a file, so each one, with the files SQLite keeps beside
        // it, is missing instead, in a folder whose writes are thrown away: the agent makes a new one there, which
        // lasts as long as the command. Right in `/`, where no overlay can keep one missing, one there reads empty
        // instead, as other records do.
        let (in_root, databases): (Vec<PathBuf>, Vec<PathBuf>) = databases
            .into_iter()
            .partition(|d| d.parent() == Some(Path::new("/")));
        m.extend(
            in_root
                .into_iter()
                .filter(|d| d.exists())
                .map(|d| (d, Mount::Secret)),
        );
        scratch(&mut m, &databases);
        // One that is missing stays missing where the cell would see what a program outside makes there: in the world,
        // in a folder the key opens, or on the computer itself.
        let mut gaps: Vec<PathBuf> = missing
            .into_iter()
            .chain(databases)
            .filter(|g| {
                g.parent().is_some_and(|dir| {
                    matches!(over(&m, dir), Mount::World | Mount::Folder | Mount::Root)
                })
            })
            .collect();
        gaps.sort();
        // A password file or record that the cell sees as the computer's own, one linked from the home folder to
        // elsewhere, gets its folder through an overlay, so that its mask stays when its program replaces it, and so
        // that one missing there stays missing. The overlay goes in before anything that may lie inside it, which
        // lands on top as before.
        let holders = holders(&m, &gaps);
        m.splice(
            fresh..fresh,
            holders.into_iter().map(|p| (p, Mount::Holder)),
        );
        // Right in `/`, there is no overlay to keep one missing.
        gaps.retain(|g| g.parent().is_some_and(|dir| over(&m, dir) != Mount::Root));
        Ok(View {
            mounts: m,
            home,
            key_home,
            gaps,
        })
    }

    /// The `bwrap` arguments for a cell with these mounts in `world` that runs `cmd` in `cwd` with `path` as its
    /// `PATH`, with the seccomp filter read from the descriptor `filter`, its outbox socket at `socket`, and `layer`
    /// on the overlays that would show a gap.
    #[allow(clippy::too_many_arguments)]
    fn args(
        &self,
        world: &World,
        cmd: &[OsString],
        cwd: &Path,
        path: &OsStr,
        filter: RawFd,
        socket: &Path,
        port: Option<RawFd>,
        layer: Option<&Layer>,
        opts: &Options,
    ) -> Result<Vec<OsString>> {
        let mut a: Vec<OsString> = Vec::new();
        // What an overlay at `p` shows: the computer's folder, then, on top, the layer that keeps its gaps missing.
        // bwrap stacks the sources in order, the last on top.
        let sources = |a: &mut Vec<OsString>, p: &Path| {
            add(a, &[&"--overlay-src", &p]);
            if let Some(top) = layer.and_then(|l| l.on(p)) {
                add(a, &[&"--overlay-src", &top]);
            }
        };
        for (p, how) in &self.mounts {
            match how {
                Mount::Root | Mount::File => add(&mut a, &[&"--ro-bind", p, p]),
                Mount::Dev => add(&mut a, &[&"--dev", p]),
                Mount::Proc => add(&mut a, &[&"--proc", p]),
                Mount::Empty | Mount::Secrets => add(&mut a, &[&"--tmpfs", p]),
                Mount::Dir => add(&mut a, &[&"--dir", p]),
                Mount::Home => match &self.key_home {
                    Some(dir) => add(&mut a, &[&"--bind", dir, p]),
                    None => add(&mut a, &[&"--dir", p]),
                },
                Mount::World => {
                    sources(&mut a, &world.base);
                    add(
                        &mut a,
                        &[&"--overlay", &world.up(), &world.work(), &world.base],
                    );
                }
                Mount::Folder => {
                    sources(&mut a, p);
                    add(&mut a, &[&"--tmp-overlay", p]);
                }
                // A plain bind would be `nodev`, and reading the null device there fails; this one reads empty.
                Mount::Secret => add(&mut a, &[&"--dev-bind", &"/dev/null", p]),
                // bwrap's read-only overlay needs two sources, so this is a throwaway one, made read-only.
                Mount::Holder => {
                    sources(&mut a, p);
                    add(&mut a, &[&"--tmp-overlay", p, &"--remount-ro", p]);
                }
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
            ],
        );
        // Under a key that opens hosts, tools find the cell's proxy through these, and arbor runs first to open its
        // port (see `net`).
        if port.is_some() {
            for (name, value) in net::vars() {
                add(&mut a, &[&"--setenv", &name, &value]);
            }
        }
        add(&mut a, &[&"--"]);
        if let Some(fd) = port {
            add(&mut a, &[&arbor, &"proxy-port", &fd.to_string(), &"--"]);
        }
        a.extend(cmd.iter().cloned());
        Ok(a)
    }

    /// The last mount over `p`, absolute and without `.` or `..`: it decides what the cell sees there.
    fn over(&self, p: &Path) -> Mount {
        over(&self.mounts, p)
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
            Mount::Root | Mount::Holder | Mount::Folder | Mount::File => p.clone(),
            Mount::World => match world_file(world, &p) {
                Some(Some(file)) => file,
                Some(None) => return At::Absent,
                None => return At::Unsure,
            },
            Mount::Home => match self.home_file(&p) {
                Some(Some(file)) => file,
                Some(None) => return self.unseen(world, &p, hops),
                None => return At::Unsure,
            },
            // A fresh /dev or /proc has what the computer's has not, and the other way round.
            Mount::Dev | Mount::Proc => return At::Unsure,
            Mount::Empty | Mount::Dir | Mount::Secrets | Mount::Secret => {
                return self.unseen(world, &p, hops);
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

    /// What the cell finds at `p`, where it sees nothing: the computer may have the file, and where it really is.
    fn unseen(&self, world: &World, p: &Path, hops: u32) -> At {
        match fs::metadata(p) {
            Ok(meta) if meta.is_file() => {
                let mut places = vec![p.to_path_buf()];
                if let Ok(real) = fs::canonicalize(p)
                    && real != p
                    && !matches!(self.at(world, &real, hops + 1), At::Runs(_))
                {
                    places.push(real);
                }
                At::Hidden(places)
            }
            _ => At::Absent,
        }
    }

    /// Where the key's home folder keeps `p`, a path inside the home folder: `Some(Some(file))` where it has
    /// something there, `Some(None)` where it has nothing, and `None` where a link on the way leaves arbor unsure.
    fn home_file(&self, p: &Path) -> Option<Option<PathBuf>> {
        let (Some(home), Some(key_home)) = (&self.home, &self.key_home) else {
            return Some(None);
        };
        let rel = p.strip_prefix(home).ok()?;
        let mut at = key_home.clone();
        let mut parts = rel.components().peekable();
        while let Some(part) = parts.next() {
            at.push(part);
            match fs::symlink_metadata(&at) {
                Ok(_) if parts.peek().is_none() => break,
                Ok(meta) if meta.file_type().is_symlink() => return None,
                Ok(meta) if meta.is_dir() => {}
                _ => return Some(None),
            }
        }
        Some(Some(at))
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
        // A link and the file it points to are often in the same folder, as rustup's are.
        let mut folders: Vec<String> = Vec::new();
        for p in &places {
            let folder = paths::tilde(p.parent().unwrap_or(p));
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
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

/// The last of `mounts` over `p`, absolute and without `.` or `..`.
fn over(mounts: &[(PathBuf, Mount)], p: &Path) -> Mount {
    mounts
        .iter()
        .rev()
        .find(|(m, _)| paths::contains(m, p))
        .map_or(Mount::Root, |(_, how)| *how)
}

/// The folders that hold a mask in `mounts`, or one of `gaps`, where the cell sees the computer itself, outer ones
/// first and none inside another: each goes through an overlay of its own (`Mount::Holder`), so its masks stay and
/// its gaps stay missing. Never `/`, whose overlay would stand in for the whole computer.
fn holders(mounts: &[(PathBuf, Mount)], gaps: &[PathBuf]) -> Vec<PathBuf> {
    let folders: Vec<&Path> = mounts
        .iter()
        .filter(|(_, how)| matches!(how, Mount::Secret | Mount::Secrets))
        .map(|(p, _)| p)
        .chain(gaps)
        .filter_map(|p| p.parent())
        .filter(|folder| *folder != Path::new("/") && over(mounts, folder) == Mount::Root)
        .collect();
    outermost(folders)
}

/// Gives each folder in `mounts` that holds one of `databases` where the cell sees the world an overlay of its own
/// whose writes are thrown away, as a folder a key opens throws them away already. Each goes right over the world,
/// under the masks inside it, outer ones first and none inside another, so that what an agent writes there never
/// reaches the world's changes, and through them the real database. What the world changed in it before does not show
/// there.
fn scratch(mounts: &mut Vec<(PathBuf, Mount)>, databases: &[PathBuf]) {
    let folders = databases
        .iter()
        .filter_map(|d| d.parent())
        .filter(|folder| over(mounts, folder) == Mount::World)
        .collect();
    let folders = outermost(folders);
    let above = mounts
        .iter()
        .rposition(|(_, how)| *how == Mount::World)
        .map_or(mounts.len(), |i| i + 1);
    mounts.splice(
        above..above,
        folders.into_iter().map(|p| (p, Mount::Folder)),
    );
}

/// Of `folders`, those inside none of the others, outer ones first.
fn outermost(mut folders: Vec<&Path>) -> Vec<PathBuf> {
    folders.sort_by_key(|folder| folder.components().count());
    let mut outer: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if !outer.iter().any(|o| paths::contains(o, folder)) {
            outer.push(folder.to_path_buf());
        }
    }
    outer
}

/// The overlays among `mounts` with one of `gaps` inside: the world's, those of the folders a key opens, and the
/// holders'. Each gets the layer of whiteouts on top of what it shows.
fn layered(mounts: &[(PathBuf, Mount)], gaps: &[PathBuf]) -> Vec<PathBuf> {
    mounts
        .iter()
        .filter(|(p, how)| {
            matches!(how, Mount::World | Mount::Folder | Mount::Holder)
                && gaps.iter().any(|g| g != p && paths::contains(p, g))
        })
        .map(|(p, _)| p.clone())
        .collect()
}

/// A layer of overlayfs whiteouts, one at each of a cell's gaps, laid out like the computer's own folders. On top of
/// an overlay's other layers, it keeps each gap there missing, whatever a program outside makes there while the cell
/// runs, as the cell found it when it started. It is not a mount, so the cell writes there as anywhere else: in a
/// world, what it writes is among the world's changes, and the layer never is. Removed when dropped, after the cell.
struct Layer {
    dir: PathBuf,
    /// The overlays it goes on (see `layered`).
    over: Vec<PathBuf>,
}

impl Layer {
    /// The layer for `view`'s gaps, where an overlay would show one, in a new folder of the session's runtime folder
    /// (`XDG_RUNTIME_DIR`), else of `/dev/shm` or the temp folder: a tmpfs, which no key may open and no cell sees.
    /// overlayfs refuses a layer that lies inside another of the same overlay, so it is never inside one it goes on.
    fn new(view: &View) -> Result<Option<Layer>> {
        let over = layered(&view.mounts, &view.gaps);
        if over.is_empty() {
            return Ok(None);
        }
        let place = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .into_iter()
            .chain([PathBuf::from("/dev/shm"), env::temp_dir()])
            .filter_map(|d| fs::canonicalize(d).ok())
            .find(|d| d.is_dir() && !over.iter().any(|o| paths::contains(o, d)))
            .ok_or_else(|| {
                Error::new(
                    "cannot keep missing password files missing: XDG_RUNTIME_DIR, /dev/shm and the temp folder \
                     are all missing or inside what the command sees",
                )
            })?;
        let dir = sys::make_temp_dir(&place.join("arbor-layer-"))
            .ctx(|| format!("cannot make a folder in {}", place.display()))?;
        let layer = Layer { dir, over };
        layer
            .fill(&view.gaps)
            .ctx(|| format!("cannot write {}", layer.dir.display()))?;
        Ok(Some(layer))
    }

    /// Where the layer has `p`, an absolute path.
    fn at(&self, p: &Path) -> PathBuf {
        self.dir.join(p.strip_prefix("/").unwrap_or(p))
    }

    /// The layer's folder for the overlay at `p`, where it goes on that one.
    fn on(&self, p: &Path) -> Option<PathBuf> {
        self.over.iter().any(|o| o == p).then(|| self.at(p))
    }

    /// A whiteout at each of `gaps`, and the folders on the way, each with its mode and times as the computer has it:
    /// a cell sees a folder that several layers have with those of the top one. Their times are set once they are
    /// full, since what is made in a folder changes them. Their owner keeps every permission, so the layer can be
    /// removed.
    fn fill(&self, gaps: &[PathBuf]) -> io::Result<()> {
        let (mut made, mut known): (Vec<&Path>, HashSet<&Path>) = Default::default();
        for gap in gaps {
            let mut way: Vec<&Path> = gap
                .ancestors()
                .skip(1)
                .filter(|a| a.parent().is_some())
                .collect();
            way.reverse();
            for folder in way {
                if known.insert(folder) {
                    fs::create_dir(self.at(folder))?;
                    made.push(folder);
                }
            }
            sys::make_whiteout(&self.at(gap))?;
        }
        for folder in made {
            let Ok(meta) = fs::metadata(folder) else {
                continue;
            };
            let copy = self.at(folder);
            let mode = meta.mode() & 0o7777 | 0o700;
            fs::set_permissions(&copy, fs::Permissions::from_mode(mode))?;
            let times = FileTimes::new()
                .set_accessed(meta.accessed()?)
                .set_modified(meta.modified()?);
            File::open(&copy)?.set_times(times)?;
        }
        Ok(())
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The records agents keep in a project's folder (`key::PROJECT_RECORDS`) that a cell in `world` would find, where
/// the agents would have written them: at the top of the world's folder and of each folder in `shown` that the key
/// opens, in each folder right inside one of those, and in `cwd`, where the command starts, and in each folder above
/// it up to the top of the world. Deeper ones are not looked for: a walk of a project's tree would take longer than a
/// cell takes to start. In the world, only those its base holds count, as it holds them: what a command in the world
/// wrote is the world's own, as a key's home folder keeps what an agent writes there.
fn project_records(world: &World, shown: &[(PathBuf, Mount)], cwd: &Path) -> Vec<PathBuf> {
    let tops = shown
        .iter()
        .filter(|(_, how)| matches!(how, Mount::World | Mount::Folder))
        .map(|(p, _)| (p.as_path(), true));
    let above = cwd
        .ancestors()
        .take_while(|dir| paths::contains(&world.base, dir))
        .map(|dir| (dir, false));
    let mut found: Vec<PathBuf> = tops
        .chain(above)
        .flat_map(|(dir, inside)| key::in_project(dir, inside))
        .filter(|p| {
            !paths::contains(&world.base, p) || world_file(world, p) == Some(Some(p.clone()))
        })
        .collect();
    // Outer ones first, so one inside another is covered by it.
    found.sort();
    found.dedup();
    found
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
    list_running(true)
}

/// The running cells, oldest first, for a reader such as Mission Control: files left by a crash are skipped and
/// left for `running` to remove, so reading changes nothing.
pub fn running_seen() -> Result<Vec<Running>> {
    list_running(false)
}

fn list_running(clean: bool) -> Result<Vec<Running>> {
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
            Ok(()) if !clean => {}
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
    /// What went through its proxy, under a key that opens hosts.
    pub net: Option<net::Tally>,
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
/// left behind, death tied to arbor's, and the descriptors in `inherit` left open for it. Returns its process ID
/// and a descriptor that becomes readable when it exits.
fn spawn(
    program: &CStr,
    argv: &[*const libc::c_char],
    envp: &[*const libc::c_char],
    group: Option<&Group>,
    inherit: &[BorrowedFd],
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
            // bwrap reads the seccomp filter from one of these, and the cell's first process hands its proxy port
            // over another, so they stay open across exec.
            for fd in inherit {
                if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, 0) } != 0 {
                    fail(errno());
                }
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
    /// Its key ended, at this time, in nanoseconds since 1970.
    KeyExpired(u64),
    /// Someone asked: `arbor stop`, a closed terminal or a signal to arbor.
    Stopped,
    KeyTakenBack,
}

/// Waits for the cell's first process. At its time limit or when its key ends, whichever comes first, or when arbor
/// is asked to stop (SIGTERM, SIGHUP), it kills the whole cell and says why. The key's end is read again when it
/// comes, since a renewal may have moved it later meanwhile.
fn wait(
    pid: libc::pid_t,
    exited: &OwnedFd,
    time_limit: Option<Instant>,
    key: Option<&Key>,
    group: Option<&Group>,
    stops: &sys::Interrupts,
) -> io::Result<(ExitStatus, Option<Ended>)> {
    let at = |end_ns: u64| {
        let left = end_ns.saturating_sub(world::now_ns());
        (Instant::now() + Duration::from_nanos(left), end_ns)
    };
    let mut key_end = key.map(|k| at(k.until_ns));
    let ended = loop {
        if stops.interrupted() {
            break Some(Ended::Stopped);
        }
        let deadline = [
            time_limit.map(|t| (t, Ended::TimeLimit)),
            key_end.map(|(t, end)| (t, Ended::KeyExpired(end))),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|(t, _)| *t);
        let left = deadline.map(|(t, why)| (t.saturating_duration_since(Instant::now()), why));
        if let Some((left, why)) = left
            && left.is_zero()
        {
            if let (Ended::KeyExpired(was), Some(k)) = (why, key) {
                let end = k.ends_at();
                if end > was {
                    key_end = Some(at(end));
                    continue;
                }
            }
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
        Some(Ended::KeyExpired(end)) => {
            let at = key::clock(end, world::now_ns());
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
    let cwd = start_dir(world);
    let view = View::new(world, key, &cwd)?;
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
    // A key taken back after the caller checked it, but before the cell was listed, is caught here. From here on a
    // take-back stops this cell before it removes the key's home, so a home that is gone can be made again, empty.
    if let Some(k) = key {
        if !key::exists(&k.name) {
            return Err(Error::refused(format!("key '{}' was taken back", k.name)));
        }
        k.ensure_home()?;
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
    // Under a key that opens hosts, the cell's first process hands its proxy port out over this pair (see `net`).
    let hosts = key.map(|k| k.net.clone()).unwrap_or_default();
    let pair = if hosts.is_empty() {
        None
    } else {
        Some(UnixStream::pair()?)
    };
    // The layer that keeps missing password files and records missing, kept until the cell has ended.
    let layer = Layer::new(&view)?;
    let args = view.args(
        world,
        cmd,
        &cwd,
        &path,
        filter.as_raw_fd(),
        socket.path(),
        pair.as_ref().map(|(_, inside)| inside.as_raw_fd()),
        layer.as_ref(),
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
    let time_limit = limits.time.map(|t| Instant::now() + t);
    let mut inherit = vec![filter.as_fd()];
    if let Some((_, inside)) = &pair {
        inherit.push(inside.as_fd());
    }
    let (pid, exited) = spawn(
        &program,
        &pointers(&argv),
        &pointers(&envp),
        group.as_ref(),
        &inherit,
    )
    .ctx(|| "cannot start the cell".to_string())?;
    drop(inherit);
    drop(filter);
    started(&id);
    // The cell holds its end of the pair now. Arbor's copy closes, so that a cell that ends before handing its
    // port over closes the pair, and arbor stops waiting for the port.
    let proxy = pair.and_then(|(outside, inside)| {
        drop(inside);
        let who = net::Who {
            world: world.name.clone(),
            cell: id.clone(),
            key: key.map(|k| k.name.clone()).unwrap_or_default(),
        };
        match net::Proxy::start(outside, hosts, who) {
            Ok(proxy) => Some(proxy),
            Err(e) => {
                eprintln!("arbor: warning: this command has no network: {e}");
                None
            }
        }
    });
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
    let waited = wait(pid, &exited, time_limit, key, group.as_ref(), &stops);
    // The adopted init dies with bwrap (--die-with-parent) and takes the cell's last processes with it.
    while unsafe { libc::waitpid(-1, std::ptr::null_mut(), 0) } > 0
        || io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
    {}
    unsafe {
        libc::signal(libc::SIGINT, old_int);
        libc::signal(libc::SIGQUIT, old_quit);
    }
    // Every process of the cell has ended, so its requests and connections have too.
    drop(server);
    let net = proxy.map(net::Proxy::finish);
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
        net,
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
            (Ended::KeyExpired(0), "killed: key expired"),
            (Ended::Stopped, "stopped"),
            (Ended::KeyTakenBack, "stopped: key taken back"),
        ] {
            assert_eq!(outcome(137, Some(ended), None, &limits, None).0, out);
        }
        // A key ends when it last said it would, which a renewal may have moved since the command started.
        let end = world::now_ns() + 3600 * 1_000_000_000;
        let (_, warning) = outcome(137, Some(Ended::KeyExpired(end)), None, &limits, None);
        assert!(
            warning
                .unwrap()
                .contains(&format!("ended at {},", key::clock(end, world::now_ns()))),
        );
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
            key_home: None,
            gaps: Vec::new(),
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
    fn a_mask_where_the_cell_sees_the_computer_gets_its_folder_through_an_overlay() {
        let p = PathBuf::from;
        let mounts = vec![
            (p("/"), Mount::Root),
            (p("/tmp"), Mount::Empty),
            (p("/home"), Mount::Empty),
            (p("/home/u"), Mount::Dir),
            (p("/home/u/shop"), Mount::World),
            (p("/home/u/.claude"), Mount::Folder),
            // In what a world or a key shows, a mask sits on an overlay already, and in a folder a cell gets fresh
            // there is nothing of the computer's to replace.
            (p("/home/u/shop/.git-credentials"), Mount::Secret),
            (p("/home/u/.claude/.credentials.json"), Mount::Secret),
            (p("/tmp/dots/aws"), Mount::Secrets),
            // Linked from the home folder to elsewhere on the computer: two in one folder, one in a folder inside it,
            // a folder of them, and one right in `/`.
            (p("/srv/dots/claude.json"), Mount::Secret),
            (p("/srv/dots/netrc"), Mount::Secret),
            (p("/srv/dots/more/npmrc"), Mount::Secret),
            (p("/data/gnupg"), Mount::Secrets),
            (p("/netrc"), Mount::Secret),
        ];
        assert_eq!(holders(&mounts, &[]), [p("/data"), p("/srv/dots")]);
        assert!(holders(&mounts[..9], &[]).is_empty());
    }

    #[test]
    fn a_missing_one_stays_missing_in_each_overlay_that_would_show_it() {
        let p = PathBuf::from;
        let mut mounts = vec![
            (p("/"), Mount::Root),
            (p("/tmp"), Mount::Empty),
            (p("/home"), Mount::Empty),
            (p("/home/u"), Mount::Dir),
            (p("/home/u/shop"), Mount::World),
            (p("/home/u/.claude"), Mount::Folder),
            (p("/home/u/.config"), Mount::Folder),
        ];
        // In the world, in a folder the key opens, inside another one there, and linked from the home folder to
        // elsewhere on the computer, in a folder that gets an overlay of its own, or right in `/`, which cannot.
        let gaps = [
            p("/home/u/shop/.env"),
            p("/home/u/.claude/history.jsonl"),
            p("/home/u/.config/gh"),
            p("/srv/dots/claude.json"),
            p("/srv/dots/more/netrc"),
            p("/netrc"),
        ];
        assert_eq!(holders(&mounts, &gaps), [p("/srv/dots")]);
        mounts.insert(2, (p("/srv/dots"), Mount::Holder));
        assert_eq!(
            layered(&mounts, &gaps),
            [
                p("/srv/dots"),
                p("/home/u/shop"),
                p("/home/u/.claude"),
                p("/home/u/.config")
            ]
        );
        // An overlay with no gap inside gets no layer, nor does a folder the cell gets fresh.
        assert_eq!(layered(&mounts, &gaps[1..2]), [p("/home/u/.claude")]);
        assert!(layered(&mounts, &[p("/tmp/x"), p("/home/u/notes")]).is_empty());
    }

    #[test]
    fn an_agent_s_database_in_a_world_gets_its_folder_through_an_overlay_whose_writes_vanish() {
        let p = PathBuf::from;
        let mut mounts = vec![
            (p("/"), Mount::Root),
            (p("/tmp"), Mount::Empty),
            (p("/home"), Mount::Empty),
            (p("/home/u"), Mount::World),
            (p("/data/agents"), Mount::Folder),
            (p("/home/u/.codex/sessions"), Mount::Secrets),
            (p("/home/u/.codex/auth.json"), Mount::Secret),
            (p("/home/u/.state"), Mount::Empty),
        ];
        let before = mounts.clone();
        // In the world, two in one folder and one in a folder inside it, and another agent's; in a folder the key
        // opens, whose writes vanish already; and linked out of the home folder, where the cell sees the computer's.
        let databases = [
            p("/home/u/.codex/state_5.sqlite"),
            p("/home/u/.codex/state_5.sqlite-wal"),
            p("/home/u/.codex/old/logs_1.sqlite"),
            p("/home/u/.copilot/session-store.db"),
            p("/data/agents/codex/logs_2.sqlite"),
            p("/srv/dots/session-store.db"),
        ];
        scratch(&mut mounts, &databases);
        assert_eq!(
            mounts[4..6],
            [
                (p("/home/u/.codex"), Mount::Folder),
                (p("/home/u/.copilot"), Mount::Folder)
            ]
        );
        assert_eq!(mounts[6..], before[4..]);
        // The masks inside them stay on top, and each overlay gets the layer that keeps the databases missing.
        let seen = |path: &str| over(&mounts, Path::new(path));
        assert_eq!(seen("/home/u/.codex/auth.json"), Mount::Secret);
        assert_eq!(seen("/home/u/.codex/old/logs_1.sqlite"), Mount::Folder);
        assert_eq!(
            layered(&mounts, &databases),
            [
                p("/home/u"),
                p("/home/u/.codex"),
                p("/home/u/.copilot"),
                p("/data/agents")
            ]
        );
        // With no database, nothing changes.
        let mut none = before.clone();
        scratch(&mut none, &[]);
        assert_eq!(none, before);
    }

    #[test]
    fn a_layer_has_a_whiteout_at_each_gap_in_folders_like_the_computer_s_own() {
        use std::os::unix::fs::FileTypeExt;
        let root = env::temp_dir().join(format!("arbor-unit-layer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/.config")).unwrap();
        let root = fs::canonicalize(&root).unwrap();
        let (home, config) = (root.join("home"), root.join("home/.config"));
        fs::set_permissions(&config, fs::Permissions::from_mode(0o750)).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        File::open(&config)
            .unwrap()
            .set_times(FileTimes::new().set_accessed(old).set_modified(old))
            .unwrap();
        let gaps = [
            home.join(".config/gh"),
            home.join(".netrc"),
            home.join(".ssh"),
        ];
        let dir = sys::make_temp_dir(&env::temp_dir().join("arbor-unit-layer-")).unwrap();
        let layer = Layer {
            dir: dir.clone(),
            over: vec![home.clone()],
        };
        layer.fill(&gaps).unwrap();

        for gap in &gaps {
            let meta = fs::symlink_metadata(layer.at(gap)).unwrap();
            assert!(
                meta.file_type().is_char_device() && meta.rdev() == 0,
                "{}",
                gap.display()
            );
        }
        // A cell sees the folder on the way as the computer has it, not as the layer made it.
        let copy = fs::metadata(layer.at(&config)).unwrap();
        assert_eq!(copy.mode() & 0o7777, 0o750);
        assert_eq!(copy.modified().unwrap(), old);
        assert_eq!(fs::read_dir(layer.at(&home)).unwrap().count(), 3);
        // It goes only on the overlays it was made for, and its folder goes with it.
        assert_eq!(layer.on(&home), Some(layer.at(&home)));
        assert_eq!(layer.on(&config), None);
        drop(layer);
        assert!(!dir.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_records_in_a_project_where_agents_wrote_them_but_not_the_world_s_own() {
        let root = env::temp_dir().join(format!("arbor-unit-projects-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in [
            "base/a/b/c",
            "base/sub",
            "base/gone",
            "world/up/sub",
            "world/up/gone",
            "world/work",
            "other/shop",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let root = fs::canonicalize(&root).unwrap();
        let (base, other) = (root.join("base"), root.join("other"));
        for file in [
            "base/.aider.chat.history.md",
            "base/a/.aider.input.history",
            "base/a/b/.aider.llm.history",
            "base/sub/.aider.chat.history.md",
            "base/sub/.aider.input.history",
            "base/gone/.aider.chat.history.md",
            "other/.aider.chat.history.md",
            "other/shop/.aider.input.history",
            // The world's own version of one of the base's
            "world/up/sub/.aider.input.history",
        ] {
            fs::write(root.join(file), "").unwrap();
        }
        // And one of the base's that the world deleted.
        sys::make_whiteout(&root.join("world/up/gone/.aider.chat.history.md")).unwrap();
        let world = World {
            name: "shop".into(),
            base: base.clone(),
            since_ns: 0,
            state: State::Open,
            kept_ns: None,
            dir: root.join("world"),
        };
        let shown = [(base.clone(), Mount::World), (other.clone(), Mount::Folder)];
        let mut found = vec![
            base.join(".aider.chat.history.md"),
            base.join("a/.aider.input.history"),
            base.join("sub/.aider.chat.history.md"),
            other.join(".aider.chat.history.md"),
            other.join("shop/.aider.input.history"),
        ];

        // At the top of the world and of what the key opens, and one level down...
        assert_eq!(project_records(&world, &shown, &base), found);
        // ...and in each folder above the one a command starts in.
        found.insert(2, base.join("a/b/.aider.llm.history"));
        assert_eq!(project_records(&world, &shown, &base.join("a/b/c")), found);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_key_s_home_stands_in_the_home_folder_under_the_world_and_what_the_key_opens() {
        let root = env::temp_dir().join(format!("arbor-unit-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (home, key_home) = (root.join("u"), root.join("state/homes/agent"));
        let world = World {
            name: "shop".into(),
            base: home.join("shop"),
            since_ns: 0,
            state: State::Open,
            kept_ns: None,
            dir: root.join("state/worlds/shop"),
        };
        let script = |path: &Path| {
            use std::os::unix::fs::PermissionsExt;
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "#!/bin/sh\n").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        // The agent installed one tool in its home; the other is only in the real home folder.
        script(&key_home.join("bin/mine"));
        script(&home.join("bin/yours"));
        fs::create_dir_all(key_home.join(".local")).unwrap();
        std::os::unix::fs::symlink("/elsewhere", key_home.join(".local/bin")).unwrap();
        fs::create_dir_all(world.up()).unwrap();
        fs::create_dir_all(&world.base).unwrap();
        let view = View {
            mounts: vec![
                ("/".into(), Mount::Root),
                (root.clone(), Mount::Empty),
                (home.clone(), Mount::Home),
                (world.base.clone(), Mount::World),
                (home.join(".cargo"), Mount::Folder),
            ],
            home: Some(home.clone()),
            key_home: Some(key_home.clone()),
            gaps: Vec::new(),
        };
        for (path, seen) in [
            (home.join("notes"), Mount::Home),
            (home.join("shop/src"), Mount::World),
            (home.join(".cargo/bin/cargo"), Mount::Folder),
        ] {
            assert_eq!(view.over(&path), seen, "{}", path.display());
        }
        assert_eq!(
            view.at(&world, &home.join("bin/mine"), 0),
            At::Runs(key_home.join("bin/mine"))
        );
        assert_eq!(
            view.at(&world, &home.join("bin/yours"), 0),
            At::Hidden(vec![home.join("bin/yours")])
        );
        assert_eq!(view.at(&world, &home.join("bin/none"), 0), At::Absent);
        assert_eq!(view.at(&world, &home.join(".local/bin/x"), 0), At::Unsure);
        fs::remove_dir_all(&root).unwrap();
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
            net: Vec::new(),
            home: None,
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
