//! What a cell may not ask the kernel for: a seccomp filter that arbor builds here, as a short classic BPF program,
//! and bwrap loads last, right before the command starts.
//!
//! It refuses the few calls that real work rarely needs and that sandbox escapes and kernel bugs usually start from
//! (`REFUSED`, `ABSENT` and `TERMINAL`). Everything else is allowed, `ptrace` too, so debuggers work on the cell's own
//! processes. A program built for another architecture, such as a 32-bit one on a 64-bit PC, is stopped at its first
//! call: its call numbers differ, so the filter could not tell what it asks for.

use libc::{
    BPF_ABS, BPF_JEQ, BPF_JGE, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W, SECCOMP_RET_ALLOW,
    SECCOMP_RET_DATA, SECCOMP_RET_ERRNO, SECCOMP_RET_KILL_PROCESS, c_int, c_long, sock_filter,
};

/// The kernel's name for the architecture the call numbers below belong to (`AUDIT_ARCH_*` in linux/audit.h): its
/// ELF machine number, marked 64-bit and little-endian.
#[cfg(target_arch = "x86_64")]
const ARCH: u32 = 0xC000_0000 | 62;
#[cfg(target_arch = "aarch64")]
const ARCH: u32 = 0xC000_0000 | 183;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("cells filter kernel calls on x86_64 and aarch64 only");

/// Calls refused with "operation not permitted".
const REFUSED: &[c_long] = &[
    // The kernel keyrings, where a login session keeps secrets.
    libc::SYS_add_key,
    libc::SYS_keyctl,
    libc::SYS_request_key,
    // Mounts and namespaces: bwrap makes the cell's, and they stay as made.
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_chroot,
    libc::SYS_unshare,
    libc::SYS_setns,
    // Kernel interfaces with a long record of bugs.
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_userfaultfd,
    // NUMA page moves and policies.
    libc::SYS_move_pages,
    libc::SYS_migrate_pages,
    libc::SYS_mbind,
    libc::SYS_get_mempolicy,
    libc::SYS_set_mempolicy,
    // The kernel log, accounting and quotas.
    libc::SYS_syslog,
    libc::SYS_acct,
    libc::SYS_quotactl,
    // On x86 only: an old way to load libraries, and segment tables for 16-bit code.
    #[cfg(target_arch = "x86_64")]
    libc::SYS_uselib,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_modify_ldt,
];

/// Calls refused with "function not implemented", so that programs fall back to the older ones: io_uring, and the
/// newer mount calls.
const ABSENT: &[c_long] = &[
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_open_tree,
    libc::SYS_move_mount,
    libc::SYS_fsopen,
    libc::SYS_fsconfig,
    libc::SYS_fsmount,
    libc::SYS_fspick,
    libc::SYS_mount_setattr,
];

/// The `ioctl` requests refused with "operation not permitted": typing into the terminal (`TIOCSTI`, CVE-2017-5226)
/// and pasting into a Linux console (`TIOCLINUX`, CVE-2023-28100). A cell run at a terminal shares the person's.
const TERMINAL: [u32; 2] = [libc::TIOCSTI as u32, libc::TIOCLINUX as u32];

/// Where `struct seccomp_data` holds the call number and the architecture.
const NR: u32 = 0;
const ARCH_AT: u32 = 4;

/// Where it holds the low 32 bits of argument `i`, on a little-endian computer.
const fn arg(i: u32) -> u32 {
    16 + 8 * i
}

/// x86_64 also takes x32 calls, under the same architecture, with this bit set in the number.
#[cfg(target_arch = "x86_64")]
const X32: u32 = 0x4000_0000;

fn load(at: u32) -> sock_filter {
    sock_filter {
        code: (BPF_LD | BPF_W | BPF_ABS) as u16,
        jt: 0,
        jf: 0,
        k: at,
    }
}

/// Compares the loaded word with `k`, then skips `yes` instructions if they are equal, else `no`.
fn equals(k: u32, yes: u8, no: u8) -> sock_filter {
    sock_filter {
        code: (BPF_JMP | BPF_JEQ | BPF_K) as u16,
        jt: yes,
        jf: no,
        k,
    }
}

fn ret(action: u32) -> sock_filter {
    sock_filter {
        code: (BPF_RET | BPF_K) as u16,
        jt: 0,
        jf: 0,
        k: action,
    }
}

fn error(errno: c_int) -> u32 {
    SECCOMP_RET_ERRNO | (errno as u32 & SECCOMP_RET_DATA)
}

