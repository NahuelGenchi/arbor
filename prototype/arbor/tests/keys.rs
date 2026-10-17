//! Keys end to end: a cell sees only its world until a key opens more, for a while and within limits, and a key
//! taken back or ended stops what runs under it. They skip like `common` says.

mod common;

use std::fs;
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::{Env, cgroups, new_world, wait_for};

/// Runs `script` in world demo, with `key` if given, and returns what it printed.
fn inside(e: &Env, key: Option<&str>, script: &str) -> String {
    let mut args = vec!["run", "demo"];
    if let Some(k) = key {
        args.extend(["--key", k]);
    }
    args.extend(["--", "sh", "-c", script]);
    e.ok(&args)
}

#[test]
fn the_home_folder_is_hidden_until_a_key_opens_it_and_password_files_stay_hidden() {
    let Some(e) = Env::new("grants") else { return };
    new_world(&e);
    let home = &e.user_home;
    fs::write(home.join("notes"), "private").unwrap();
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/id_ed25519"), "secret").unwrap();
    fs::create_dir_all(home.join(".cargo/bin")).unwrap();
    fs::write(home.join(".cargo/config.toml"), "[build]").unwrap();
    fs::write(home.join(".cargo/credentials.toml"), "token").unwrap();
    let h = home.to_str().unwrap();
    let cat =
        |key: Option<&str>, path: &str| inside(&e, key, &format!("cat {path} || echo hidden"));

    // Sealed, nothing of the home folder shows.
    assert_eq!(cat(None, "~/notes"), "hidden\n");
    // A key opens a file...
    e.ok(&["key", "new", "notes", "--read", &format!("{h}/notes")]);
    assert_eq!(cat(Some("notes"), "~/notes"), "private");
    assert_eq!(cat(Some("notes"), "~/.cargo/config.toml"), "hidden\n");
    // ...or a folder, with the password files in it still hidden...
    e.ok(&["key", "new", "cargo", "--read", &format!("{h}/.cargo")]);
    assert_eq!(cat(Some("cargo"), "~/.cargo/config.toml"), "[build]");
    assert_eq!(cat(Some("cargo"), "~/.cargo/credentials.toml"), "");
    // ...even when it opens the whole home folder.
    e.ok(&["key", "new", "all", "--read", h]);
    assert_eq!(cat(Some("all"), "~/notes"), "private");
    assert_eq!(cat(Some("all"), "~/.ssh/id_ed25519"), "hidden\n");
    assert_eq!(cat(Some("all"), "~/.cargo/credentials.toml"), "");
    // A password folder shows only when a key opens it by name.
    e.ok(&["key", "new", "ssh", "--read", &format!("{h}/.ssh")]);
    assert_eq!(cat(Some("ssh"), "~/.ssh/id_ed25519"), "secret");
    // Writes to an opened folder are thrown away.
    inside(&e, Some("cargo"), "echo x > ~/.cargo/new");
    assert!(!home.join(".cargo/new").exists());
}

