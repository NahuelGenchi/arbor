//! What the end-to-end tests share: a throwaway project, home folder and Arbor state, and the `arbor` binary run
//! against them. Where `bwrap` cannot mount an overlay (no bubblewrap, or user namespaces blocked), each test prints
//! why and passes without checking anything, unless `ARBOR_REQUIRE_BWRAP` is set, as in CI, where that is a
//! failure. The same goes for a check that needs something else this computer may lack (`skip`). Memory, CPU and
//! process limits need a cgroup from the systemd user manager: where there is none, the tests that need one print
//! why and pass without checking it, unless `ARBOR_REQUIRE_CGROUPS` is set.

#![allow(dead_code)] // each test file uses some of it

/// Says why a test leaves out a check this computer cannot make, or fails where `ARBOR_REQUIRE_BWRAP` is set, as in
/// CI, so that there every check runs.
pub fn skip(why: &str) {
    if std::env::var_os("ARBOR_REQUIRE_BWRAP").is_some() {
        panic!("{why}, and ARBOR_REQUIRE_BWRAP is set");
    }
    eprintln!("skipped: {why}");
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub struct Env {
    pub root: PathBuf,
    /// Arbor's state (`ARBOR_HOME`).
    pub home: PathBuf,
    /// The home folder arbor and its cells are given (`HOME`), so tests can put files there.
    pub user_home: PathBuf,
    pub base: PathBuf,
}

impl Env {
    pub fn new(tag: &str) -> Option<Env> {
        let root = std::env::temp_dir().join(format!("arbor-it-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let base = root.join("project");
        fs::create_dir_all(base.join("src")).unwrap();
        fs::write(base.join("src/a"), "one").unwrap();
        fs::write(base.join("src/gone"), "bye").unwrap();
        let user_home = root.join("home");
        fs::create_dir_all(&user_home).unwrap();
        let env = Env {
            home: root.join("arbor-home"),
            user_home,
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

    /// The `arbor` command, set to this test's state.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arbor"));
        cmd.args(args)
            .env("ARBOR_HOME", &self.home)
            .env("HOME", &self.user_home)
            .stdin(Stdio::null());
        cmd
    }

    pub fn arbor(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.arbor(args);
        assert!(
            out.status.success(),
            "arbor {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn read(&self, rel: &str) -> Option<String> {
        fs::read_to_string(self.base.join(rel)).ok()
    }

    /// The ledger's rows, without the header.
    pub fn ledger_rows(&self) -> Vec<String> {
        self.ok(&["ledger", "--last", "1000"])
            .lines()
            .skip(1)
            .map(str::to_string)
            .collect()
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

pub fn new_world(e: &Env) {
    e.ok(&["new", "demo", "--from", e.base.to_str().unwrap()]);
}

/// Waits up to 10 s for a command in a world to make `path`.
pub fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(path.exists(), "the command never started");
}

/// The user manager's cgroup; cells' cgroups live in its `arbor.slice/arbor.service/cells`.
pub fn user_manager() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service"
    ))
}

/// Whether cells can have cgroups of their own here. Says why not, or fails where they must.
pub fn cgroups() -> bool {
    let why = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if Path::new(&dir).join("systemd/private").exists() => {
            let handed = fs::read_to_string(user_manager().join("cgroup.subtree_control"))
                .unwrap_or_default();
            ["cpu", "memory", "pids"]
                .into_iter()
                .find(|c| !handed.split_whitespace().any(|h| h == *c))
                .map(|c| format!("the systemd user manager does not hand on the {c} controller"))
        }
        _ => Some("there is no systemd user manager to reach".to_string()),
    };
    match why {
        None => true,
        Some(why) if std::env::var_os("ARBOR_REQUIRE_CGROUPS").is_some() => {
            panic!("cells cannot have cgroups here, and ARBOR_REQUIRE_CGROUPS is set: {why}")
        }
        Some(why) => {
            eprintln!("skipped: cells cannot have cgroups here: {why}");
            false
        }
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
