//! The outbox end to end: a command in a world asks for an effect, which waits for a person. Released, it runs
//! once, in the real folder, and leaves a receipt; refused, or left behind by a world thrown away or a key taken
//! back, it never runs. A key's budget holds at release. They skip like `common` says.

mod common;

use std::fs;
use std::process::{Child, Output, Stdio};

use common::{Env, new_world, wait_for};

/// Runs `script` in world demo, with `key` if given.
fn inside(e: &Env, key: Option<&str>, script: &str) -> Output {
    let mut args = vec!["run", "demo"];
    if let Some(k) = key {
        args.extend(["--key", k]);
    }
    args.extend(["--", "sh", "-c", script]);
    e.arbor(&args)
}

/// Starts `script` in world demo, and leaves it running.
fn start(e: &Env, script: &str) -> Child {
    e.command(&["run", "demo", "--", "sh", "-c", script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Asks for an effect from world demo, `args` being what follows `arbor effect`, and returns its number.
fn ask(e: &Env, key: Option<&str>, args: &str) -> u64 {
    let out = inside(e, key, &format!("arbor effect {args}"));
    assert!(out.status.success(), "{args}: {}", text(&out.stderr));
    let said = text(&out.stdout);
    said.strip_prefix("effect ")
        .and_then(|s| s.strip_suffix(" waiting\n"))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{args}: {said}"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Each step in the ledger as `act by result`, then `#N` for the effect it is about and the money it cost.
fn steps(e: &Env) -> Vec<String> {
    fs::read_to_string(e.home.join("ledger.jsonl"))
        .unwrap()
        .lines()
        .map(|line| {
            let s: serde_json::Value = serde_json::from_str(line).unwrap();
            let mut step = format!("{} {} {}", s["act"], s["by"], s["out"]).replace('"', "");
            if let Some(id) = s["effect"].as_u64() {
                step += &format!(" #{id}");
            }
            if let Some(cents) = s["cost"]["cents"].as_u64() {
                step += &format!(" {cents}c");
            }
            step
        })
        .collect()
}

#[test]
fn an_effect_waits_then_runs_once_in_the_real_folder_with_a_receipt() {
    let Some(e) = Env::new("effect") else { return };
    new_world(&e);
    // Asked for from a folder of the world, it is to run in the same folder of the real one.
    let out = inside(
        &e,
        None,
        "cd src && arbor effect --cost 0.40 --why 'tell the team' -- sh -c 'echo sent > outside; pwd; exit 7'",
    );
    assert_eq!(
        text(&out.stdout),
        "effect 1 waiting\n",
        "{}",
        text(&out.stderr)
    );
    // It waits for a person: nothing ran.
    assert_eq!(e.read("src/outside"), None);
    let list = e.ok(&["outbox"]);
    let row = list.lines().nth(1).unwrap();
    assert!(
        row.starts_with("1   waiting  demo   -    $0.40  "),
        "{list}"
    );
    assert!(
        row.ends_with("  sh -c 'echo sent > outside; pwd; exit 7'"),
        "{list}"
    );
    let real = e.base.join("src").display().to_string();
    let about = e.ok(&["outbox", "1"]);
    for part in [
        "effect 1, waiting since ".to_string(),
        format!("  in      {real}, the real folder, as you, with the network\n"),
        "  cost    $0.40, by the asker's word\n".into(),
        "  why     (the asker's words, not checked) tell the team\n".into(),
    ] {
        assert!(about.contains(&part), "{part} in {about}");
    }

    // Released, it runs once, as the person, and arbor ends as it did.
    let out = e.arbor(&["release", "1"]);
    assert_eq!(out.status.code(), Some(7), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), format!("{real}\n"));
    assert_eq!(e.read("src/outside").as_deref(), Some("sent\n"));
    let again = e.arbor(&["release", "1"]);
    assert_eq!(again.status.code(), Some(3));
    assert!(
        text(&again.stderr).contains("an effect runs only once"),
        "{}",
        text(&again.stderr)
    );

    // Its receipt: how it ended, and what it printed.
    assert_eq!(e.ok(&["outbox"]), "");
    let all = e.ok(&["outbox", "--all"]);
    assert!(
        all.lines().nth(1).unwrap().starts_with("1   exit 7  demo"),
        "{all}"
    );
    let about = e.ok(&["outbox", "1"]);
    assert!(
        about.contains(&format!("  output  {}B, SHA-256 ", real.len() + 1)),
        "{about}"
    );
    assert_eq!(
        fs::read_to_string(e.home.join("outbox/1.out")).unwrap(),
        format!("{real}\n")
    );
    // The ledger has each step, the release that would have run it again included.
    assert_eq!(
        steps(&e),
        [
            "new you ok",
            "start you running",
            "effect cell waiting #1",
            "run you exit 0",
            "release you running #1",
            "release you exit 7 #1 40c",
            "release you refused #1",
        ]
    );
    assert!(
        e.ok(&["ledger", "--verify"])
            .starts_with("ledger ok: 7 steps, head ")
    );
}

#[test]
fn a_command_that_waits_hears_the_answer() {
    let Some(e) = Env::new("wait") else { return };
    new_world(&e);
    // Released: what the effect printed comes back to the command that asked, and how it ended.
    let asking = start(
        &e,
        "printf 'hi there' | arbor effect --wait --input - -- sh -c 'cat; echo; exit 4'; echo \"exit $?\"",
    );
    wait_for(&e.home.join("outbox/1.json"));
    let released = e.arbor(&["release", "1"]);
    assert_eq!(released.status.code(), Some(4));
    assert_eq!(text(&released.stdout), "hi there\n");
    let done = asking.wait_with_output().unwrap();
    assert_eq!(text(&done.stdout), "hi there\nexit 4\n");
    assert!(
        text(&done.stderr).contains("effect 1 waiting"),
        "{}",
        text(&done.stderr)
    );

    // Refused: why comes back.
    let asking = start(&e, "arbor effect --wait -- touch never; echo \"exit $?\"");
    wait_for(&e.home.join("outbox/2.json"));
    assert_eq!(
        e.ok(&["refuse", "2", "--why", "not today"]),
        "refused effect 2: touch never\n"
    );
    let done = asking.wait_with_output().unwrap();
    assert_eq!(text(&done.stdout), "exit 3\n");
    assert!(
        text(&done.stderr).contains("effect 2 was refused: not today"),
        "{}",
        text(&done.stderr)
    );
    assert!(!e.base.join("never").exists());
}

#[test]
fn a_key_s_budget_holds_at_release() {
    let Some(e) = Env::new("budget") else { return };
    new_world(&e);
    e.ok(&["key", "new", "pay", "--spend", "1"]);
    e.ok(&["key", "new", "small", "--within", "pay", "--spend", "0.50"]);
    let wide = e.arbor(&["key", "new", "wide", "--within", "pay", "--spend", "2"]);
    assert_eq!(wide.status.code(), Some(3));

    // Under a budget, an effect says what it costs.
    let unsaid = inside(&e, Some("small"), "arbor effect -- true");
    assert_eq!(unsaid.status.code(), Some(2));
    assert!(
        text(&unsaid.stderr).contains("key 'small' may spend $0.50 in all, so say what this costs"),
        "{}",
        text(&unsaid.stderr)
    );
    assert_eq!(ask(&e, Some("small"), "--cost 0.40 -- true"), 1);
    assert_eq!(ask(&e, Some("small"), "--cost 0.20 -- true"), 2);
    assert_eq!(ask(&e, Some("pay"), "--cost 0.70 -- true"), 3);
    assert_eq!(ask(&e, Some("pay"), "--cost 0.60 -- true"), 4);
    e.ok(&["release", "1"]);
    // Then small has $0.10 left, and pay, which counts what small spends, $0.60.
    for (id, why) in [
        (
            "2",
            "effect 2 costs $0.20, and key 'small' has $0.10 left of $0.50",
        ),
        (
            "3",
            "effect 3 costs $0.70, and key 'pay' has $0.60 left of $1.00",
        ),
    ] {
        let over = e.arbor(&["release", id]);
        assert_eq!(over.status.code(), Some(3));
        assert!(text(&over.stderr).contains(why), "{}", text(&over.stderr));
    }
    e.ok(&["release", "4"]);
    let keys = e.ok(&["keys"]);
    let row = |name: &str| {
        keys.lines()
            .find(|l| l.starts_with(&format!("{name} ")))
            .unwrap()
            .to_string()
    };
    assert!(row("pay").contains("  $1.00 of $1.00  "), "{keys}");
    assert!(row("small").contains("  $0.40 of $0.50  "), "{keys}");

    // Taking a key back refuses what still waits under it, and under the keys made within it.
    assert_eq!(
        e.ok(&["key", "take-back", "pay"]),
        "took back key pay, and key small made within it; stopped 0 commands; refused 2 effects\n"
    );
    assert!(
        e.ok(&["outbox", "2"])
            .contains("  refused key pay was taken back\n")
    );
    let steps = steps(&e);
    assert_eq!(
        steps[steps.len() - 3..],
        ["take-back you ok", "refuse you ok #2", "refuse you ok #3"]
    );
}

#[test]
fn throwing_a_world_away_refuses_what_it_asked_for() {
    let Some(e) = Env::new("discard") else { return };
    new_world(&e);
    assert_eq!(ask(&e, None, "-- touch never"), 1);
    let out = e.ok(&["discard", "demo"]);
    assert!(
        out.ends_with(" is as it was\nrefused the 1 effect it asked for\n"),
        "{out}"
    );
    let late = e.arbor(&["release", "1"]);
    assert_eq!(late.status.code(), Some(3));
    assert!(
        text(&late.stderr).contains("effect 1 was refused at "),
        "{}",
        text(&late.stderr)
    );
    assert!(
        e.ok(&["outbox", "1"])
            .contains("  refused world demo was thrown away\n")
    );
    assert!(!e.base.join("never").exists());
    let steps = steps(&e);
    assert_eq!(
        steps[steps.len() - 3..],
        [
            "discard you ok",
            "refuse you ok #1",
            "release you refused #1"
        ]
    );
}

#[test]
fn inside_a_world_arbor_only_asks() {
    let Some(e) = Env::new("guard") else { return };
    new_world(&e);
    assert_eq!(ask(&e, None, "-- touch released"), 1);
    // The arbor in a world refuses anything but asking. Without its socket it finds no outbox at all: Arbor's
    // state is not in the world.
    let out = inside(
        &e,
        None,
        "arbor release 1; echo $?; arbor worlds; echo $?; ARBOR_OUTBOX= arbor release 1; echo $?",
    );
    assert_eq!(text(&out.stdout), "3\n3\n2\n", "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("inside a world, arbor only asks for effects"),
        "{}",
        text(&out.stderr)
    );
    assert!(!e.base.join("released").exists());
    assert!(e.ok(&["outbox"]).contains("\n1   waiting  "));
    // Outside a world there is no one to ask: the person runs the command.
    assert_eq!(e.arbor(&["effect", "--", "true"]).status.code(), Some(2));
}

#[test]
fn what_a_world_asks_for_is_shown_so_that_it_cannot_act_on_the_terminal() {
    let Some(e) = Env::new("shown") else { return };
    new_world(&e);
    // An escape sequence in the command, and one in the reason with a line made to look like arbor's own.
    let id = ask(
        &e,
        None,
        r#"--why "$(printf 'fine\033[2J\n  cost    $0.00')" -- printf "$(printf 'x\033]0;owned\007')""#,
    );
    assert_eq!(id, 1);
    let list = e.ok(&["outbox"]);
    assert!(list.ends_with("  printf $'x\\e]0;owned\\x07'\n"), "{list}");
    let about = e.ok(&["outbox", "1"]);
    assert!(about.contains("fine\\e[2J\\n  cost    $0.00\n"), "{about}");
    let ledger = e.ok(&["ledger"]);
    for shown in [&list, &about, &ledger] {
        assert!(
            !shown.contains(['\x1b', '\x07']),
            "{shown:?} acts on a terminal"
        );
    }
}

#[test]
fn an_effect_s_input_is_capped_and_one_that_cannot_start_waits_again() {
    let Some(e) = Env::new("input") else { return };
    new_world(&e);
    let big = inside(
        &e,
        None,
        "head -c 1048577 /dev/zero | arbor effect --input - -- wc -c",
    );
    assert_eq!(big.status.code(), Some(2));
    assert!(
        text(&big.stderr).contains("the input is larger than 1MiB"),
        "{}",
        text(&big.stderr)
    );
    let out = inside(
        &e,
        None,
        "head -c 1048576 /dev/zero | arbor effect --input - -- wc -c",
    );
    assert_eq!(
        text(&out.stdout),
        "effect 1 waiting\n",
        "{}",
        text(&out.stderr)
    );
    assert_eq!(text(&e.arbor(&["release", "1"]).stdout), "1048576\n");

    // A command that cannot start leaves its effect waiting, for a person to refuse.
    assert_eq!(ask(&e, None, "-- no-such-command-here"), 2);
    let failed = e.arbor(&["release", "2"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        text(&failed.stderr).contains("cannot run no-such-command-here"),
        "{}",
        text(&failed.stderr)
    );
    assert!(e.ok(&["outbox"]).contains("\n2   waiting  "));
    e.ok(&["refuse", "2"]);
}

#[test]
fn an_effect_whose_arbor_stopped_while_it_ran_never_runs_again() {
    let Some(e) = Env::new("crash") else { return };
    new_world(&e);
    assert_eq!(ask(&e, None, "-- sh -c 'echo $$ > pid; exec sleep 30'"), 1);
    let mut release = e
        .command(&["release", "1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for(&e.base.join("pid"));
    assert!(e.ok(&["outbox"]).contains("\n1   running  "));
    release.kill().unwrap();
    release.wait().unwrap();
    // Its command may still run, or may have done anything: no one knows, so it is not run again.
    assert!(e.ok(&["outbox"]).contains("\n1   unknown  "));
    let again = e.arbor(&["release", "1"]);
    assert_eq!(again.status.code(), Some(3));
    assert!(
        text(&again.stderr).contains("effect 1 was released at "),
        "{}",
        text(&again.stderr)
    );
    assert!(
        e.ok(&["outbox", "1"])
            .contains("arbor stopped while it ran, so what it did is not known")
    );
    let pid: i32 = e.read("pid").unwrap().trim().parse().unwrap();
    unsafe { libc::kill(pid, libc::SIGKILL) };
}