#[test]
fn a_command_the_world_hides_is_named_with_the_key_that_opens_it() {
    let Some(e) = Env::new("tools") else { return };
    new_world(&e);
    let home = &e.user_home;
    let script = |path: &std::path::Path, body: &str| {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    };
    script(&home.join(".cargo/bin/hello"), "#!/bin/sh\necho hello\n");
    // As rustup links cargo to itself.
    std::os::unix::fs::symlink("hello", home.join(".cargo/bin/linked")).unwrap();
    script(&home.join("bin/own"), "#!/bin/sh\necho own\n");
    // A script in the world whose interpreter the world hides.
    script(&e.base.join("greet"), "#!/usr/bin/env hello\n");
    let path = ["", ".cargo/bin", ".npm-global/bin", "bin"]
        .iter()
        .map(|d| home.join(d).display().to_string())
        .skip(1)
        .chain(["/usr/bin".into(), "/bin".into()])
        .collect::<Vec<_>>()
        .join(":");
    let run = |args: &[&str]| {
        let out = e.command(args).env("PATH", &path).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        (
            out.status.code(),
            String::from_utf8(out.stdout).unwrap(),
            err,
        )
    };
    let refused = |cmd: &str, key: Option<&str>| {
        let mut args = vec!["run", "demo"];
        args.extend(key.map(|k| ["--key", k]).into_iter().flatten());
        args.extend(["--", cmd]);
        let (code, _, err) = run(&args);
        assert_eq!(code, Some(127), "{cmd}: {err}");
        err
    };

    // Before it starts, arbor says where the command is and which key option opens it.
    let err = refused("hello", None);
    assert!(
        err.contains("`hello` is in ~/.cargo/bin, which the world hides")
            && err.contains("`arbor key new dev --toolchains`, then add `--key dev`"),
        "{err}"
    );
    let err = refused("linked", None);
    assert!(
        err.contains("`linked` is in ~/.cargo/bin, which the world hides"),
        "{err}"
    );
    assert!(refused("own", None).contains("`arbor key new dev --read ~/bin`"));
    let err = refused("./greet", None);
    assert!(
        err.contains("`./greet` needs `hello`, and `hello` is in ~/.cargo/bin"),
        "{err}"
    );
    assert!(refused("missing", None).contains("world demo has no command `missing`"));
    assert!(e.ledger_rows().last().unwrap().contains("not found"));

    // `--toolchains` opens npm's folder for tools installed without root only where there is one.
    assert!(
        !e.ok(&["key", "new", "early", "--toolchains"])
            .contains(".npm-global")
    );
    script(&home.join(".npm-global/bin/hi"), "#!/bin/sh\necho hi\n");
    assert!(refused("hi", None).contains("`arbor key new dev --toolchains`"));
    let made = e.ok(&["key", "new", "dev", "--toolchains"]);
    assert!(
        made.contains("~/.cargo") && made.contains("~/.npm-global"),
        "{made}"
    );
    for (cmd, said) in [("hello", "hello\n"), ("hi", "hi\n"), ("./greet", "hello\n")] {
        let (code, out, err) = run(&["run", "demo", "--key", "dev", "--", cmd]);
        assert_eq!((code, out.as_str()), (Some(0), said), "{cmd}: {err}");
    }
    // A key that does not open it is named, and the suggested key's name is free.
    let err = refused("own", Some("dev"));
    assert!(
        err.contains("which the world hides and key 'dev' does not open")
            && err.contains("`arbor key new tools --read ~/bin`"),
        "{err}"
    );
}

