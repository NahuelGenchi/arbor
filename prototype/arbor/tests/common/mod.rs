//! What the end-to-end tests share: a throwaway project and Arbor state, and the `arbor` binary run against them.
//! Where `bwrap` cannot mount an overlay (no bubblewrap, or user namespaces blocked), each test prints why and
//! passes without checking anything, unless `ARBOR_REQUIRE_BWRAP` is set, as in CI, where that is a failure.

#![allow(dead_code)] // each test file uses some of it

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub struct Env {
    pub root: PathBuf,
    pub home: PathBuf,
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

    /// The `arbor` command, set to this test's state.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_arbor"));
        cmd.args(args)
            .env("ARBOR_HOME", &self.home)
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
