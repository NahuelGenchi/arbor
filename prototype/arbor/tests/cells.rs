//! Cells end to end: limits, `arbor ps` and `arbor watch`. Memory, CPU and process limits need a cgroup from the
//! systemd user manager. Where there is none, the tests that need one print why and pass without checking it,
//! unless `ARBOR_REQUIRE_CGROUPS` is set, as in CI, where that is a failure. All of them skip like `common`'s.

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::Env;

/// The user manager's cgroup; cells' cgroups live in its `arbor.slice/arbor.service/cells`.
fn user_manager() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service"
    ))
}

/// Whether cells can have cgroups of their own here. Says why not, or fails where they must.
fn cgroups() -> bool {
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

fn new_world(e: &Env) {
    e.ok(&["new", "demo", "--from", e.base.to_str().unwrap()]);
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(path.exists(), "the command never started");
}

#[test]
fn a_memory_hog_is_stopped_at_its_limit() {
    let Some(e) = Env::new("mem") else { return };
    if !cgroups() {
        return;
    }
    new_world(&e);
    // Files in the cell's /tmp are memory: 200 MiB of them cannot fit in 32.
    let hog = "head -c 200M /dev/zero > /tmp/fill";
    let out = e.arbor(&["run", "demo", "--mem", "32M", "--", "sh", "-c", hog]);
    assert_eq!(out.status.code(), Some(137));
    assert!(String::from_utf8_lossy(&out.stderr).contains("memory limit (32MiB)"));
    let rows = e.ledger_rows();
    let last = rows.last().unwrap();
    assert!(
        last.contains(" killed: memory limit ") && last.contains("[mem 32MiB]"),
        "{rows:#?}"
    );
}

#[test]
fn a_fork_loop_stops_at_the_process_limit() {
    let Some(e) = Env::new("pids") else { return };
    if !cgroups() {
        return;
    }
    new_world(&e);
    let forks = "i=0; while [ $i -lt 40 ]; do sleep 5 & i=$((i+1)); done; wait";
    let started = Instant::now();
    let out = e.arbor(&["run", "demo", "--pids", "8", "--", "sh", "-c", forks]);
    assert_ne!(out.status.code(), Some(0), "every fork succeeded");
    // The sleeps that did start die with the cell instead of running on.
    assert!(started.elapsed() < Duration::from_secs(4));
    let rows = e.ledger_rows();
    assert!(
        rows.last().unwrap().contains(", process limit hit "),
        "{rows:#?}"
    );
}

#[test]
fn a_cell_is_stopped_at_its_time_limit_with_or_without_a_cgroup() {
    let Some(e) = Env::new("time") else { return };
    new_world(&e);
    for no_cgroup in ["", "1"] {
        let started = Instant::now();
        let out = e
            .command(&["run", "demo", "--time", "1s", "--", "sleep", "30"])
            .env("ARBOR_NO_CGROUP", no_cgroup)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(137));
        assert!(started.elapsed() < Duration::from_secs(10));
        let rows = e.ledger_rows();
        assert!(
            rows.last().unwrap().contains(" killed: time limit "),
            "{rows:#?}"
        );
    }
    // The other limits need a cgroup: without one they are refused, never ignored.
    let out = e
        .command(&["run", "demo", "--mem", "64M", "--", "true"])
        .env("ARBOR_NO_CGROUP", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    // Values are checked before anything runs.
    for (flag, bad) in [
        ("--mem", "lots"),
        ("--cpu", "0"),
        ("--pids", "0"),
        ("--time", "soon"),
    ] {
        let out = e.arbor(&["run", "demo", flag, bad, "--", "true"]);
        assert_eq!(out.status.code(), Some(2), "{flag} {bad}");
    }
}

#[test]
fn ps_and_watch_follow_a_running_cell() {
    let Some(e) = Env::new("watch") else { return };
    let with_cgroup = cgroups();
    new_world(&e);
    let verified = e.ok(&["ledger", "--verify"]);
    let head = verified.split_whitespace().nth(2).unwrap();
    let mut watch = e
        .command(&["watch", "--since", head])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout = watch.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let next = || -> serde_json::Value {
        let line = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        serde_json::from_str(&line).unwrap()
    };

    // A limit that needs a cgroup puts the cell in one of its own.
    let mut args = vec!["run", "demo"];
    if with_cgroup {
        args.extend(["--pids", "64"]);
    }
    args.extend(["--", "sh", "-c", "touch started && sleep 2"]);
    let mut running = e.command(&args).spawn().unwrap();
    wait_for(&e.home.join("worlds/demo/up/started"));
    let ps = e.ok(&["ps"]);
    assert!(
        ps.lines().count() == 2 && ps.contains(" demo ") && ps.contains("touch started"),
        "{ps}"
    );
    let start = next();
    assert_eq!(
        (&start["act"], &start["out"]),
        (&"start".into(), &"running".into())
    );
    let cell = start["cell"].as_str().unwrap().to_string();

    assert!(running.wait().unwrap().success());
    let end = next();
    assert_eq!(end["act"], "run");
    assert_eq!(end["out"], "exit 0");
    assert_eq!(end["cell"].as_str(), Some(cell.as_str()));
    assert_eq!(e.ok(&["ps"]), "", "a finished cell is still listed");
    if with_cgroup {
        // Linux keeps a peak process count since 6.x; where it does, bwrap and the shell are in it.
        assert!(end["cost"]["pids"].as_u64().is_none_or(|p| p >= 2), "{end}");
        let group = user_manager()
            .join("arbor.slice/arbor.service/cells")
            .join(&cell);
        assert!(!group.exists(), "the cell's cgroup was left behind");
    }
    watch.kill().unwrap();
    watch.wait().unwrap();
}

#[test]
fn a_cell_dies_with_its_arbor_and_ps_cleans_up() {
    let Some(e) = Env::new("orphan") else { return };
    new_world(&e);
    let beat = "touch started; while :; do date +%s%N > beat; sleep 0.05; done";
    let mut running = e
        .command(&["run", "demo", "--", "sh", "-c", beat])
        .spawn()
        .unwrap();
    wait_for(&e.home.join("worlds/demo/up/beat"));
    // arbor is killed outright, with no chance to clean up.
    running.kill().unwrap();
    running.wait().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let read = || fs::read_to_string(e.home.join("worlds/demo/up/beat")).unwrap_or_default();
    let last = read();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(read(), last, "the cell outlived the arbor that ran it");
    // Its entry is cleaned up, and the world is free for the next command.
    assert_eq!(e.ok(&["ps"]), "");
    assert_eq!(fs::read_dir(e.home.join("cells")).unwrap().count(), 0);
    e.ok(&["run", "demo", "--", "true"]);
}
