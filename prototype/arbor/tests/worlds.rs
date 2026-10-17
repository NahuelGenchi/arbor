//! Worlds end to end, through the `arbor` binary and bubblewrap (see `common` for when they skip).

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use arbor::seccomp;
use common::Env;

#[test]
fn a_world_changes_nothing_until_kept() {
    let Some(e) = Env::new("flow") else { return };
    let base = e.base.to_str().unwrap();
    assert_eq!(e.ok(&["new", "demo", "--from", base]).lines().count(), 1);
    e.ok(&[
        "run",
        "demo",
        "--",
        "sh",
        "-c",
        "echo two > src/a && echo new > src/b && rm src/gone",
    ]);
    assert_eq!(
        e.read("src/a").as_deref(),
        Some("one"),
        "the real folder changed before keep"
    );
    assert_eq!(e.read("src/b"), None);
    assert_eq!(e.ok(&["diff", "demo"]), "M src/a\nA src/b\nD src/gone\n");
    let patch = e.ok(&["diff", "demo", "--patch"]);
    assert!(
        patch.contains("--- a/src/a\n+++ b/src/a\n")
            && patch.contains("-one\n")
            && patch.contains("+two\n"),
        "{patch}"
    );
    assert!(e.ok(&["worlds"]).contains("demo  open   3"));

    e.ok(&["keep", "demo"]);
    assert_eq!(e.read("src/a").as_deref(), Some("two\n"));
    assert_eq!(e.read("src/b").as_deref(), Some("new\n"));
    assert_eq!(e.read("src/gone"), None);
    let refused = e.arbor(&["run", "demo", "--", "true"]);
    assert_eq!(
        refused.status.code(),
        Some(3),
        "a kept world must not run commands"
    );

    e.ok(&["undo", "demo"]);
    assert_eq!(e.read("src/a").as_deref(), Some("one"));
    assert_eq!(e.read("src/gone").as_deref(), Some("bye"));
    assert_eq!(e.ok(&["diff", "demo"]), "M src/a\nA src/b\nD src/gone\n");

    e.ok(&["discard", "demo"]);
    assert_eq!(e.read("src/a").as_deref(), Some("one"));
    assert_eq!(e.ok(&["worlds"]), "");

    // Every step that changed something or ran a command is in the ledger, the refused run included; a
    // command's cell records its start too.
    let rows = e.ledger_rows();
    let expect = [
        ("new", "ok"),
        ("start", "running"),
        ("run", "exit 0"),
        ("keep", "ok"),
        ("run", "refused"),
        ("undo", "ok"),
        ("discard", "ok"),
    ];
    assert_eq!(rows.len(), expect.len(), "{rows:#?}");
    for (row, (act, out)) in rows.iter().zip(expect) {
        assert!(
            row.contains(&format!(" {act} ")) && row.contains(&format!(" {out} ")),
            "{rows:#?}"
        );
    }
    assert!(
        rows[3].contains("3 changes (1 added, 1 modified, 1 deleted)"),
        "{rows:#?}"
    );
    assert!(
        e.ok(&["ledger", "--verify"])
            .starts_with("ledger ok: 7 steps, head ")
    );
}

#[test]
fn a_cell_is_sealed() {
    let Some(e) = Env::new("seal") else { return };
    fs::set_permissions(&e.base, fs::Permissions::from_mode(0o750)).unwrap();
    e.ok(&["new", "demo", "--from", e.base.to_str().unwrap()]);
    fs::write(e.user_home.join("notes"), "private").unwrap();
    // In order: network interfaces (/proc/net/dev lists only loopback); what is left of /run (none of the
    // computer's sockets: only the outbox's socket and the arbor that asks through it), of Arbor's state, of the
    // home folder, of /home and of the mounted drives; whether a user namespace can be made; the mode of the world's
    // root; a write to the home folder; the computer's name; the world's name.
    let script = format!(
        "grep -c : /proc/net/dev; echo $(ls -A /run /run/arbor /run/arbor/bin); ls -A '{}' | wc -l; ls -A ~ | wc -l; \
         ls -A /home | wc -l; ls -A /media /mnt 2>/dev/null | grep -c '^[^/]'; \
         unshare -U true 2>/dev/null && echo userns || echo no-userns; stat -c %a .; echo x > ~/leak && echo wrote; \
         hostname; echo $ARBOR_WORLD",
        e.home.display()
    );
    let out = e.ok(&["run", "demo", "--", "sh", "-c", &script]);
    assert_eq!(
        out,
        "1\n/run: arbor /run/arbor: bin outbox.sock /run/arbor/bin: arbor\n0\n0\n0\n0\nno-userns\n750\nwrote\narbor\n\
         demo\n",
        "unexpected cell output"
    );
    assert!(
        !e.user_home.join("leak").exists(),
        "a write to the home folder escaped the cell"
    );
}

