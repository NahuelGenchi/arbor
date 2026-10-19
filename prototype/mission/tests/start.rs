//! Start an agent, without the window: `agents::start` makes the try and the key and runs the agent in a room of the
//! try, as Start does, with `env` in place of a terminal. The agent is a made-up program where OpenCode's installer
//! puts it, in a made-up home folder, that leaves a file in its folder. It needs the `arbor` built next to the tests,
//! as `answer.rs` does, and bubblewrap for the room: without it, only the try and the key are checked, unless
//! ARBOR_REQUIRE_BWRAP is set, as in CI.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use arbor::key::Key;
use arbor::tree;
use arbor::world::World;
use mission::{act, agents, words};

#[test]
fn start_makes_a_try_and_a_key_for_it_alone() {
    if !act::arbor().is_absolute() {
        assert!(
            std::env::var_os("ARBOR_REQUIRE_GTK").is_none(),
            "no arbor next to the tests, and ARBOR_REQUIRE_GTK is set"
        );
        return eprintln!("skipped: no arbor next to the tests (cargo test --workspace builds it)");
    }
    let root = std::env::temp_dir().join(format!("mission-start-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let home = root.join("home");
    let folder = root.join("letters");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("thanks.txt"), "thank you\n").unwrap();
    let program = home.join(".opencode/bin/opencode");
    fs::create_dir_all(program.parent().unwrap()).unwrap();
    fs::write(&program, "#!/bin/sh\necho drafted > draft.txt\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: this is the only test in this file, so nothing else reads the environment meanwhile.
    unsafe {
        std::env::set_var("ARBOR_HOME", root.join("state"));
        std::env::set_var("HOME", &home);
    }
    let home = fs::canonicalize(&home).unwrap();
    let found = agents::found(Some(&home), None);
    assert_eq!(found.len(), 1);
    let h = &found[0];

    let name = agents::next_name(&folder);
    assert_eq!(name, "letters");
    let s = agents::start(&name, &folder, h, &["env".to_string()]);
    assert_eq!(s.failed, None);
    let line = words::started(&s, 0);
    assert_eq!(
        line.title,
        "Started OpenCode in try letters: it works in a terminal window"
    );

    // The key opens that try alone, reads only the agent's own folder, reaches only its hosts, and has a home.
    let k = Key::load("letters").unwrap();
    assert_eq!(k.worlds, ["letters"]);
    assert_eq!(k.read, [home.join(".opencode/bin")]);
    assert_eq!(k.home.as_deref(), Some("letters"));
    let mut hosts: Vec<String> = k.net.iter().map(|h| h.to_string()).collect();
    hosts.sort();
    assert_eq!(hosts, ["models.opencode.ai", "opencode.ai"]);
    let hours = (k.until_ns - k.made_ns) as f64 / 3.6e12;
    assert!((hours - agents::HOURS as f64).abs() < 0.01, "{hours} h");
    // The next start of the same folder gets a try of its own.
    assert_eq!(agents::next_name(&folder), "letters-2");

    // The agent ran in a room of the try: what it wrote is in the try, not in the folder.
    if std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_err()
    {
        assert!(
            std::env::var_os("ARBOR_REQUIRE_BWRAP").is_none(),
            "no bwrap, and ARBOR_REQUIRE_BWRAP is set"
        );
        eprintln!("skipped the room: no bwrap");
    } else {
        let w = World::load("letters").unwrap();
        let limit = Instant::now() + Duration::from_secs(20);
        let changed = loop {
            let c = tree::entries(&w)
                .and_then(|e| tree::changes(&w, &e))
                .unwrap();
            if !c.is_empty() || Instant::now() > limit {
                break c;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let paths: Vec<&str> = changed.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["draft.txt"]);
        assert!(!folder.join("draft.txt").exists());
    }
    act::run(&["key", "take-back", "letters"]);
    let _ = fs::remove_dir_all(&root);
}