/// The filter's instructions, which the kernel runs for each call: the architecture first, then the call number,
/// and for `ioctl` the request.
pub fn program() -> Vec<sock_filter> {
    let mut p = vec![
        load(ARCH_AT),
        equals(ARCH, 1, 0),
        ret(SECCOMP_RET_KILL_PROCESS),
        load(NR),
    ];
    #[cfg(target_arch = "x86_64")]
    p.extend([
        sock_filter {
            code: (BPF_JMP | BPF_JGE | BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: X32,
        },
        ret(error(libc::ENOSYS)),
    ]);
    // The kernel remembers which calls the filter always allows and stops asking it about them. ioctl's answer
    // depends on the request, so the filter runs for every ioctl, and it is checked first. The kernel reads only the
    // request's low 32 bits, and so does the filter: setting the high ones cannot hide a refused request.
    let n = TERMINAL.len() as u8;
    p.push(equals(libc::SYS_ioctl as u32, 0, n + 3));
    p.push(load(arg(1)));
    for (i, request) in (0..).zip(TERMINAL) {
        p.push(equals(request, n - i, 0));
    }
    p.extend([ret(SECCOMP_RET_ALLOW), ret(error(libc::EPERM))]);
    for (calls, errno) in [(REFUSED, libc::EPERM), (ABSENT, libc::ENOSYS)] {
        for &nr in calls {
            p.extend([equals(nr as u32, 0, 1), ret(error(errno))]);
        }
    }
    p.push(ret(SECCOMP_RET_ALLOW));
    p
}

/// The filter as bwrap reads it (`--seccomp FD`): each instruction's 8 bytes, in the computer's byte order.
pub fn bytes() -> Vec<u8> {
    program()
        .iter()
        .flat_map(|i| {
            let [c0, c1] = i.code.to_ne_bytes();
            let [k0, k1, k2, k3] = i.k.to_ne_bytes();
            [c0, c1, i.jt, i.jf, k0, k1, k2, k3]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the filter answers for one call, run here the way the kernel runs it.
    fn answer(arch: u32, nr: c_long, args: [u64; 6]) -> u32 {
        let mut data = [0u8; 64];
        data[0..4].copy_from_slice(&(nr as i32).to_ne_bytes());
        data[4..8].copy_from_slice(&arch.to_ne_bytes());
        for (i, a) in args.iter().enumerate() {
            data[16 + 8 * i..24 + 8 * i].copy_from_slice(&a.to_ne_bytes());
        }
        let p = program();
        let (mut pc, mut acc) = (0, 0u32);
        loop {
            let i = p[pc];
            pc += 1;
            let skip = |yes: bool| usize::from(if yes { i.jt } else { i.jf });
            match u32::from(i.code) {
                c if c == BPF_LD | BPF_W | BPF_ABS => {
                    let at = i.k as usize;
                    acc = u32::from_ne_bytes(data[at..at + 4].try_into().unwrap());
                }
                c if c == BPF_JMP | BPF_JEQ | BPF_K => pc += skip(acc == i.k),
                c if c == BPF_JMP | BPF_JGE | BPF_K => pc += skip(acc >= i.k),
                c if c == BPF_RET | BPF_K => return i.k,
                c => panic!("unexpected instruction {c:#x}"),
            }
        }
    }

    fn call(nr: c_long) -> u32 {
        answer(ARCH, nr, [0; 6])
    }

    fn ioctl(request: u64) -> u32 {
        answer(ARCH, libc::SYS_ioctl, [1, request, 0, 0, 0, 0])
    }

    #[test]
    fn refuses_the_calls_a_cell_does_not_need() {
        for &nr in REFUSED {
            assert_eq!(call(nr), error(libc::EPERM), "call {nr}");
        }
        for &nr in ABSENT {
            assert_eq!(call(nr), error(libc::ENOSYS), "call {nr}");
        }
        assert_eq!(call(libc::SYS_keyctl), SECCOMP_RET_ERRNO | 1);
        for nr in [
            libc::SYS_read,
            libc::SYS_openat,
            libc::SYS_execve,
            libc::SYS_clone,
            libc::SYS_clone3,
            libc::SYS_ptrace,
            libc::SYS_socket,
        ] {
            assert_eq!(call(nr), SECCOMP_RET_ALLOW, "call {nr}");
        }
    }

    #[test]
    fn refuses_typing_into_the_terminal_and_no_other_ioctl() {
        assert_eq!(ioctl(libc::TIOCSTI), error(libc::EPERM));
        assert_eq!(ioctl(libc::TIOCLINUX), error(libc::EPERM));
        // The kernel ignores the high bits, so they must not hide the request.
        assert_eq!(
            ioctl(0xffff_ffff_0000_0000 | libc::TIOCSTI),
            error(libc::EPERM)
        );
        for request in [libc::TCGETS, libc::TIOCGWINSZ, libc::FIONREAD] {
            assert_eq!(ioctl(request), SECCOMP_RET_ALLOW, "{request:#x}");
        }
        // The request is the second argument: the same number as the first is a descriptor.
        let first = [libc::TIOCSTI, 0, 0, 0, 0, 0];
        assert_eq!(answer(ARCH, libc::SYS_ioctl, first), SECCOMP_RET_ALLOW);
    }

    #[test]
    fn stops_programs_built_for_another_architecture() {
        const I386: u32 = 0x4000_0000 | 3;
        const ARM: u32 = 0x4000_0000 | 40;
        for arch in [I386, ARM, 0] {
            assert_eq!(
                answer(arch, libc::SYS_read, [0; 6]),
                SECCOMP_RET_KILL_PROCESS
            );
        }
        #[cfg(target_arch = "x86_64")]
        for nr in [libc::SYS_read, libc::SYS_ioctl, libc::SYS_keyctl] {
            assert_eq!(call(X32 as c_long | nr), error(libc::ENOSYS), "x32 {nr}");
        }
    }

    #[test]
    fn is_a_program_the_kernel_takes() {
        let p = program();
        assert!(p.len() <= 4096, "{} instructions", p.len());
        for (pc, i) in p.iter().enumerate() {
            if u32::from(i.code) & 0x07 == BPF_JMP {
                let far = usize::from(i.jt.max(i.jf));
                assert!(pc + 1 + far < p.len(), "jump at {pc} leaves the program");
            }
        }
        assert_eq!(u32::from(p[p.len() - 1].code), BPF_RET | BPF_K);
        // The second instruction compares the architecture, which is its last 4 bytes.
        let b = bytes();
        assert_eq!(b.len(), p.len() * 8);
        assert_eq!(&b[12..16], &ARCH.to_ne_bytes());
    }
}