/// Makes each kernel call given on its command line, as `number,argument,...` where `c` points to one character,
/// and prints the error it returned, or 0.
const CALLS: &str = r#"import ctypes, sys
libc = ctypes.CDLL(None, use_errno=True)
libc.syscall.restype = ctypes.c_long
c = ctypes.create_string_buffer(b"x")
for call in sys.argv[1:]:
    ctypes.set_errno(0)
    ok = libc.syscall(*[c if a == "c" else ctypes.c_long(int(a)) for a in call.split(",")]) != -1
    print(0 if ok else ctypes.get_errno())
"#;

/// Makes `getpid` as a 32-bit program would on x86_64 (`int 0x80`), from this 64-bit one, and prints the result.
#[cfg(target_arch = "x86_64")]
const X86_CALL: &str = r#"import ctypes, mmap
m = mmap.mmap(-1, 4096, prot=mmap.PROT_READ | mmap.PROT_WRITE | mmap.PROT_EXEC)
m.write(bytes([0xB8, 20, 0, 0, 0, 0xCD, 0x80, 0xC3]))
print(ctypes.CFUNCTYPE(ctypes.c_int)(ctypes.addressof(ctypes.c_char.from_buffer(m)))())
"#;

#[test]
fn a_cell_refuses_kernel_calls() {
    let Some(e) = Env::new("filter") else { return };
    let python = "/usr/bin/python3";
    if !Path::new(python).exists() {
        return common::skip(&format!("this test needs {python}"));
    }
    e.ok(&["new", "demo", "--from", e.base.to_str().unwrap()]);
    // Each call and the error it must get inside a cell. Outside one, the kernel itself answers each refused call
    // here differently, so these errors come from the filter.
    let mut calls = vec![
        (format!("{}", libc::SYS_getpid), 0),
        (format!("{},0,-3,0", libc::SYS_keyctl), libc::EPERM),
        (format!("{},0", libc::SYS_unshare), libc::EPERM),
        (format!("{},-1,0", libc::SYS_setns), libc::EPERM),
        (
            format!("{},0,0,0,0,0", libc::SYS_get_mempolicy),
            libc::EPERM,
        ),
        (
            format!("{},0,0,-1,-1,0", libc::SYS_perf_event_open),
            libc::EPERM,
        ),
        (format!("{},0,0", libc::SYS_io_uring_setup), libc::ENOSYS),
        (format!("{},c,0", libc::SYS_fsopen), libc::ENOSYS),
        // Typing into the terminal and pasting into a console are refused before the kernel looks at the
        // descriptor. Another request reaches it, and finds that /dev/null is no terminal.
        (
            format!("{},0,{},c", libc::SYS_ioctl, libc::TIOCSTI),
            libc::EPERM,
        ),
        (
            format!("{},0,{},c", libc::SYS_ioctl, libc::TIOCLINUX),
            libc::EPERM,
        ),
        (
            format!("{},0,{},c", libc::SYS_ioctl, libc::TIOCGWINSZ),
            libc::ENOTTY,
        ),
    ];
    // Then every call the filter refuses, with arguments of 0.
    for (list, errno) in [
        (seccomp::REFUSED, libc::EPERM),
        (seccomp::ABSENT, libc::ENOSYS),
    ] {
        calls.extend(list.iter().map(|nr| (format!("{nr},0,0,0,0,0,0"), errno)));
    }
    let mut args = vec!["run", "demo", "--", python, "-c", CALLS];
    args.extend(calls.iter().map(|(call, _)| call.as_str()));
    let out = e.ok(&args);
    let got: Vec<(&str, i32)> = calls
        .iter()
        .zip(out.lines())
        .map(|((call, _), errno)| (call.as_str(), errno.parse().unwrap()))
        .collect();
    let want: Vec<(&str, i32)> = calls.iter().map(|(c, e)| (c.as_str(), *e)).collect();
    assert_eq!(got, want);
    // A program that makes a 32-bit call is stopped, where this computer takes such calls at all.
    #[cfg(target_arch = "x86_64")]
    if std::process::Command::new(python)
        .args(["-c", X86_CALL])
        .output()
        .unwrap()
        .status
        .success()
    {
        let stopped = e.arbor(&["run", "demo", "--", python, "-c", X86_CALL]);
        assert_eq!(stopped.status.code(), Some(128 + libc::SIGSYS));
        assert!(stopped.stdout.is_empty());
    } else {
        common::skip("this computer does not take 32-bit calls");
    }
}

