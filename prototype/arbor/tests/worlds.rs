//! End to end through the `arbor` binary and bubblewrap. Where `bwrap` cannot mount an overlay (no bubblewrap,
//! or user namespaces blocked), each test prints why and passes without checking anything, unless
//! `ARBOR_REQUIRE_BWRAP` is set, as in CI, where that is a failure.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Env {
    root: PathBuf,
    home: PathBuf,
    base: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Option<Env> {
        let root = std::env::temp_dir().join(format!("arbor-it-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let base = root.join("project");
        fs::create_dir_all(base.join("src")).unwrap();
        fs::write(base.join("src/a"), "one").unwrap();
        fs::write(base.join("src/gone"), "bye").unwrap();
        let env = Env {
            home: root.join("arbor-home"),
            base,
            root,
        };
        if let Err(why) = bwrap_overlay_works(&env.root) {
            if std::env::var_os("ARBOR_REQUIRE_BWRAP").is_some() {
                panic!("bwrap cannot mount an overlay here, and ARBOR_REQUIRE_BWRAP is set: {why}");
            }
            eprintln!("skipped: bwrap cannot mount an overlay here: {why}");
            return None;
        }
        Some(env)
    }

    fn arbor(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_arbor"))
            .args(args)
            .env("ARBOR_HOME", &self.home)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.arbor(args);
        assert!(
            out.status.success(),
            "arbor {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn read(&self, rel: &str) -> Option<String> {
        fs::read_to_string(self.base.join(rel)).ok()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = Command::new("chmod")
            .arg("-R")
            .arg("u+rwx")
            .arg(&self.root)
            .status();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn bwrap_overlay_works(scratch: &Path) -> Result<(), String> {
    let probe = scratch.join("probe");
    fs::create_dir_all(&probe).unwrap();
    let out = Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--overlay-src"])
        .arg(&probe)
        .arg("--tmp-overlay")
        .arg(&probe)
        .args(["--unshare-all", "true"])
        .output()
        .map_err(|e| format!("cannot run bwrap: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

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
}

#[test]
fn a_cell_has_no_network_and_cannot_see_arbor_or_keep_home_writes() {
    let Some(e) = Env::new("seal") else { return };
    e.ok(&["new", "demo", "--from", e.base.to_str().unwrap()]);
    let leak = format!(".arbor-it-leak-{}", std::process::id());
    let script = format!(
        "grep -c : /proc/net/dev; ls -A \"$ARBOR_HOME\" | wc -l; echo x > \"$HOME/{leak}\" && echo wrote; echo $ARBOR_WORLD"
    );
    let out = e.ok(&["run", "demo", "--", "sh", "-c", &script]);
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let leaked = home.join(&leak).exists();
    let _ = fs::remove_file(home.join(&leak));
    // /proc/net/dev lists the namespace's interfaces: only loopback.
    assert_eq!(out, "1\n0\nwrote\ndemo\n", "unexpected cell output");
    assert!(!leaked, "a write to the home folder escaped the cell");
    let with_net = e.ok(&[
        "run",
        "demo",
        "--net",
        "--",
        "grep",
        "-c",
        ":",
        "/proc/net/dev",
    ]);
    assert_ne!(
        with_net.trim(),
        "1",
        "--net should share the computer's interfaces"
    );
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
    let mut running = Command::new(env!("CARGO_BIN_EXE_arbor"))
        .args(["run", "demo", "--", "sh", "-c", "touch started && sleep 3"])
        .env("ARBOR_HOME", &e.home)
        .stdin(Stdio::null())
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
    assert!(running.wait().unwrap().success());
    e.ok(&["keep", "demo"]);
    assert!(e.base.join("started").exists());
}
