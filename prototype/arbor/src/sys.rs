//! Thin wrappers for the few Linux calls the standard library lacks: locks, atomic renames, extended attributes,
//! whiteouts, private temporary folders, the session keyring, process and folder watches, threads that leave signals
//! to the main one, and deferred interrupts. Path calls never follow a final symlink.

use std::ffi::{CString, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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

/// Makes a new folder that only its owner may enter, named `prefix` and six random characters, and returns its path.
pub fn make_temp_dir(prefix: &Path) -> io::Result<PathBuf> {
    let mut template = prefix.as_os_str().as_bytes().to_vec();
    template.extend_from_slice(b"XXXXXX\0");
    if unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) }.is_null() {
        return Err(io::Error::last_os_error());
    }
    template.pop();
    Ok(PathBuf::from(OsString::from_vec(template)))
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

/// Sends `sig` to the process `pidfd` names. Unlike a process ID, the descriptor never names another process
/// that took the number after it ended.
pub fn pidfd_send_signal(pidfd: &OwnedFd, sig: libc::c_int) -> io::Result<()> {
    let ret = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            sig,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Waits up to `timeout` for `fd` to become readable, as a pidfd does when its process ends.
pub fn readable_within(fd: BorrowedFd, timeout: Duration) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        let ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(i32::MAX as u128) as i32;
        let mut pfd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        match unsafe { libc::poll(&mut pfd, 1, ms) } {
            n if n > 0 => return Ok(true),
            0 => return Ok(false),
            _ => {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::Interrupted {
                    return Err(e);
                }
            }
        }
    }
}

