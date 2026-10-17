//! Running a command inside a world, through bubblewrap, without root.
//!
//! Ubuntu 24.04 and later block plain user namespaces for ordinary programs, but allow `bwrap`. So a cell is a
//! `bwrap` process, sealed: the computer read-only; the home folders, `/media` and `/mnt` empty; the world's overlay
//! mounted over its base folder's own path, so tools see the usual paths; `/run` and Arbor's own state hidden; a
//! cleared environment; no network, no new user namespaces and an empty session keyring. A key (see `key`) opens
//! more folders and files and passes variables in; password files stay hidden even then. Where the
//! user's systemd manager allows it, each cell also runs in its own cgroup (see `cgroup`), with its limits, and its
//! whole cost is read from there. Network through a filter comes later.

use std::env;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cgroup::{self, Group, Limits, Usage};
use crate::error::{Context, Error, Result};
use crate::key::{self, Key};
use crate::world::{self, Lock, State, World};
use crate::{paths, sys};

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

/// TIOCSTI lets a process type into its terminal. Linux 6.2+ can switch it off for everyone; where it is still
/// on, the cell gets a new session so it cannot type into the person's terminal.
fn tiocsti_possible() -> bool {
    fs::read_to_string("/proc/sys/dev/tty/legacy_tiocsti")
        .map(|v| v.trim() != "0")
        .unwrap_or(true)
}

/// How a path shows through into a sealed cell.
#[derive(Clone, Copy)]
enum Shown {
    /// The world, over its base folder's path.
    World,
    /// A folder a key opens. Writes to it are thrown away, so tools that keep caches there still work.
    Folder,
    /// A file a key opens, read-only.
    File,
}

fn add(a: &mut Vec<OsString>, items: &[&dyn AsRef<OsStr>]) {
    a.extend(items.iter().map(|s| s.as_ref().to_os_string()));
}

/// The `bwrap` arguments for a cell in `world` that runs `cmd` in `cwd`. Mounts land in order, each on top of the
/// ones before it.
pub fn bwrap_args(
    world: &World,
    cmd: &[OsString],
    cwd: &Path,
    opts: &Options,
) -> Result<Vec<OsString>> {
    let mut a: Vec<OsString> = Vec::new();
    add(
        &mut a,
        &[
            &"--ro-bind",
            &"/",
            &"/",
            &"--dev",
            &"/dev",
            &"--proc",
            &"/proc",
            &"--tmpfs",
            &"/tmp",
            // /run holds the computer's and the session's sockets: Docker (which is root), D-Bus, systemd, snapd,
            // the session bus, Wayland and agents. Each is a way out of the cell.
            &"--tmpfs",
            &"/run",
        ],
    );
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
            add(&mut a, &[&"--tmpfs", dir]);
        }
    }
    if let Some(home) = &home
        && hidden.iter().any(|h| paths::contains(h, home))
    {
        add(&mut a, &[&"--dir", home]);
    }
    // What shows through, outer paths first so inner ones land on top.
    let mut shown = vec![(world.base.clone(), Shown::World)];
    for p in opts.key.iter().flat_map(|k| &k.read) {
        if let Ok(meta) = fs::metadata(p) {
            let how = if meta.is_dir() {
                Shown::Folder
            } else {
                Shown::File
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
    let masked = |p: &Path| masks.iter().any(|(m, _)| paths::contains(m, p));
    let mount = |a: &mut Vec<OsString>, p: &Path, how: Shown| match how {
        Shown::World => add(
            a,
            &[
                &"--overlay-src",
                &world.base,
                &"--overlay",
                &world.up(),
                &world.work(),
                &world.base,
            ],
        ),
        Shown::Folder => add(a, &[&"--overlay-src", &p, &"--tmp-overlay", &p]),
        Shown::File => add(a, &[&"--ro-bind", &p, &p]),
    };
    for (p, how) in &shown {
        // What lies inside the world is in it already.
        let in_world = !matches!(how, Shown::World) && paths::contains(&world.base, p);
        if !masked(p) && !in_world {
            mount(&mut a, p, *how);
        }
    }
    for (m, dir) in &masks {
        if *dir {
            add(&mut a, &[&"--tmpfs", m]);
        } else {
            // A plain bind would be `nodev`, and reading the null device there fails; this one reads empty.
            add(&mut a, &[&"--dev-bind", &"/dev/null", m]);
        }
    }
    for (p, how) in &shown {
        if masked(p) {
            mount(&mut a, p, *how);
        }
    }
    // What the cell must not see: every world's changes, and the session's sockets if they live outside /run.
    let state = fs::canonicalize(paths::state_dir()?)?;
    add(&mut a, &[&"--tmpfs", &state]);
    if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR")
        .and_then(|r| fs::canonicalize(r).ok())
        .filter(|r| r.is_dir() && !r.starts_with("/run"))
    {
        add(&mut a, &[&"--tmpfs", &runtime]);
    }
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
    if !opts.interactive || tiocsti_possible() {
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
            &"--",
        ],
    );
    a.extend(cmd.iter().cloned());
    Ok(a)
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

/// A command as a shell would take it back: arguments with spaces or quotes are quoted.
pub fn shown(cmd: &[OsString]) -> String {
    let word = |a: &OsString| {
        let a = a.to_string_lossy();
        let plain = !a.is_empty()
            && a.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
        if plain {
            a.into_owned()
        } else {
            format!("'{}'", a.replace('\'', "'\\''"))
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

/// What this process's finished children have used so far: CPU milliseconds and the largest peak memory in KiB.
fn children_usage() -> (u64, u64) {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut ru) };
    let ms = |t: libc::timeval| t.tv_sec as u64 * 1000 + t.tv_usec as u64 / 1000;
    (ms(ru.ru_utime) + ms(ru.ru_stime), ru.ru_maxrss as u64)
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
/// left behind and death tied to arbor's. Returns its process ID and a descriptor that becomes readable when it
/// exits.
fn spawn(
    program: &CStr,
    argv: &[*const libc::c_char],
    envp: &[*const libc::c_char],
    group: Option<&Group>,
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
    let args = bwrap_args(world, cmd, &start_dir(world), opts)?;
    let limits = &opts.limits;
    let key = opts.key.as_ref();
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
    let (cpu_before, _) = children_usage();
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
    let (pid, exited) = spawn(&program, &pointers(&argv), &pointers(&envp), group.as_ref())
        .ctx(|| "cannot start the cell".to_string())?;
    started(&id);
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
    let (cpu, max_rss) = children_usage();
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
    fn keeps_only_plain_variables_and_those_a_key_passes() {
        let mut key = Key {
            name: "k".into(),
            within: None,
            worlds: Vec::new(),
            read: Vec::new(),
            env: vec!["CARGO_HOME".into()],
            limits: Limits::default(),
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
