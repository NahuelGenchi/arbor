//! The window's See changes, Keep and Undo the keep, without the window: `act::look` reads a try's changes as the
//! view shows them, and `act::keep_or_undo` runs `arbor keep` and `arbor undo` as the buttons do. It needs the
//! `arbor` built next to the tests, as `answer.rs` does. No cell is started, so it needs no bubblewrap: the try's
//! changes are written into its folder of changes directly, as a command in it would leave them.

use std::fs;
use std::time::Duration;

use arbor::world::{State, World};
use mission::act::{self, Do};
use mission::words;

#[test]
fn keep_and_undo_run_what_arbor_runs() {
    if !act::arbor().is_absolute() {
        assert!(
            std::env::var_os("ARBOR_REQUIRE_GTK").is_none(),
            "no arbor next to the tests, and ARBOR_REQUIRE_GTK is set"
        );
        return eprintln!("skipped: no arbor next to the tests (cargo test --workspace builds it)");
    }
    let root = std::env::temp_dir().join(format!("mission-keep-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let base = root.join("letters");
    fs::create_dir_all(&base).unwrap();
    fs::write(base.join("thanks.txt"), "thank you\n").unwrap();
    // SAFETY: this is the only test in this file, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("ARBOR_HOME", root.join("state")) };
    let w = World::create(Some("letters"), &base).unwrap();
    fs::write(w.up().join("thanks.txt"), "thank you, all\n").unwrap();
    fs::create_dir_all(w.up().join("drafts")).unwrap();
    fs::write(w.up().join("drafts/seeds.txt"), "for the seeds\n").unwrap();

    // What the view shows before a keep: each file in plain words, by folder, with its lines.
    let look = act::look("letters");
    assert_eq!((look.failed.as_deref(), look.conflicts.len()), (None, 0));
    let view = words::changes("letters", &base, &look);
    assert_eq!(view.note, None);
    let shown: Vec<(String, Vec<(String, String)>)> = view
        .groups
        .iter()
        .map(|(d, f)| {
            let f = f.iter().map(|f| (f.word.clone(), f.name.clone()));
            (d.clone(), f.collect())
        })
        .collect();
    let top = format!("at the top of {}", base.display());
    assert_eq!(
        shown,
        [
            (top, vec![("changed".into(), "thanks.txt".into())]),
            (
                "in drafts/".into(),
                vec![("added".into(), "seeds.txt".into())]
            ),
        ]
    );
    let patch = view.groups[0].1[0].patch.as_deref().unwrap();
    assert_eq!(patch, "-thank you\n+thank you, all");

    // Kept: the changes are in the real folder, and the line says so.
    let kept = act::keep_or_undo("letters", Do::Keep);
    assert_eq!((kept.failed.as_deref(), kept.counts), (None, [1, 1, 0]));
    assert_eq!(
        fs::read_to_string(base.join("thanks.txt")).unwrap(),
        "thank you, all\n"
    );
    let line = words::kept(&kept, 0);
    assert_eq!(
        (line.mark, line.detail.as_str()),
        (words::Mark::Done, "2 files: 1 added, 1 changed")
    );
    assert_eq!(World::load("letters").unwrap().state, State::Kept);

    // Undone: the real folder is as it was, and the try is open again.
    let undone = act::keep_or_undo("letters", Do::Undo);
    assert_eq!(undone.failed, None);
    assert_eq!(
        fs::read_to_string(base.join("thanks.txt")).unwrap(),
        "thank you\n"
    );
    assert!(!base.join("drafts").exists());
    assert!(words::kept(&undone, 0).title.starts_with("Undid the keep"));

    // The real folder changes meanwhile: the view says a keep would be refused, and arbor refuses it.
    std::thread::sleep(Duration::from_millis(20));
    fs::write(base.join("thanks.txt"), "thank you so much\n").unwrap();
    let look = act::look("letters");
    assert_eq!(look.conflicts.len(), 1, "{:?}", look.conflicts);
    let view = words::changes("letters", &base, &look);
    assert!(
        view.note
            .as_deref()
            .is_some_and(|n| n.contains("thanks.txt (changed)")),
        "{:?}",
        view.note
    );
    let refused = act::keep_or_undo("letters", Do::Keep);
    assert!(refused.failed.is_some());
    assert_eq!(refused.problems.len(), 1, "{:?}", refused.problems);
    let line = words::kept(&refused, 0);
    assert_eq!(line.mark, words::Mark::Failed);
    assert_eq!(
        line.title,
        "Not kept: your own files changed since try letters began"
    );
    assert!(
        line.detail.contains("thanks.txt (changed)"),
        "{}",
        line.detail
    );
    assert_eq!(
        fs::read_to_string(base.join("thanks.txt")).unwrap(),
        "thank you so much\n"
    );
    fs::remove_dir_all(&root).unwrap();
}
