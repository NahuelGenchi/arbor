//! The window's Release and Refuse, without the window: `act::answer` runs `arbor release` and `arbor refuse` as the
//! buttons do, and reads the effect back. It needs the `arbor` built next to the tests (`cargo test --workspace`
//! builds it, as `../check.sh` does); without it, it says so and checks nothing, unless ARBOR_REQUIRE_GTK is set, as
//! in CI. No cell is started, so it needs no bubblewrap: the effects are put in the outbox directly.

use std::fs;
use std::path::Path;

use arbor::ledger::Ledger;
use arbor::outbox::{Effect, Outbox, State};
use mission::act::{self, Ask};
use mission::words;

fn effect(base: &Path, cmd: &[&str]) -> Effect {
    let e = serde_json::json!({
        "id": 0, "world": "shop", "cell": "0a1b2c3d", "base": base, "cmd": cmd,
        "why": "publish the fix", "asked_ns": 0, "state": "waiting",
    });
    serde_json::from_value(e).unwrap()
}

#[test]
fn release_and_refuse_run_what_arbor_runs() {
    if !act::arbor().is_absolute() {
        assert!(
            std::env::var_os("ARBOR_REQUIRE_GTK").is_none(),
            "no arbor next to the tests, and ARBOR_REQUIRE_GTK is set"
        );
        return eprintln!("skipped: no arbor next to the tests (cargo test --workspace builds it)");
    }
    let root = std::env::temp_dir().join(format!("mission-answer-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let base = root.join("shop");
    fs::create_dir_all(&base).unwrap();
    // SAFETY: this is the only test in this file, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("ARBOR_HOME", root.join("state")) };
    let outbox = Outbox::open().unwrap();
    let add = |cmd: &[&str]| outbox.add(effect(&base, cmd), b"").unwrap();
    let published = add(&["sh", "-c", "echo published; echo here > out.txt"]);
    let refused = add(&["sh", "-c", "echo never > never.txt"]);
    let failing = add(&["sh", "-c", "echo oops; exit 3"]);

    // Released: it ran once, in the real folder, and its receipt and output are read back.
    let a = act::answer(published.id, &Ask::Release);
    assert_eq!(a.failed, None);
    let e = a.effect.clone().unwrap();
    assert_eq!(
        (e.state, e.receipt.as_ref().map(|r| r.exit)),
        (State::Done, Some(0))
    );
    assert_eq!(a.output.as_deref(), Some("published"));
    assert_eq!(fs::read_to_string(base.join("out.txt")).unwrap(), "here\n");
    let line = words::answered(&published, &a, 0);
    assert_eq!(line.title, "Released: it finished");
    assert_eq!(
        words::answer_news(&line),
        "Released sh -c 'echo published; echo here > out.txt': it finished"
    );

    // Refused, with a reason: it never ran, and the reason is kept.
    let a = act::answer(refused.id, &Ask::Refuse("not yet".into()));
    let e = a.effect.clone().unwrap();
    assert_eq!((e.state, e.refusal.as_str()), (State::Refused, "not yet"));
    assert!(!base.join("never.txt").exists());
    let line = words::answered(&refused, &a, 0);
    assert_eq!(
        (line.title.as_str(), line.note.as_deref()),
        ("Refused: it did not run", Some("Your reason: “not yet”"))
    );

    // An answer arbor will not give, in its own words: an effect runs only once.
    let again = act::answer(refused.id, &Ask::Release);
    assert!(
        again
            .failed
            .as_deref()
            .is_some_and(|f| f.contains("was refused at")),
        "{:?}",
        again.failed
    );
    assert_eq!(
        words::answered(&refused, &again, 0).mark,
        words::Mark::Failed
    );

    // A command that fails is still released once: its receipt says how it ended.
    let a = act::answer(failing.id, &Ask::Release);
    assert_eq!(a.failed, None);
    let line = words::answered(&failing, &a, 0);
    assert_eq!(line.title, "Released: it failed with exit code 3");
    assert_eq!(line.output.as_deref(), Some("oops"));

    // The ledger has each answer as a step of yours, as when the person types them.
    let ledger = fs::read_to_string(Ledger::open().unwrap().path()).unwrap();
    let acts: Vec<(String, String)> = ledger
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .map(|s| (format!("{} {}", s["act"], s["out"]), s["by"].to_string()))
        .map(|(a, b)| (a.replace('"', ""), b.replace('"', "")))
        .collect();
    let mine = |a: &str| acts.iter().any(|(x, by)| x == a && by == "you");
    for step in [
        "release running",
        "release exit 0",
        "refuse ok",
        "release refused",
        "release exit 3",
    ] {
        assert!(mine(step), "no step '{step}' in {acts:?}");
    }
    fs::remove_dir_all(&root).unwrap();
}