/// Waits until one of `fds` is readable or hung up, or until `timeout` passes. Returns which are.
pub fn ready(fds: &[BorrowedFd], timeout: Option<Duration>) -> io::Result<Vec<bool>> {
    let mut polled: Vec<libc::pollfd> = fds
        .iter()
        .map(|fd| libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let ms = timeout.map_or(-1, |t| t.as_millis().min(i32::MAX as u128) as i32);
    loop {
        let n = unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as libc::nfds_t, ms) };
        if n >= 0 {
            return Ok(polled.iter().map(|p| p.revents != 0).collect());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// A pipe whose ends close on exec: (read, write).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    check(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Starts a thread with every signal blocked. Signals sent to arbor then reach its main thread, which waits for
/// them (see `Interrupts`), and a write to a socket whose reader went away fails with an error instead of killing
/// arbor with SIGPIPE.
pub fn spawn_quiet<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> io::Result<JoinHandle<T>> {
    spawn_quiet_on(std::thread::Builder::new(), f)
}

/// The same, on a thread that `builder` makes, such as one with a smaller stack.
pub fn spawn_quiet_on<T: Send + 'static>(
    builder: std::thread::Builder,
    f: impl FnOnce() -> T + Send + 'static,
) -> io::Result<JoinHandle<T>> {
    unsafe {
        let mut all: libc::sigset_t = std::mem::zeroed();
        libc::sigfillset(&mut all);
        let mut before: libc::sigset_t = std::mem::zeroed();
        libc::pthread_sigmask(libc::SIG_SETMASK, &all, &mut before);
        // The new thread starts with the mask of the one that makes it.
        let thread = builder.spawn(f);
        libc::pthread_sigmask(libc::SIG_SETMASK, &before, std::ptr::null_mut());
        thread
    }
}

/// Room for one descriptor in a message's control data, aligned as the kernel's `cmsghdr` needs.
#[repr(C)]
union FdRoom {
    _align: libc::cmsghdr,
    _bytes: [u8; 64],
}

/// Sends the open descriptor `fd` over the Unix socket `sock`, the receiver getting a copy of its own. One byte of
/// data goes with it, as the kernel requires.
pub fn send_fd(sock: BorrowedFd, fd: BorrowedFd) -> io::Result<()> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let mut room: FdRoom = unsafe { std::mem::zeroed() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = (&raw mut room).cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(size_of::<RawFd>() as u32) } as _;
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd.as_raw_fd());
    }
    loop {
        if unsafe { libc::sendmsg(sock.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) } >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Receives one descriptor sent with `send_fd`, closing on exec. `None` when the other end closed without
/// sending one. Any further descriptors in the message are closed.
pub fn recv_fd(sock: BorrowedFd) -> io::Result<Option<OwnedFd>> {
    let mut byte = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: byte.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let mut room: FdRoom = unsafe { std::mem::zeroed() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = (&raw mut room).cast();
    msg.msg_controllen = size_of::<FdRoom>() as _;
    loop {
        let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
        if n >= 0 {
            break;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
    let mut got = None;
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
                let count =
                    ((*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / size_of::<RawFd>();
                for i in 0..count {
                    let fd = OwnedFd::from_raw_fd(std::ptr::read_unaligned(data.add(i)));
                    // The first is kept; the others close as they drop.
                    if got.is_none() {
                        got = Some(fd);
                    }
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }
    Ok(got)
}

/// What this process's finished children have used so far: CPU milliseconds and the largest peak memory in KiB.
pub fn children_usage() -> (u64, u64) {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut ru) };
    let ms = |t: libc::timeval| t.tv_sec as u64 * 1000 + t.tv_usec as u64 / 1000;
    (ms(ru.ru_utime) + ms(ru.ru_stime), ru.ru_maxrss as u64)
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
                self.clear();
                return Ok(true);
            }
        }
    }

    /// Waits until something in the folder changes (true), or until one of `ends` is readable or hung up (false):
    /// a peer that wrote or went away, or a pipe whose other end closed.
    pub fn wait_or(&self, ends: &[BorrowedFd]) -> io::Result<bool> {
        let mut fds = vec![self.fd.as_fd()];
        fds.extend_from_slice(ends);
        if ready(&fds, None)?[1..].iter().any(|&r| r) {
            return Ok(false);
        }
        self.clear();
        Ok(true)
    }

    /// Reads the waiting events. Which file changed does not matter: the caller looks again.
    fn clear(&self) {
        let mut buf = [0u8; 4096];
        unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    }
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn note_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// While held, the signals it was made for only raise a flag that `interrupted` reads. Keep and undo stop between
/// two steps with their journal in order, never halfway through one; a cell is stopped and how it ended recorded.
/// Signals ignored before (as under `nohup`) stay ignored.
pub struct Interrupts {
    old: Vec<(libc::c_int, libc::sighandler_t)>,
}

/// Defers Ctrl-C, a closed terminal and `kill`.
pub fn defer_interrupts() -> Interrupts {
    defer(&[libc::SIGINT, libc::SIGTERM, libc::SIGHUP])
}

pub fn defer(signals: &[libc::c_int]) -> Interrupts {
    INTERRUPTED.store(false, Ordering::SeqCst);
    let handler = note_interrupt as extern "C" fn(libc::c_int) as libc::sighandler_t;
    let old = signals
        .iter()
        .map(|&sig| unsafe {
            let old = libc::signal(sig, handler);
            if old == libc::SIG_IGN {
                libc::signal(sig, libc::SIG_IGN);
            }
            (sig, old)
        })
        .collect();
    Interrupts { old }
}

impl Interrupts {
    pub fn interrupted(&self) -> bool {
        INTERRUPTED.load(Ordering::SeqCst)
    }

    /// Waits until `fd` is readable (true), or `timeout` passes or one of the signals arrives (false). The signals
    /// are held back until `ppoll` lets them in, so one that arrives just before the wait still ends it.
    pub fn poll(&self, fd: BorrowedFd, timeout: Option<Duration>) -> io::Result<bool> {
        unsafe {
            let mut held: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut held);
            for &(sig, _) in &self.old {
                libc::sigaddset(&mut held, sig);
            }
            let mut before: libc::sigset_t = std::mem::zeroed();
            libc::pthread_sigmask(libc::SIG_BLOCK, &held, &mut before);
            let mut during = before;
            for &(sig, _) in &self.old {
                libc::sigdelset(&mut during, sig);
            }
            let ts = timeout.map(|t| libc::timespec {
                tv_sec: t.as_secs() as libc::time_t,
                tv_nsec: t.subsec_nanos() as libc::c_long,
            });
            let mut pfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let n = if self.interrupted() {
                0
            } else {
                libc::ppoll(
                    &mut pfd,
                    1,
                    ts.as_ref().map_or(std::ptr::null(), |t| t as *const _),
                    &during,
                )
            };
            let err = io::Error::last_os_error();
            libc::pthread_sigmask(libc::SIG_SETMASK, &before, std::ptr::null_mut());
            match n {
                n if n > 0 => Ok(true),
                0 => Ok(false),
                _ if err.kind() == io::ErrorKind::Interrupted => Ok(false),
                _ => Err(err),
            }
        }
    }
}

impl Drop for Interrupts {
    fn drop(&mut self) {
        for &(sig, old) in &self.old {
            unsafe { libc::signal(sig, old) };
        }
    }
}
