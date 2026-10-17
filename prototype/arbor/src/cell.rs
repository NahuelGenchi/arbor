//! Running a command inside a world, through bubblewrap, without root.
//!
//! Ubuntu 24.04 and later block plain user namespaces for ordinary programs, but allow `bwrap`. So a cell is a
//! `bwrap` process: the computer read-only, the world's overlay mounted over its base folder's own path so tools
//! see the usual paths, a throwaway layer over the rest of the home folder (writes there vanish at exit), `/run`
//! and Arbor's own state hidden, no network, no new user namespaces and an empty session keyring. Sealing the
//! home folder, keys and limits come in M3; network through a filter after that.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Context, Error, Result};
use crate::world::{Lock, State, World};
use crate::{paths, sys};

pub struct Options {
    /// The command will talk to the person at the terminal.
    pub interactive: bool,
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

/// How a command in a cell ended.
pub struct Ran {
    /// Its exit code, or 128 + the signal that killed it.
    pub code: i32,
    /// CPU time of all its processes.
    pub cpu_ms: u64,
    /// Peak memory of its largest process, in KiB.
    pub mem_kib: u64,
}

/// What this process's finished children have used so far: CPU milliseconds and the largest peak memory in KiB.
fn children_usage() -> (u64, u64) {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut ru) };
    let ms = |t: libc::timeval| t.tv_sec as u64 * 1000 + t.tv_usec as u64 / 1000;
    (ms(ru.ru_utime) + ms(ru.ru_stime), ru.ru_maxrss as u64)
}

/// Runs `cmd` in the world. The caller holds the world's lock and loaded the world after taking it.
pub fn run(world: &World, _lock: &Lock, cmd: &[OsString], opts: &Options) -> Result<Ran> {
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
    let mut command = Command::new(bwrap);
    command.args(&args);
    // The login session's keyring stays outside. Where keyctl is blocked (as in Docker's default profile), the
    // cell cannot reach a keyring either, so a failure is ignored.
    unsafe {
        command.pre_exec(|| {
            let _ = sys::new_session_keyring();
            Ok(())
        });
    }
    // bwrap's outer process exits as soon as the command does, without waiting for the cell's init, and the
    // cell's CPU time and memory would go with it. As a subreaper, arbor adopts that init and waits for it below,
    // so they count.
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) };
    let (cpu_before, _) = children_usage();
    let mut child = command.spawn().ctx(|| "cannot start bwrap".to_string())?;
    // Ctrl-C belongs to the command inside: Arbor waits for it instead of dying first and leaving it behind.
    let (old_int, old_quit) = unsafe {
        (
            libc::signal(libc::SIGINT, libc::SIG_IGN),
            libc::signal(libc::SIGQUIT, libc::SIG_IGN),
        )
    };
    let status = child.wait();
    // The adopted init dies with bwrap (--die-with-parent) and takes the cell's last processes with it.
    while unsafe { libc::waitpid(-1, std::ptr::null_mut(), 0) } > 0
        || io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
    {}
    unsafe {
        libc::signal(libc::SIGINT, old_int);
        libc::signal(libc::SIGQUIT, old_quit);
    }
    let status = status?;
    let (cpu, mem_kib) = children_usage();
    Ok(Ran {
        code: status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)),
        cpu_ms: cpu - cpu_before,
        mem_kib,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_bwrap_versions() {
        assert_eq!(super::bwrap_version("bubblewrap 0.11.1\n"), Some((0, 11)));
        assert_eq!(super::bwrap_version("bubblewrap 0.9.0"), Some((0, 9)));
        assert!(super::bwrap_version("bubblewrap 0.9.0") < Some(super::BWRAP_MIN));
        assert_eq!(super::bwrap_version("garbage"), None);
    }
}