#[test]
fn a_variable_passes_in_only_with_a_key_that_names_it() {
    let Some(e) = Env::new("env") else { return };
    new_world(&e);
    e.ok(&["key", "new", "foo", "--env", "FOO"]);
    let echo = |key: &[&str]| {
        let mut args = vec!["run", "demo"];
        args.extend(key);
        args.extend(["--", "sh", "-c", "echo ${FOO-unset} ${TOKEN-unset} $HOME"]);
        let out = e
            .command(&args)
            .env("FOO", "bar")
            .env("TOKEN", "t0ken")
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let home = e.user_home.display();
    assert_eq!(echo(&[]), format!("unset unset {home}\n"));
    assert_eq!(echo(&["--key", "foo"]), format!("bar unset {home}\n"));
}

#[test]
fn keys_are_checked_and_never_widened() {
    let Some(e) = Env::new("narrow") else { return };
    new_world(&e);
    let cargo = e.user_home.join(".cargo");
    fs::create_dir_all(cargo.join("bin")).unwrap();
    let cargo = cargo.to_str().unwrap();
    let code = |args: &[&str]| e.arbor(args).status.code();
    e.ok(&[
        "key", "new", "dev", "--world", "demo", "--read", cargo, "--env", "FOO", "--time", "10m",
        "--for", "1h",
    ]);

    // Narrower is fine, and what it leaves out it takes from the key it was made within.
    e.ok(&[
        "key",
        "new",
        "tests",
        "--within",
        "dev",
        "--read",
        &format!("{cargo}/bin"),
        "--time",
        "5m",
    ]);
    let keys = e.ok(&["keys"]);
    let tests = keys.lines().find(|l| l.starts_with("tests ")).unwrap();
    for part in [" dev ", " demo ", " time 5m ", " FOO ", "/.cargo/bin"] {
        assert!(tests.contains(part), "{part} in {keys}");
    }
    // Wider is refused, whatever widens it.
    for wider in [
        &["--world", "shop"][..],
        &["--read", "/usr/share"],
        &["--env", "TOKEN"],
        &["--time", "1h"],
        &["--for", "2h"],
    ] {
        let mut args = vec!["key", "new", "wide", "--within", "dev"];
        args.extend(wider);
        assert_eq!(code(&args), Some(3), "{wider:?}");
    }

    // Mistakes in how a key is asked for.
    for (args, want) in [
        (
            &["key", "new", "x", "--for", "1h", "--until", "18:00"][..],
            2,
        ),
        (&["key", "new", "x", "--until", "noon"], 2),
        (&["key", "new", "Bad Name"], 2),
        (&["key", "new", "x", "--read", "/no/such/path"], 2),
        (&["key", "new", "x", "--env", "A-B"], 2),
        (&["key", "new", "x", "--read", "/proc"], 3),
        (&["key", "new", "dev"], 3),
        (&["key", "frobnicate"], 2),
        (&["run", "demo", "--key", "nosuch", "--", "true"], 2),
    ] {
        assert_eq!(code(args), Some(want), "{args:?}");
    }

    // A key opens only its worlds.
    e.ok(&["new", "other", "--from", e.base.to_str().unwrap()]);
    assert_eq!(
        code(&["run", "other", "--key", "dev", "--", "true"]),
        Some(3)
    );
    assert_eq!(
        code(&["run", "demo", "--key", "dev", "--", "true"]),
        Some(0)
    );
    // Refusals are in the ledger, with the key; mistakes in how arbor was called are not.
    let rows = e.ledger_rows();
    let refused = |act: &str, key: &str| {
        rows.iter().any(|r| {
            r.contains(&format!(" {act} "))
                && r.contains(&format!(" {key} "))
                && r.contains(" refused ")
        })
    };
    assert!(refused("key", "wide") && refused("run", "dev"), "{rows:#?}");
    // Of the mistakes above, only the refused grant is a step: the others changed nothing.
    assert_eq!(
        rows.iter().filter(|r| r.contains(" x ")).count(),
        1,
        "{rows:#?}"
    );
}

#[test]
fn a_key_that_ends_stops_its_command() {
    let Some(e) = Env::new("expire") else { return };
    new_world(&e);
    e.ok(&["key", "new", "brief", "--for", "1s"]);
    let started = Instant::now();
    let out = e.arbor(&["run", "demo", "--key", "brief", "--", "sleep", "30"]);
    assert_eq!(out.status.code(), Some(137));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(String::from_utf8_lossy(&out.stderr).contains("key 'brief' ended at"));
    // Once ended, it opens nothing.
    assert_eq!(
        e.arbor(&["run", "demo", "--key", "brief", "--", "true"])
            .status
            .code(),
        Some(3)
    );
    assert!(e.ok(&["keys"]).contains(" ended "));
    // Every step under the key carries it.
    let rows = e.ledger_rows();
    let acts: Vec<(&str, &str)> = rows
        .iter()
        .filter(|r| r.contains(" brief "))
        .map(|r| {
            let act = r.split_whitespace().nth(3).unwrap();
            let out = [" ok ", " running ", " killed: key expired ", " refused "]
                .into_iter()
                .find(|o| r.contains(o))
                .unwrap_or("?");
            (act, out.trim())
        })
        .collect();
    assert_eq!(
        acts,
        [
            ("key", "ok"),
            ("start", "running"),
            ("run", "killed: key expired"),
            ("run", "refused")
        ],
        "{rows:#?}"
    );
}

#[test]
fn a_key_s_limits_hold_whatever_the_command_asks() {
    let Some(e) = Env::new("keylimit") else {
        return;
    };
    if !cgroups() {
        return;
    }
    new_world(&e);
    e.ok(&["key", "new", "small", "--mem", "32M"]);
    // Asking for more memory than the key allows gets the key's limit.
    let hog = "head -c 200M /dev/zero > /tmp/fill";
    let out = e.arbor(&[
        "run", "demo", "--key", "small", "--mem", "1G", "--", "sh", "-c", hog,
    ]);
    assert_eq!(out.status.code(), Some(137));
    let rows = e.ledger_rows();
    let last = rows.last().unwrap();
    assert!(
        last.contains(" small ")
            && last.contains(" killed: memory limit ")
            && last.contains("[mem 32MiB]"),
        "{rows:#?}"
    );
}

#[test]
fn taking_a_key_back_stops_what_runs_under_the_keys_made_within_it() {
    let Some(e) = Env::new("takeback") else {
        return;
    };
    new_world(&e);
    e.ok(&["key", "new", "outer"]);
    e.ok(&["key", "new", "inner", "--within", "outer", "--for", "1m"]);
    let started = Instant::now();
    let running = e
        .command(&[
            "run",
            "demo",
            "--key",
            "inner",
            "--",
            "sh",
            "-c",
            "touch started; sleep 30",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&e.home.join("worlds/demo/up/started"));
    assert_eq!(
        e.ok(&["key", "take-back", "outer"]),
        "took back key outer, and key inner made within it; stopped 1 command\n"
    );
    let done = running.wait_with_output().unwrap();
    assert_eq!(done.status.code(), Some(137));
    assert!(String::from_utf8_lossy(&done.stderr).contains("key 'inner' was taken back"));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(e.ok(&["keys"]), "");
    assert_eq!(e.ok(&["ps"]), "");
    let rows = e.ledger_rows();
    let n = rows.len();
    assert!(
        rows[n - 2].contains(" run ")
            && rows[n - 2].contains(" inner ")
            && rows[n - 2].contains(" stopped: key taken back "),
        "{rows:#?}"
    );
    assert!(
        rows[n - 1].contains(" take-back ") && rows[n - 1].contains(" outer "),
        "{rows:#?}"
    );
}

#[test]
fn stop_ends_a_running_command() {
    let Some(e) = Env::new("stop") else { return };
    new_world(&e);
    let started = Instant::now();
    let running = e
        .command(&["run", "demo", "--", "sh", "-c", "touch started; sleep 30"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(&e.home.join("worlds/demo/up/started"));
    let ps = e.ok(&["ps"]);
    let id = ps
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let out = e.ok(&["stop", id]);
    assert!(
        out.starts_with(&format!("stopped cell {id} in world demo")),
        "{out}"
    );
    let done = running.wait_with_output().unwrap();
    assert_eq!(done.status.code(), Some(137));
    assert!(String::from_utf8_lossy(&done.stderr).contains("the command was stopped"));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(e.ok(&["ps"]), "");
    assert_eq!(
        e.arbor(&["stop", id]).status.code(),
        Some(2),
        "a cell that ended"
    );
    let rows = e.ledger_rows();
    let n = rows.len();
    assert!(
        rows[n - 2].contains(" run ") && rows[n - 2].contains(" stopped "),
        "{rows:#?}"
    );
    assert!(
        rows[n - 1].contains(" stop ") && rows[n - 1].contains(" ok "),
        "{rows:#?}"
    );
    // The world is free for the next command.
    e.ok(&["run", "demo", "--", "true"]);
}
