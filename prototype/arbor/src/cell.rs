//! Running a command inside a world, through bubblewrap, without root.
//!
//! Ubuntu 24.04 and later block plain user namespaces for ordinary programs, but allow `bwrap`. So a cell is a
//! `bwrap` process: the computer read-only, the world's overlay mounted over its base folder's own path so tools
//! see the usual paths, a throwaway layer over the rest of the home folder (writes there vanish at exit), `/run`
//! and Arbor's own state hidden, no network, no new user namespaces and an empty session keyring. Where the
//! user's systemd manager allows it, each cell also runs in its own cgroup (see `cgroup`), with its limits, and
//! its whole cost is read from there. Sealing the home folder and keys come next in M3; network through a filter
//! after that.

use std::env;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::cgroup::{self, Group, Limits, Usage};
use crate::error::{Context, Error, Result};
use crate::world::{self, Lock, State, World};
use crate::{paths, sys};

pub struct Options {
    /// The command will talk to the person at the terminal.
    pub interactive: bool,
    pub limits: Limits,
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

/// The `bwrap` arguments for a cell in `world` that runs `cmd` in `cwd`.
pub fn bwrap_args(
    world: &World,
    cmd: &[OsString],
    cwd: &Path,
    opts: &Options,
) -> Result<Vec<OsString>> {
    let mut a: Vec<OsString> = Vec::new();
    let mut push = |items: &[&dyn AsRef<std::ffi::OsStr>]| {
        a.extend(items.iter().map(|s| s.as_ref().to_os_string()));
    };
    push(&[
        &"--ro-bind",
        &"/",
        &"/",
        &"--dev",
        &"/dev",
        &"--proc",
        &"/proc",
        &"--tmpfs",
        &"/tmp",
        // /run holds the computer's and the session's sockets: Docker (which is root), D-Bus, systemd, snapd, the
        // session bus, Wayland and agents. Each is a way out of the cell.
        &"--tmpfs",
        &"/run",
    ]);
    // Writes anywhere else in the home folder land in a layer that is thrown away when the cell ends.
    if let Some(home) = paths::home().and_then(|h| fs::canonicalize(h).ok())
        && home.is_dir()
        && !paths::contains(&world.base, &home)
    {
        push(&[&"--overlay-src", &home, &"--tmp-overlay", &home]);
    }
    push(&[
        &"--overlay-src",
        &world.base,
        &"--overlay",
        &world.up(),
        &world.work(),
        &world.base,
    ]);
    // What the cell must not see: every world's changes, and the session's sockets if they live outside /run.
    let state = fs::canonicalize(paths::state_dir()?)?;
    push(&[&"--tmpfs", &state]);
    if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR")
        .and_then(|r| fs::canonicalize(r).ok())
        .filter(|r| r.is_dir() && !r.starts_with("/run"))
    {
        push(&[&"--tmpfs", &runtime]);
    }
    // No network, and no user namespaces of its own: those would reopen what bwrap closed.
    push(&[
        &"--unshare-all",
        &"--unshare-user",
        &"--disable-userns",
        &"--die-with-parent",
    ]);
    if !opts.interactive || tiocsti_possible() {
        push(&[&"--new-session"]);
    }
    push(&[
        &"--chdir",
        &cwd,
        &"--setenv",
        &"ARBOR_WORLD",
        &world.name.as_str(),
        &"--",
    ]);
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

/// How a command in a cell ended.
pub struct Ran {
    /// The cell's name, as `arbor ps` and the ledger show it.
    pub id: String,
    /// Its exit code, or 128 + the signal that killed it.
    pub code: i32,
    /// How it ended, for the ledger: `exit N`, `killed: time limit` or `killed: memory limit`.
    pub out: String,
    /// A line for the person when a limit stopped it.
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

/// Waits for the cell's first process. At the deadline it kills the whole cell and says so.
fn wait(
    pid: libc::pid_t,
    exited: &OwnedFd,
    deadline: Option<Instant>,
    group: Option<&Group>,
) -> io::Result<(ExitStatus, bool)> {
    let mut timed_out = false;
    while let Some(deadline) = deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            // Not reaped yet, so its process ID still names it.
            match group {
                Some(g) => g.kill(),
                None => unsafe {
                    libc::kill(pid, libc::SIGKILL);
                },
            }
            timed_out = true;
            break;
        }
        let mut pfd = libc::pollfd {
            fd: exited.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = left.as_millis().clamp(1, i32::MAX as u128) as i32;
        match unsafe { libc::poll(&mut pfd, 1, ms) } {
            n if n > 0 => break,
            0 => {}
            _ => {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::Interrupted {
                    return Err(e);
                }
            }
        }
    }
    let mut status = 0;
    while unsafe { libc::waitpid(pid, &mut status, 0) } < 0 {
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
    Ok((ExitStatus::from_raw(status), timed_out))
}

/// What the ledger says about how a cell ended, and a line for the person when a limit stopped it.
fn outcome(
    code: i32,
    timed_out: bool,
    usage: Option<&Usage>,
    limits: &Limits,
) -> (String, Option<String>) {
    let oom = usage.is_some_and(|u| u.oom_kills > 0);
    if timed_out {
        let limit = limits.time.map(cgroup::duration).unwrap_or_default();
        return (
            "killed: time limit".into(),
            Some(format!(
                "the command reached its time limit ({limit}) and was stopped"
            )),
        );
    }
    if oom && let Some(mem) = limits.mem {
        return (
            "killed: memory limit".into(),
            Some(format!(
                "the command reached its memory limit ({}) and was stopped",
                cgroup::size(mem)
            )),
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

/// Runs `cmd` in the world. The caller holds the world's lock and loaded the world after taking it; `started` is
/// told the cell's name once it runs.
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
    let group = cell_group(limits)?;
    let id = group
        .as_ref()
        .map_or_else(cgroup::new_id, |g| g.id().to_string());
    let _entry = register(&Running {
        id: id.clone(),
        world: world.name.clone(),
        what: shown(cmd),
        limits: limits.describe(),
        started_ns: world::now_ns(),
        cgroup: group.as_ref().map(|g| g.dir().to_path_buf()),
    })?;
    // Everything exec needs is made before the fork: the child may not allocate.
    let program = c_string(bwrap.as_os_str())?;
    let argv = std::iter::once(OsStr::new("bwrap"))
        .chain(args.iter().map(OsString::as_os_str))
        .map(c_string)
        .collect::<Result<Vec<_>>>()?;
    let envp = env::vars_os()
        .filter(|(key, _)| key != cgroup::SCOPE_MARK)
        .map(|(mut key, value)| {
            key.push("=");
            key.push(value);
            c_string(&key)
        })
        .collect::<Result<Vec<_>>>()?;
    // bwrap's outer process exits as soon as the command does, without waiting for the cell's init, and the
    // cell's CPU time and memory would go with it. As a subreaper, arbor adopts that init and waits for it below,
    // so they count.
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) };
    let (cpu_before, _) = children_usage();
    let deadline = limits.time.map(|t| Instant::now() + t);
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
    let waited = wait(pid, &exited, deadline, group.as_ref());
    // The adopted init dies with bwrap (--die-with-parent) and takes the cell's last processes with it.
    while unsafe { libc::waitpid(-1, std::ptr::null_mut(), 0) } > 0
        || io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
    {}
    unsafe {
        libc::signal(libc::SIGINT, old_int);
        libc::signal(libc::SIGQUIT, old_quit);
    }
    let (status, timed_out) = waited?;
    let code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
    let timed_out = timed_out && status.signal() == Some(libc::SIGKILL);
    let (cpu, max_rss) = children_usage();
    let usage = group.as_ref().map(Group::usage);
    let (out, warning) = outcome(code, timed_out, usage.as_ref(), limits);
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
            outcome(0, false, Some(&used(0, 0)), &limits),
            ("exit 0".into(), None)
        );
        let (out, warning) = outcome(137, true, None, &limits);
        assert_eq!(out, "killed: time limit");
        assert!(warning.unwrap().contains("(2s)"));
        assert_eq!(
            outcome(137, false, Some(&used(1, 0)), &limits).0,
            "killed: memory limit"
        );
        assert_eq!(
            outcome(2, false, Some(&used(0, 3)), &limits).0,
            "exit 2, process limit hit"
        );
        assert_eq!(
            outcome(137, false, Some(&used(1, 0)), &Limits::default()).0,
            "exit 137, out of memory"
        );
    }
}
