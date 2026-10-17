//! Thin wrappers for the few Linux calls the standard library lacks: locks, atomic renames, extended attributes,
//! whiteouts, the session keyring, process and folder watches, and deferred interrupts. Path calls never follow a
//! final symlink.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

fn cpath(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn check(ret: libc::c_int) -> io::Result<()> {
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Takes an advisory lock on `file`; it is released when the file is closed or the process exits.
/// With `wait` false, a lock held elsewhere returns `WouldBlock`.
pub fn flock(file: &File, exclusive: bool, wait: bool) -> io::Result<()> {
    let mut op = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    if !wait {
        op |= libc::LOCK_NB;
    }
    loop {
        let ret = unsafe { libc::flock(file.as_raw_fd(), op) };
        match check(ret) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

fn renameat2(from: &Path, to: &Path, flags: libc::c_uint) -> io::Result<()> {
    let (f, t) = (cpath(from)?, cpath(to)?);
    check(unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            f.as_ptr(),
            libc::AT_FDCWD,
            t.as_ptr(),
            flags,
        )
    })
}

/// Swaps two paths in one step: neither is ever missing. Both must exist on the same filesystem.
pub fn rename_exchange(a: &Path, b: &Path) -> io::Result<()> {
    renameat2(a, b, libc::RENAME_EXCHANGE)
}

/// Renames `from` to `to`, failing with `AlreadyExists` instead of replacing anything at `to`.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    renameat2(from, to, libc::RENAME_NOREPLACE)
}

/// Reads one extended attribute; `None` when the path does not have it.
pub fn get_xattr(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let (p, n) = (
        cpath(path)?,
        CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?,
    );
    let mut buf = vec![0u8; 256];
    loop {
        let len =
            unsafe { libc::lgetxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if len >= 0 {
            buf.truncate(len as usize);
            return Ok(Some(buf));
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::ENODATA) | Some(libc::ENOTSUP) => return Ok(None),
            Some(libc::ERANGE) => buf.resize(buf.len() * 4, 0),
            _ => return Err(err),
        }
    }
}

/// Lists the names of a path's extended attributes.
pub fn list_xattrs(path: &Path) -> io::Result<Vec<String>> {
    let p = cpath(path)?;
    let len = unsafe { libc::llistxattr(p.as_ptr(), std::ptr::null_mut(), 0) };
    if len < 0 {
        let err = io::Error::last_os_error();
        return if err.raw_os_error() == Some(libc::ENOTSUP) {
            Ok(Vec::new())
        } else {
            Err(err)
        };
    }
    let mut buf = vec![0u8; len as usize];
    let len = unsafe { libc::llistxattr(p.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if len < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(len as usize);
    Ok(buf
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect())
}

pub fn set_xattr(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
    let (p, n) = (
        cpath(path)?,
        CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?,
    );
    check(unsafe {
        libc::lsetxattr(
            p.as_ptr(),
            n.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    })
}

pub fn remove_xattr(path: &Path, name: &str) -> io::Result<()> {
    let (p, n) = (
        cpath(path)?,
        CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?,
    );
    check(unsafe { libc::lremovexattr(p.as_ptr(), n.as_ptr()) })
}

/// Creates an overlayfs whiteout: a character device numbered 0:0. Linux has allowed this without privileges
/// since 5.8, so a world can be restored to exactly what it held.
pub fn make_whiteout(path: &Path) -> io::Result<()> {
    let p = cpath(path)?;
    check(unsafe { libc::mknod(p.as_ptr(), libc::S_IFCHR, libc::makedev(0, 0)) })
}

/// True for an overlayfs whiteout: a character device numbered 0:0.
pub fn is_whiteout(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.file_type().is_char_device() && meta.rdev() == 0
}

/// Gives the calling process a new, empty session keyring, so a command started from it cannot use the keys of
/// the person's login session. Safe between fork and exec.
pub fn new_session_keyring() -> io::Result<()> {
    const KEYCTL_JOIN_SESSION_KEYRING: libc::c_long = 1;
    let ret = unsafe {
        libc::syscall(
            libc::SYS_keyctl,
            KEYCTL_JOIN_SESSION_KEYRING,
            std::ptr::null::<libc::c_char>(),
        )
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// A descriptor that becomes readable when the process `pid` exits, so a wait can have a deadline.
pub fn pidfd_open(pid: libc::pid_t) -> io::Result<OwnedFd> {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
    }
}

/// A pipe whose ends close on exec: (read, write).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    check(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// `clone3`'s arguments, as in linux/sched.h.
#[repr(C)]
#[derive(Default)]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

const CLONE_PIDFD: u64 = 0x1000;
const CLONE_INTO_CGROUP: u64 = 0x2_0000_0000;

/// Which side of a fork this is.
pub enum Forked {
    /// The child. `placed` is false when it still has to move itself into the cgroup.
    Child { placed: bool },
    /// The parent, with the child's process ID and a descriptor that becomes readable when it exits.
    Parent { pid: libc::pid_t, exited: OwnedFd },
}

/// Forks, the child starting inside the cgroup whose folder is open as `cgroup`. Moving a process into a cgroup
/// afterwards can wait for the kernel to quiesce cgroup changes everywhere (an RCU grace period), which made each
/// cell start several milliseconds later on a busy desktop; starting it there avoids that wait. Where `clone3` is
/// missing or filtered (Docker's default profile), a plain fork.
///
/// # Safety
/// As for `fork`: the child may only make async-signal-safe calls, then exec or `_exit`.
pub unsafe fn fork_into(cgroup: Option<BorrowedFd>) -> io::Result<Forked> {
    let mut pidfd: libc::c_int = -1;
    let mut args = CloneArgs {
        flags: CLONE_PIDFD
            | if cgroup.is_some() {
                CLONE_INTO_CGROUP
            } else {
                0
            },
        pidfd: &raw mut pidfd as u64,
        exit_signal: libc::SIGCHLD as u64,
        cgroup: cgroup.map_or(0, |fd| fd.as_raw_fd() as u64),
        ..CloneArgs::default()
    };
    let ret = unsafe {
        libc::syscall(
            libc::SYS_clone3,
            &raw mut args,
            std::mem::size_of::<CloneArgs>(),
        )
    };
    match ret {
        0 => return Ok(Forked::Child { placed: true }),
        pid if pid > 0 => {
            return Ok(Forked::Parent {
                pid: pid as libc::pid_t,
                exited: unsafe { OwnedFd::from_raw_fd(pidfd) },
            });
        }
        _ => {}
    }
    let err = io::Error::last_os_error();
    if !matches!(
        err.raw_os_error(),
        Some(libc::ENOSYS | libc::EPERM | libc::E2BIG)
    ) {
        return Err(err);
    }
    match unsafe { libc::fork() } {
        0 => Ok(Forked::Child {
            placed: cgroup.is_none(),
        }),
        -1 => Err(io::Error::last_os_error()),
        pid => match pidfd_open(pid) {
            Ok(exited) => Ok(Forked::Parent { pid, exited }),
            Err(e) => {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    libc::waitpid(pid, std::ptr::null_mut(), 0);
                }
                Err(e)
            }
        },
    }
}

/// Changes to the files directly inside one folder, through inotify: waiting costs nothing while nothing changes.
pub struct DirWatch {
    fd: OwnedFd,
}

impl DirWatch {
    pub fn new(dir: &Path) -> io::Result<DirWatch> {
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let p = cpath(dir)?;
        let mask = libc::IN_MODIFY | libc::IN_CREATE | libc::IN_MOVED_TO;
        if unsafe { libc::inotify_add_watch(fd.as_raw_fd(), p.as_ptr(), mask) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(DirWatch { fd })
    }

    /// Waits until something in the folder changes (true), or until `out`, the reader's end, goes away (false).
    pub fn wait(&self, out: BorrowedFd) -> io::Result<bool> {
        let mut fds = [
            libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: out.as_raw_fd(),
                events: 0,
                revents: 0,
            },
        ];
        loop {
            if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if fds[1].revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                return Ok(false);
            }
            if fds[0].revents & libc::POLLIN != 0 {
                // Which file changed does not matter: the caller looks again.
                let mut buf = [0u8; 4096];
                unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
                return Ok(true);
            }
        }
    }
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
const DEFERRED: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

extern "C" fn note_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// While held, Ctrl-C, a closed terminal or `kill` only raise a flag that `interrupted` reads, so keep and undo
/// stop between two steps with their journal in order, never halfway through one. Signals ignored before (as
/// under `nohup`) stay ignored.
pub struct Interrupts {
    old: [libc::sighandler_t; 3],
}

pub fn defer_interrupts() -> Interrupts {
    INTERRUPTED.store(false, Ordering::SeqCst);
    let handler = note_interrupt as extern "C" fn(libc::c_int) as libc::sighandler_t;
    let old = DEFERRED.map(|sig| unsafe {
        let old = libc::signal(sig, handler);
        if old == libc::SIG_IGN {
            libc::signal(sig, libc::SIG_IGN);
        }
        old
    });
    Interrupts { old }
}

impl Interrupts {
    pub fn interrupted(&self) -> bool {
        INTERRUPTED.load(Ordering::SeqCst)
    }
}

impl Drop for Interrupts {
    fn drop(&mut self) {
        for (sig, old) in DEFERRED.into_iter().zip(self.old) {
            unsafe { libc::signal(sig, old) };
        }
    }
}