#[test]
fn exit_codes_and_refusals() {
    let Some(e) = Env::new("codes") else { return };
    let base = e.base.to_str().unwrap();
    e.ok(&["new", "demo", "--from", base]);
    assert_eq!(
        e.arbor(&["run", "demo", "--", "sh", "-c", "exit 7"])
            .status
            .code(),
        Some(7)
    );
    assert_eq!(
        e.arbor(&["new", "demo", "--from", base]).status.code(),
        Some(3),
        "a taken name"
    );
    assert_eq!(
        e.arbor(&["new", "Bad Name", "--from", base]).status.code(),
        Some(2)
    );
    assert_eq!(e.arbor(&["frobnicate"]).status.code(), Some(2));
    assert_eq!(e.arbor(&["ledger", "--last", "x"]).status.code(), Some(2));
    assert_eq!(
        e.arbor(&["run", "demo", "--net", "--", "true"])
            .status
            .code(),
        Some(2),
        "worlds have no network option"
    );
    assert_eq!(e.arbor(&["diff", "nope"]).status.code(), Some(2));
    // A world over a folder that holds Arbor's own state is refused (overlay layers may not nest).
    assert_eq!(
        e.arbor(&["new", "all", "--from", e.root.to_str().unwrap()])
            .status
            .code(),
        Some(2)
    );
    // Without a name, the world is named after its folder.
    assert!(
        e.ok(&["new", "--from", base])
            .starts_with("made world project ")
    );
}

#[test]
fn keep_waits_for_a_running_command() {
    let Some(e) = Env::new("busy") else { return };
    e.ok(&["new", "demo", "--from", e.base.to_str().unwrap()]);
    e.ok(&[
        "new",
        "beside",
        "--from",
        e.base.join("src").to_str().unwrap(),
    ]);
    let mut running = e
        .command(&["run", "demo", "--", "sh", "-c", "touch started && sleep 3"])
        .spawn()
        .unwrap();
    let marker = e.home.join("worlds/demo/up/started");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(marker.exists(), "the command never started");
    let busy = e.arbor(&["keep", "demo"]);
    assert_eq!(
        busy.status.code(),
        Some(3),
        "keep must refuse while a command runs"
    );
    assert!(String::from_utf8_lossy(&busy.stderr).contains("busy"));
    // Keeping another world over the same folder would change it under the running command.
    let near = e.arbor(&["keep", "beside"]);
    assert_eq!(
        near.status.code(),
        Some(3),
        "keep must wait for a world over the same folder"
    );
    assert!(String::from_utf8_lossy(&near.stderr).contains("world 'demo'"));
    assert!(running.wait().unwrap().success());
    e.ok(&["keep", "demo"]);
    assert!(e.base.join("started").exists());
}
