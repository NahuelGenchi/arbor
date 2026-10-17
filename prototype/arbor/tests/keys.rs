//! Keys end to end: a cell sees only its world until a key opens more, for a while and within limits, password files
//! and agents' records of your sessions stay hidden even then, also when their programs replace or make them while a
//! command runs, a key's own home folder keeps what its commands write there from one run to the next, a key taken
//! back or ended stops what runs under it, and a renewed one lasts longer, home folder and running commands included.
//! They skip like `common` says.

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

/// Writes a script at `path` that may run.
fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A `PATH` of `dirs`, then `/usr/bin` and `/bin`, and the folder bubblewrap is in, since arbor finds it on this
/// `PATH` too (CI builds it into /usr/local/bin).
fn path_of(dirs: &[PathBuf]) -> std::ffi::OsString {
    let bwrap = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .find(|d| d.join("bwrap").is_file())
        .expect("Env::new ran bwrap from PATH");
    let mut all = dirs.to_vec();
    all.extend(["/usr/bin", "/bin"].map(PathBuf::from));
    if !all.contains(&bwrap) {
        all.push(bwrap);
    }
    std::env::join_paths(all).unwrap()
}

/// Where AI agents keep their logins, in the home folder, each with what a command reads there while it is hidden:
/// nothing from a hidden file, and `-` in a hidden folder, which looks empty.
const AGENT_LOGINS: [(&str, &str); 41] = [
    (".local/share/opencode/opencode.db", "-"),
    (".local/share/opencode/opencode.db-wal", "-"),
    (".local/share/opencode/opencode.db-shm", "-"),
    (".local/share/opencode/auth.json", "-"),
    (".local/share/opencode/mcp-auth.json", "-"),
    (".claude/.credentials.json", ""),
    (".claude.json", ""),
    (".claude/backups/.claude.json.backup.1", "-"),
    (".config/anthropic/credentials/default.json", "-"),
    (".codex/auth.json", ""),
    (".codex/.credentials.json", ""),
    (".codex/secrets/codex_auth.age", "-"),
    (".gemini/oauth_creds.json", "-"),
    (".gemini/cli-browser-profile/Default/Cookies", "-"),
    (".qwen/settings.json", "-"),
    (".config/github-copilot/auth.db", "-"),
    (".local/share/amp/secrets.json", "-"),
    (".config/goose/secrets.yaml", "-"),
    (".local/share/crush/crush.json", "-"),
    (".factory/auth.v2.file", "-"),
    (".cline/data/settings/providers.json", "-"),
    (".local/share/kilo/auth.json", "-"),
    (".openhands/agent_settings.json", "-"),
    (".local/share/amazon-q/data.sqlite3", "-"),
    (".config/Cursor/User/globalStorage/state.vscdb", "-"),
    (".kimi-code/credentials/kimi-code.json", "-"),
    (".kimi-code/config.toml", ""),
    (".kimi/credentials/kimi-code.json", "-"),
    (".kimi/config.toml", ""),
    (".copilot/config.json", ""),
    (".copilot/mcp-oauth-config/server.json", "-"),
    (".copilot/mcp-secrets/server.json", "-"),
    (".config/cursor/auth.json", ""),
    (".cursor/projects/shop/mcp-auth.json", "-"),
    (".aider/oauth-keys.env", ""),
    (".aider.conf.yml", ""),
    (".augment/session.json", ""),
    (".pi/agent/auth.json", ""),
    (".env", ""),
    (".vibe/.env", ""),
    (".continue/.env", ""),
];

/// The folders among them that stay hidden whole.
const LOGIN_FOLDERS: [&str; 22] = [
    ".local/share/opencode",
    ".claude/backups",
    ".config/anthropic/credentials",
    ".codex/secrets",
    ".gemini",
    ".gemini/cli-browser-profile",
    ".qwen",
    ".config/github-copilot",
    ".local/share/amp",
    ".config/goose",
    ".local/share/crush",
    ".factory",
    ".cline/data",
    ".local/share/kilo",
    ".openhands",
    ".local/share/amazon-q",
    ".config/Cursor",
    ".kimi-code/credentials",
    ".kimi/credentials",
    ".copilot/mcp-oauth-config",
    ".copilot/mcp-secrets",
    ".cursor/projects",
];

/// Files next to them that hold no login.
const NEXT_TO_LOGINS: [&str; 13] = [
    ".local/share/notes",
    ".claude/settings.json",
    ".config/anthropic/configs/default.json",
    ".codex/config.toml",
    ".copilot/settings.json",
    ".cursor/cli-config.json",
    ".cline/rules/style.md",
    ".kimi-code/tui.toml",
    ".aider/analytics.json",
    ".augment/settings.json",
    ".pi/agent/settings.json",
    ".vibe/config.toml",
    ".continue/config.yaml",
];

/// Where AI agents keep their records of your sessions, in the home folder, each with what a command reads there
/// while it is hidden: nothing from a hidden file, and `-` in a hidden folder, which looks empty, and where an agent's
/// database was, which is missing. Codex's databases have made-up version numbers, since the list takes any. Aider's
/// are in the home folder, as when it runs there.
const AGENT_RECORDS: [(&str, &str); 41] = [
    (".claude/projects/-home-you-work-shop/0c1d.jsonl", "-"),
    (".claude/projects/-home-you-work-shop/memory/MEMORY.md", "-"),
    (".claude/history.jsonl", ""),
    (".claude/file-history/0c1d/env", "-"),
    (".claude/paste-cache/paste", "-"),
    (".claude/image-cache/0c1d/image", "-"),
    (".claude/uploads/0c1d/upload", "-"),
    (".claude/plans/plan.md", "-"),
    (".claude/tasks/list/1.json", "-"),
    (".claude/todos/0c1d.json", "-"),
    (".claude/debug/0c1d.txt", "-"),
    (".claude/feedback-bundles/bundle", "-"),
    (".claude/feedback/drafts/draft", "-"),
    (".claude/usage-data/report.html", "-"),
    (".claude/jobs/0c1d/state.json", "-"),
    (".claude/agent-memory/reviewer/MEMORY.md", "-"),
    (".codex/sessions/2026/10/17/rollout.jsonl", "-"),
    (".codex/archived_sessions/rollout.jsonl", "-"),
    (".codex/history.jsonl", ""),
    (".codex/session_index.jsonl", ""),
    (".codex/log/codex-tui.log", "-"),
    (".codex/memories/memory_summary.md", "-"),
    (".codex/memories_v2/memory.md", "-"),
    (".codex/memories_extensions/memory.md", "-"),
    (".codex/state_7.sqlite", "-"),
    (".codex/state_7.sqlite-wal", "-"),
    (".codex/thread_history_3.sqlite", "-"),
    (".local/state/opencode/prompt-history.jsonl", "-"),
    (".gemini/tmp/shop/chats/session-0c1d.jsonl", "-"),
    (".gemini/tmp/shop/logs.json", "-"),
    (".gemini/history/shop/.git/HEAD", "-"),
    (".copilot/session-state/0c1d/events.jsonl", "-"),
    (".copilot/session-store.db", "-"),
    (".copilot/session-store.db-wal", "-"),
    (".copilot/command-history-state/history.json", "-"),
    (".copilot/command-history-state.json", ""),
    (".copilot/logs/process-1-2.log", "-"),
    (".copilot/history-session-state/session_0c1d_1.json", "-"),
    (".aider.chat.history.md", ""),
    (".aider.input.history", ""),
    (".aider.llm.history", ""),
];

/// Files next to them that hold none: the settings, instructions and skills a broad grant is for.
const NEXT_TO_RECORDS: [&str; 9] = [
    ".claude/settings.json",
    ".claude/CLAUDE.md",
    ".claude/skills/review/SKILL.md",
    ".codex/config.toml",
    ".codex/AGENTS.md",
    ".local/state/notes",
    ".copilot/settings.json",
    ".copilot/copilot-instructions.md",
    ".aider.model.settings.yml",
];

/// Writes `text` in each of `files` in the home folder `home`.
fn put(home: &Path, files: &[&str], text: &str) {
    for rel in files {
        let path = home.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

/// Writes a made-up login, `login`, where each agent keeps one in `home`, and `other` in the files next to them.
fn made_up_logins(home: &Path) {
    put(home, &AGENT_LOGINS.map(|(f, _)| f), "login");
    put(home, &NEXT_TO_LOGINS, "other");
}

/// Writes a made-up record, `record`, where each agent keeps one in `home`, and `other` in the files next to them.
fn made_up_records(home: &Path) {
    put(home, &AGENT_RECORDS.map(|(f, _)| f), "record");
    put(home, &NEXT_TO_RECORDS, "other");
}

/// Makes the key `name`, which opens each of `opens` in the home folder.
fn key_opening(e: &Env, name: &str, opens: &[&str]) {
    let h = e.user_home.to_str().unwrap();
    let paths: Vec<String> = opens.iter().map(|o| format!("{h}/{o}")).collect();
    let mut args = vec!["key", "new", name];
    for p in &paths {
        args.extend(["--read", p.as_str()]);
    }
    e.ok(&args);
}

/// What a command in `world`, under `key` if given, reads in each of `files` in the home folder: its text, nothing
/// from a hidden file, or `-` where there is none, as in a hidden folder.
fn reads(e: &Env, world: &str, key: Option<&str>, files: &[&str]) -> Vec<String> {
    let script = files
        .iter()
        .map(|f| format!("cat ~/{f} 2>/dev/null || printf -; echo"))
        .collect::<Vec<_>>()
        .join("; ");
    let mut args = vec!["run", world];
    if let Some(k) = key {
        args.extend(["--key", k]);
    }
    args.extend(["--", "sh", "-c", &script]);
    e.ok(&args).lines().map(str::to_string).collect()
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
fn agents_logins_stay_hidden_in_what_a_world_or_key_opens_unless_a_key_names_them() {
    let Some(e) = Env::new("agentlogins") else {
        return;
    };
    new_world(&e);
    let home = &e.user_home;
    made_up_logins(home);
    let h = home.to_str().unwrap();
    let key = |name: &str, opens: &[&str]| key_opening(&e, name, opens);
    let logins = AGENT_LOGINS.map(|(f, _)| f);
    let every: Vec<&str> = logins.iter().chain(&NEXT_TO_LOGINS).copied().collect();
    // The hidden folders look empty and the hidden files read empty, while the files next to them show.
    let hidden: Vec<&str> = AGENT_LOGINS
        .iter()
        .map(|(_, seen)| *seen)
        .chain(NEXT_TO_LOGINS.map(|_| "other"))
        .collect();

    // A key that opens the whole home folder hides them...
    e.ok(&["key", "new", "all", "--read", h]);
    assert_eq!(reads(&e, "demo", Some("all"), &every), hidden);
    // ...as does one that opens the folders they are in, short of the home folder, for those in them...
    let folders = [
        ".local/share",
        ".config",
        ".claude",
        ".codex",
        ".copilot",
        ".cursor",
        ".cline",
        ".kimi-code",
        ".kimi",
        ".aider",
        ".augment",
        ".pi",
        ".vibe",
        ".continue",
    ];
    key("folders", &folders);
    let (inside, seen): (Vec<&str>, Vec<&str>) = every
        .iter()
        .zip(&hidden)
        .filter(|(f, _)| folders.iter().any(|d| Path::new(f).starts_with(d)))
        .map(|(f, r)| (*f, *r))
        .unzip();
    assert_eq!(reads(&e, "demo", Some("folders"), &inside), seen);
    // ...and a world of the home folder, where writes to them vanish and leave nothing in the world.
    e.ok(&["new", "home", "--from", h]);
    assert_eq!(reads(&e, "home", None, &every), hidden);
    let writes = logins
        .map(|f| {
            let dir = Path::new(f).parent().unwrap().display();
            format!("mkdir -p ~/{dir} && echo x > ~/{f}")
        })
        .join("; ");
    e.ok(&["run", "home", "--", "sh", "-c", &writes]);
    assert_eq!(e.ok(&["diff", "home"]), "no changes\n");
    for f in logins {
        assert_eq!(fs::read_to_string(home.join(f)).unwrap(), "login", "{f}");
    }

    // A key that names a login opens it, while the rest of the home folder stays hidden...
    key("logins", &logins);
    let mut named = vec!["login"; logins.len()];
    named.extend(["-"; NEXT_TO_LOGINS.len()]);
    assert_eq!(reads(&e, "demo", Some("logins"), &every), named);
    // ...as does one that names the folders hidden whole...
    key("whole", &LOGIN_FOLDERS);
    let in_folders: Vec<&str> = AGENT_LOGINS
        .iter()
        .filter(|(_, seen)| *seen == "-")
        .map(|(f, _)| *f)
        .collect();
    assert_eq!(
        reads(&e, "demo", Some("whole"), &in_folders),
        vec!["login"; in_folders.len()]
    );
    // ...while one that names Gemini CLI's folder alone opens its login but not its browser agent's profile...
    key("gemini", &[".gemini"]);
    assert_eq!(
        reads(
            &e,
            "demo",
            Some("gemini"),
            &[
                ".gemini/oauth_creds.json",
                ".gemini/cli-browser-profile/Default/Cookies"
            ]
        ),
        ["login", "-"]
    );
    // ...and one that names a login inside a folder it opens shows that login, while the rest of OpenCode's folder
    // stays hidden.
    key(
        "within",
        &[
            ".local/share",
            ".local/share/opencode/opencode.db",
            ".claude",
            ".claude/.credentials.json",
        ],
    );
    let files = [
        ".local/share/notes",
        ".local/share/opencode/opencode.db",
        ".local/share/opencode/opencode.db-wal",
        ".local/share/opencode/auth.json",
        ".claude/settings.json",
        ".claude/.credentials.json",
    ];
    assert_eq!(
        reads(&e, "demo", Some("within"), &files),
        ["other", "login", "-", "-", "other", "login"]
    );
}

#[test]
fn agents_records_stay_hidden_in_what_a_world_or_key_opens_unless_a_key_names_them() {
    let Some(e) = Env::new("agentrecords") else {
        return;
    };
    new_world(&e);
    let home = &e.user_home;
    made_up_records(home);
    // Gemini CLI's settings, in its folder that stays hidden whole with its logins.
    put(home, &[".gemini/settings.json"], "other");
    let h = home.to_str().unwrap();
    let key = |name: &str, opens: &[&str]| key_opening(&e, name, opens);
    let records = AGENT_RECORDS.map(|(f, _)| f);
    let every: Vec<&str> = records.iter().chain(&NEXT_TO_RECORDS).copied().collect();
    // The hidden folders look empty, the hidden files read empty and the databases are missing, while the settings
    // next to them show.
    let hidden: Vec<&str> = AGENT_RECORDS
        .iter()
        .map(|(_, seen)| *seen)
        .chain(NEXT_TO_RECORDS.map(|_| "other"))
        .collect();

    // A key that opens the whole home folder hides them...
    e.ok(&["key", "new", "all", "--read", h]);
    assert_eq!(reads(&e, "demo", Some("all"), &every), hidden);
    // ...as does one that opens the folders they are in, short of the home folder, for those in them, Gemini CLI's
    // too, which this key opens by name...
    let folders = [".claude", ".codex", ".local/state", ".gemini", ".copilot"];
    key("folders", &folders);
    let (inside, seen): (Vec<&str>, Vec<&str>) = every
        .iter()
        .zip(&hidden)
        .filter(|(f, _)| folders.iter().any(|d| Path::new(f).starts_with(d)))
        .map(|(f, r)| (*f, *r))
        .unzip();
    assert_eq!(reads(&e, "demo", Some("folders"), &inside), seen);
    // ...and a world of the home folder, where writes to them vanish and leave nothing in the world.
    e.ok(&["new", "home", "--from", h]);
    assert_eq!(reads(&e, "home", None, &every), hidden);
    let writes = records
        .map(|f| format!("mkdir -p $(dirname ~/{f}) && echo x > ~/{f}"))
        .join(" && ");
    e.ok(&["run", "home", "--", "sh", "-c", &writes]);
    assert_eq!(e.ok(&["diff", "home"]), "no changes\n");
    for f in records {
        assert_eq!(fs::read_to_string(home.join(f)).unwrap(), "record", "{f}");
    }

    // A key that names a record opens it...
    key(
        "named",
        &[
            ".claude/projects",
            ".codex/history.jsonl",
            ".codex/state_7.sqlite",
            ".local/state/opencode",
            ".gemini/tmp/shop",
            ".copilot/session-state",
            ".copilot/session-store.db",
            ".aider.chat.history.md",
        ],
    );
    let files = [
        ".claude/projects/-home-you-work-shop/0c1d.jsonl",
        ".codex/history.jsonl",
        ".codex/state_7.sqlite",
        ".codex/state_7.sqlite-wal",
        ".local/state/opencode/prompt-history.jsonl",
        ".gemini/tmp/shop/logs.json",
        ".gemini/history/shop/.git/HEAD",
        ".copilot/session-state/0c1d/events.jsonl",
        ".copilot/session-store.db",
        ".copilot/session-store.db-wal",
        ".aider.chat.history.md",
    ];
    assert_eq!(
        reads(&e, "demo", Some("named"), &files),
        [
            "record", "record", "record", "-", "record", "record", "-", "record", "record", "-",
            "record"
        ]
    );
    // ...also inside a folder the key opens, where the other records stay hidden.
    key(
        "within",
        &[
            ".claude",
            ".claude/projects/-home-you-work-shop/memory",
            ".codex",
            ".codex/state_7.sqlite",
            ".gemini",
            ".gemini/tmp/shop",
        ],
    );
    let files = [
        ".claude/settings.json",
        ".claude/projects/-home-you-work-shop/memory/MEMORY.md",
        ".claude/projects/-home-you-work-shop/0c1d.jsonl",
        ".codex/state_7.sqlite",
        ".codex/state_7.sqlite-wal",
        ".gemini/settings.json",
        ".gemini/tmp/shop/chats/session-0c1d.jsonl",
        ".gemini/history/shop/.git/HEAD",
    ];
    assert_eq!(
        reads(&e, "demo", Some("within"), &files),
        [
            "other", "record", "-", "record", "-", "other", "record", "-"
        ]
    );
}

/// Where AI agents keep their records of your sessions in a project's folder, as Aider and Crush do, each with what a
/// command reads there while it is hidden: nothing from a hidden file, and `-` in Crush's folder, which looks empty
/// with the settings and commands in it.
const PROJECT_RECORDS: [(&str, &str); 6] = [
    (".aider.chat.history.md", ""),
    (".aider.input.history", ""),
    (".aider.llm.history", ""),
    (".crush/crush.db", "-"),
    (".crush/logs/crush.log", "-"),
    (".crush/commands/review.md", "-"),
];

/// Writes a made-up record, `record`, where each agent keeps one in the folder `dir`, and `other` in a file next to
/// them, and returns their paths, the records first.
fn made_up_project(dir: &Path) -> Vec<PathBuf> {
    let files: Vec<PathBuf> = PROJECT_RECORDS
        .iter()
        .map(|(f, _)| dir.join(f))
        .chain([dir.join("notes.md")])
        .collect();
    for (i, f) in files.iter().enumerate() {
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(
            f,
            if i < PROJECT_RECORDS.len() {
                "record"
            } else {
                "other"
            },
        )
        .unwrap();
    }
    files
}

/// What `made_up_project` wrote reads as while the records are hidden.
fn hidden_in_project() -> Vec<String> {
    PROJECT_RECORDS
        .iter()
        .map(|(_, seen)| seen.to_string())
        .chain(["other".to_string()])
        .collect()
}

/// What `made_up_project` wrote reads as while it shows.
fn shown_in_project() -> Vec<String> {
    let mut seen = vec!["record".to_string(); PROJECT_RECORDS.len()];
    seen.push("other".into());
    seen
}

/// What a command in `world`, under `key` if given, started from `dir`, reads in each of `files`, as `reads` does.
fn reads_at(e: &Env, world: &str, key: Option<&str>, dir: &Path, files: &[PathBuf]) -> Vec<String> {
    let script = files
        .iter()
        .map(|f| format!("cat '{}' 2>/dev/null || printf -; echo", f.display()))
        .collect::<Vec<_>>()
        .join("; ");
    let mut args = vec!["run", world];
    if let Some(k) = key {
        args.extend(["--key", k]);
    }
    args.extend(["--", "sh", "-c", &script]);
    let out = e.command(&args).current_dir(dir).output().unwrap();
    assert!(
        out.status.success(),
        "arbor {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn agents_records_in_a_project_stay_hidden_in_its_world_and_what_a_command_writes_is_its_own() {
    let Some(e) = Env::new("projectrecords") else {
        return;
    };
    let base = &e.base;
    // At the top of the project, in a folder right inside it, and in one two levels down.
    let top = made_up_project(base);
    let app = made_up_project(&base.join("app"));
    let api = made_up_project(&base.join("docs/api"));
    fs::create_dir_all(base.join("docs/api/v1")).unwrap();
    new_world(&e);
    let all: Vec<PathBuf> = [&top, &app, &api].into_iter().flatten().cloned().collect();
    let (hidden, shown) = (hidden_in_project(), shown_in_project());

    // From the top, those at the top and one level down are hidden, but not deeper ones...
    assert_eq!(
        reads_at(&e, "demo", None, base, &all),
        [hidden.clone(), hidden.clone(), shown.clone()].concat()
    );
    // ...unless the command starts below them.
    assert_eq!(
        reads_at(&e, "demo", None, &base.join("docs/api/v1"), &all),
        [hidden.clone(), hidden.clone(), hidden.clone()].concat()
    );

    // Writes to them vanish and leave nothing in the world, and the real files stay as they were.
    let records = |files: &[PathBuf]| files[..PROJECT_RECORDS.len()].to_vec();
    let writes = [records(&top), records(&app)]
        .concat()
        .iter()
        .map(|f| {
            format!(
                "mkdir -p '{0}' && echo x >> '{1}'",
                f.parent().unwrap().display(),
                f.display()
            )
        })
        .collect::<Vec<_>>()
        .join(" && ");
    e.ok(&["run", "demo", "--", "sh", "-c", &writes]);
    assert_eq!(e.ok(&["diff", "demo"]), "no changes\n");
    for f in [records(&top), records(&app)].concat() {
        assert_eq!(fs::read_to_string(&f).unwrap(), "record", "{}", f.display());
    }

    // A record a command writes where the project had none is the world's own: it shows among the world's changes,
    // and to the world's next command.
    inside(&e, None, "echo mine > docs/.aider.chat.history.md");
    assert_eq!(e.ok(&["diff", "demo"]), "A docs/.aider.chat.history.md\n");
    assert_eq!(
        inside(&e, None, "cat docs/.aider.chat.history.md"),
        "mine\n"
    );

    // A key that names one in the world opens it.
    let named = base.join(".aider.input.history");
    e.ok(&["key", "new", "named", "--read", named.to_str().unwrap()]);
    assert_eq!(
        reads_at(&e, "demo", Some("named"), base, &[named, top[0].clone()]),
        ["record", ""]
    );
}

#[test]
fn agents_records_in_projects_a_key_opens_stay_hidden_unless_it_names_them() {
    let Some(e) = Env::new("keyprojects") else {
        return;
    };
    new_world(&e);
    // A folder of projects: records at its top, in a project in it, and in a folder inside that project.
    let work = e.user_home.join("work");
    let top = made_up_project(&work);
    let shop = made_up_project(&work.join("shop"));
    let deep = made_up_project(&work.join("shop/deep"));
    let all: Vec<PathBuf> = [&top, &shop, &deep]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    let (hidden, shown) = (hidden_in_project(), shown_in_project());
    let w = work.to_str().unwrap();

    // A key that opens it hides those at its top and in each project in it.
    e.ok(&["key", "new", "work", "--read", w]);
    assert_eq!(
        reads_at(&e, "demo", Some("work"), &e.base, &all),
        [hidden.clone(), hidden.clone(), shown.clone()].concat()
    );
    // A key that opens one project hides those at its top and one level down.
    let s = format!("{w}/shop");
    e.ok(&["key", "new", "shop", "--read", &s]);
    assert_eq!(
        reads_at(
            &e,
            "demo",
            Some("shop"),
            &e.base,
            &[shop.clone(), deep.clone()].concat()
        ),
        [hidden.clone(), hidden.clone()].concat()
    );
    // A key that names a record, or Crush's folder, opens it, also inside a folder it opens.
    let (input, crush) = (
        format!("{w}/shop/.aider.input.history"),
        format!("{w}/shop/.crush"),
    );
    e.ok(&[
        "key", "new", "named", "--read", w, "--read", &input, "--read", &crush,
    ]);
    assert_eq!(
        reads_at(&e, "demo", Some("named"), &e.base, &shop),
        ["", "record", "", "record", "record", "record", "other"]
    );
}

/// A folder of the computer that a cell sees as the computer's own: outside the home folder and the other folders a
/// cell hides, and outside /tmp, which a cell gets fresh. Removed when dropped.
struct Outside(PathBuf);

impl Outside {
    fn new(tag: &str) -> Outside {
        let dir = Path::new("/var/tmp").join(format!("arbor-it-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Outside(dir)
    }
}

impl Drop for Outside {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What a command in `world`, under `key` if given, reads in each of `files`, as `reads` does, then again after
/// `change` has run outside the cell while the command waits. Where `look_first` is false, it reads them only after,
/// so that it has never looked for one before it was made.
fn reads_across(
    e: &Env,
    world: &str,
    key: Option<&str>,
    files: &[String],
    look_first: bool,
    change: impl FnOnce(),
) -> (Vec<String>, Vec<String>) {
    let show = files
        .iter()
        .map(|f| format!("cat {f} 2>/dev/null || printf -; echo"))
        .collect::<Vec<_>>()
        .join("; ");
    // What it reads first says it runs. A command that does not look first says so with an empty line.
    let script = if look_first {
        format!("{show}; read go; {show}")
    } else {
        format!("echo; read go; {show}")
    };
    let mut args = vec!["run", world];
    if let Some(k) = key {
        args.extend(["--key", k]);
    }
    args.extend(["--", "sh", "-c", &script]);
    let mut cell = e
        .command(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(cell.stdout.take().unwrap());
    let mut lines = |n: usize| -> Vec<String> {
        (0..n)
            .map(|_| {
                let mut line = String::new();
                out.read_line(&mut line).unwrap();
                line.trim_end_matches('\n').to_string()
            })
            .collect()
    };
    let before = lines(if look_first { files.len() } else { 1 });
    change();
    // A command that ended early has closed its end; its error is shown below.
    let _ = writeln!(cell.stdin.take().unwrap(), "go");
    let after = lines(files.len());
    let done = cell.wait_with_output().unwrap();
    assert!(
        done.status.success(),
        "arbor {args:?} failed: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    (before, after)
}

#[test]
fn a_hidden_file_stays_hidden_when_its_program_replaces_it_while_a_command_runs() {
    let Some(e) = Env::new("replaced") else {
        return;
    };
    new_world(&e);
    let home = &e.user_home;
    let h = home.to_str().unwrap();
    // A login and a record in folders a key may open, and a login and a folder of keys linked from the home folder to
    // a folder a cell sees as the computer's, as a dotfiles manager may leave them, next to a file that holds neither.
    let outside = Outside::new("replaced");
    let out = &outside.0;
    put(
        home,
        &[".claude/.credentials.json", ".claude/history.jsonl"],
        "login",
    );
    put(
        out,
        &["claude.json", "gnupg/private-keys-v1.d/key"],
        "login",
    );
    put(out, &["notes"], "other");
    std::os::unix::fs::symlink(out.join("claude.json"), home.join(".claude.json")).unwrap();
    std::os::unix::fs::symlink(out.join("gnupg"), home.join(".gnupg")).unwrap();
    let files: Vec<String> = [
        "~/.claude/.credentials.json",
        "~/.claude/history.jsonl",
        "~/.claude.json",
        "~/.gnupg/private-keys-v1.d/key",
    ]
    .map(String::from)
    .into_iter()
    .chain(
        ["claude.json", "gnupg/private-keys-v1.d/key", "notes"]
            .map(|f| out.join(f).display().to_string()),
    )
    .collect();
    // Programs replace a file by renaming a new one over it, or by deleting it and writing it again, and a folder by
    // renaming it away and making a new one.
    let rename_over = |path: &Path| {
        let new = path.with_extension("new");
        fs::write(&new, "new login").unwrap();
        fs::rename(&new, path).unwrap();
    };
    let change = || {
        rename_over(&home.join(".claude/.credentials.json"));
        fs::remove_file(home.join(".claude/history.jsonl")).unwrap();
        fs::write(home.join(".claude/history.jsonl"), "new record").unwrap();
        rename_over(&out.join("claude.json"));
        let _ = fs::remove_dir_all(out.join("gnupg.old"));
        fs::rename(out.join("gnupg"), out.join("gnupg.old")).unwrap();
        put(out, &["gnupg/private-keys-v1.d/key"], "new login");
    };
    e.ok(&["key", "new", "claude", "--read", &format!("{h}/.claude")]);
    e.ok(&["key", "new", "all", "--read", h]);
    e.ok(&["new", "home", "--from", h]);

    // In a folder a key opens, under a key that opens the whole home folder, and in a world of the home folder, they
    // stay hidden after their programs replace them while the command runs, at their real places too.
    for (world, key, through_home) in [
        ("demo", Some("claude"), ["", "", "-", "-"]),
        ("demo", Some("all"), ["", "", "", "-"]),
        ("home", None, ["", "", "", "-"]),
    ] {
        let mut hidden = through_home.to_vec();
        hidden.extend(["", "-", "other"]);
        let (before, after) = reads_across(&e, world, key, &files, true, change);
        assert_eq!(before, hidden, "{world} {key:?}, before");
        assert_eq!(after, hidden, "{world} {key:?}, after");
    }
    // The folder around a linked one stays read-only, as the rest of the computer is.
    let wrote = e.arbor(&[
        "run",
        "demo",
        "--",
        "sh",
        "-c",
        &format!("echo x > {}/new", out.display()),
    ]);
    assert!(!wrote.status.success());
    assert!(!out.join("new").exists());
    // A world inside that folder still lies over it, and keeps its changes.
    put(out, &["shop/a"], "one");
    e.ok(&[
        "new",
        "near",
        "--from",
        &out.join("shop").display().to_string(),
    ]);
    assert_eq!(
        e.ok(&["run", "near", "--", "sh", "-c", "echo two > a; cat a"]),
        "two\n"
    );
    assert_eq!(e.ok(&["run", "near", "--", "cat", "a"]), "two\n");
    assert_eq!(fs::read_to_string(out.join("shop/a")).unwrap(), "one");
}

#[test]
fn a_hidden_file_made_while_a_command_runs_stays_hidden() {
    let Some(e) = Env::new("made") else {
        return;
    };
    new_world(&e);
    let home = &e.user_home;
    let h = home.to_str().unwrap();
    // Folders that are there, a database whose SQLite files beside it are not yet, and a login linked from the home
    // folder to a file not yet made in a folder a cell sees as the computer's.
    let outside = Outside::new("made");
    let out = &outside.0;
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::create_dir_all(home.join(".config")).unwrap();
    put(home, &[".codex/state_5.sqlite"], "record");
    std::os::unix::fs::symlink(out.join("claude.json"), home.join(".claude.json")).unwrap();
    // A program outside makes them while the command runs: logins and records, in folders that are there and in
    // folders it makes, and, next to them, files that are on no list.
    let made = [
        (".claude/.credentials.json", "login"),
        (".claude/history.jsonl", "record"),
        (".claude/projects/-home-you-shop/0c1d.jsonl", "record"),
        (".ssh/id_ed25519", "login"),
        (".config/gh/hosts.yml", "login"),
        (".netrc", "login"),
        (".codex/state_5.sqlite-wal", "record"),
        (".claude/settings.json", "other"),
        ("notes", "other"),
    ];
    let files: Vec<String> = made
        .iter()
        .map(|(f, _)| format!("~/{f}"))
        .chain(["~/.codex/state_5.sqlite", "~/.claude.json"].map(String::from))
        .chain(
            ["claude.json", "notes"]
                .iter()
                .map(|f| out.join(f).display().to_string()),
        )
        .collect();
    let clear = || {
        for (f, _) in &made {
            let _ = fs::remove_file(home.join(f));
        }
        for dir in [".claude/projects", ".ssh", ".config/gh"] {
            let _ = fs::remove_dir_all(home.join(dir));
        }
        for f in ["claude.json", "notes"] {
            let _ = fs::remove_file(out.join(f));
        }
    };
    let change = || {
        for (f, text) in made {
            put(home, &[f], text);
        }
        put(out, &["claude.json"], "login");
        put(out, &["notes"], "other");
    };
    e.ok(&["key", "new", "claude", "--read", &format!("{h}/.claude")]);
    e.ok(&["key", "new", "all", "--read", h]);
    e.ok(&["new", "home", "--from", h]);

    // In a folder a key opens, under a key that opens the whole home folder, and in a world of the home folder, a
    // command that looks only after they are made finds none of them, at their real places neither, while it finds
    // the files next to them that no list names.
    let hidden = ["-"; 7];
    for (world, key, rest) in [
        (
            "demo",
            Some("claude"),
            ["other", "-", "-", "-", "-", "other"],
        ),
        (
            "demo",
            Some("all"),
            ["other", "other", "-", "-", "-", "other"],
        ),
        ("home", None, ["other", "other", "-", "-", "-", "other"]),
    ] {
        clear();
        let (_, after) = reads_across(&e, world, key, &files, false, change);
        let expected: Vec<&str> = hidden.iter().chain(&rest).copied().collect();
        assert_eq!(after, expected, "{world} {key:?}");
    }
    // Nothing of what hid them is left in the world.
    assert_eq!(e.ok(&["diff", "home"]), "no changes\n");
}

/// Makes a made-up SQLite database at the path given, with a table of 50 rows, as an agent makes its own: `open` in
/// WAL mode, left as a running program leaves it, with its `-wal` and `-shm` files; `closed` in WAL mode, closed, so
/// without them; or `delete`, with a rollback journal.
const MAKE_DATABASE: &str = r#"
import os, sqlite3, sys
path, mode = sys.argv[1], sys.argv[2]
c = sqlite3.connect(path)
c.execute("pragma journal_mode=" + ("delete" if mode == "delete" else "wal"))
c.execute("pragma wal_autocheckpoint=0")
c.execute("create table thread(title)")
c.executemany("insert into thread values (?)", [("made up",)] * 50)
c.commit()
if mode == "open":
    os._exit(0)
c.close()
"#;

/// What an agent does with each database given after `how`, in a cell: prints how many tables it finds, then writes
/// to it in WAL mode and prints how many rows it reads back. With `killed`, it then stops dead, as an agent does when
/// its key ends, and leaves the files SQLite keeps beside them.
const USE_DATABASES: &str = r#"
import os, sqlite3, sys
dbs = [sqlite3.connect(path) for path in sys.argv[2:]]
for c in dbs:
    tables = c.execute("select count(*) from sqlite_master").fetchone()[0]
    c.execute("pragma journal_mode=wal")
    c.execute("create table if not exists cell(x)")
    c.execute("insert into cell values (1)")
    c.commit()
    print(tables, c.execute("select count(*) from cell").fetchone()[0])
if sys.argv[1] == "killed":
    os._exit(0)
for c in dbs:
    c.close()
"#;

#[test]
fn an_agent_s_hidden_database_is_missing_so_it_makes_its_own_and_yours_stays_as_it_was() {
    let Some(e) = Env::new("databases") else {
        return;
    };
    let python = "/usr/bin/python3";
    if !Path::new(python).exists() {
        return common::skip(&format!("this test needs {python}"));
    }
    new_world(&e);
    let home = &e.user_home;
    let h = home.to_str().unwrap();
    // Codex's databases, one open and one closed, and the Copilot CLI's, with their settings next to them.
    put(
        home,
        &[".codex/config.toml", ".copilot/settings.json"],
        "other",
    );
    let databases = [
        (".codex/state_5.sqlite", "open"),
        (".codex/logs_2.sqlite", "closed"),
        (".copilot/session-store.db", "delete"),
    ];
    for (db, mode) in databases {
        let path = home.join(db);
        let args = ["-I", "-c", MAKE_DATABASE, path.to_str().unwrap(), mode];
        assert!(Command::new(python).args(args).status().unwrap().success());
    }
    // Every file in the agents' folders, with what it holds.
    let agents = || -> Vec<(PathBuf, Vec<u8>)> {
        let mut all: Vec<_> = [".codex", ".copilot"]
            .iter()
            .flat_map(|dir| fs::read_dir(home.join(dir)).unwrap())
            .map(|entry| entry.unwrap().path())
            .map(|path| (path.clone(), fs::read(&path).unwrap()))
            .collect();
        all.sort();
        all
    };
    let yours = agents();
    assert!(yours.iter().any(|(p, _)| p.ends_with("state_5.sqlite-shm")));
    assert_eq!(yours.len(), 7);
    key_opening(&e, "agents", &[".codex", ".copilot"]);
    e.ok(&["new", "home", "--from", h]);
    let paths: Vec<String> = databases
        .iter()
        .map(|(db, _)| home.join(db).display().to_string())
        .collect();
    let agent = |world: &str, key: Option<&str>, how: &str| {
        let mut args = vec!["run", world];
        if let Some(k) = key {
            args.extend(["--key", k]);
        }
        args.extend(["--", python, "-I", "-c", USE_DATABASES, how]);
        args.extend(paths.iter().map(String::as_str));
        e.ok(&args)
    };

    // Under a key that opens the agents' folders, and in a world of the home folder, run after run, an agent finds
    // no database where yours are, makes its own, and reads back what it wrote, while the settings next to them show.
    for (world, key) in [("demo", Some("agents")), ("home", None), ("home", None)] {
        assert_eq!(agent(world, key, "closes"), "0 1\n".repeat(3), "{world}");
        let settings = [".codex/config.toml", ".copilot/settings.json"];
        assert_eq!(reads(&e, world, key, &settings), ["other", "other"]);
    }
    // In the world, what an agent leaves there when it stops dead, and what it writes next to them, vanish with the
    // command: none of it reaches the world's changes, so keeping them leaves your databases as they were.
    agent("home", None, "killed");
    e.ok(&[
        "run",
        "home",
        "--",
        "sh",
        "-c",
        "echo x >> ~/.codex/config.toml && echo x > ~/.copilot/new",
    ]);
    assert_eq!(e.ok(&["diff", "home"]), "no changes\n");
    e.ok(&["keep", "home"]);
    assert_eq!(agents(), yours);
}

#[test]
fn a_command_the_world_hides_is_named_with_the_key_that_opens_it() {
    let Some(e) = Env::new("tools") else { return };
    new_world(&e);
    let home = &e.user_home;
    script(&home.join(".cargo/bin/hello"), "#!/bin/sh\necho hello\n");
    // As rustup links cargo to itself.
    std::os::unix::fs::symlink("hello", home.join(".cargo/bin/linked")).unwrap();
    script(&home.join("bin/own"), "#!/bin/sh\necho own\n");
    // A script in the world whose interpreter the world hides.
    script(&e.base.join("greet"), "#!/usr/bin/env hello\n");
    let path = path_of(&[".cargo/bin", ".npm-global/bin", "bin"].map(|d| home.join(d)));
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

#[test]
fn a_key_s_home_keeps_what_its_commands_write_there_and_nothing_else_sees_it() {
    let Some(e) = Env::new("home") else { return };
    new_world(&e);
    let dir = e.home.join("homes/agent");
    let made = e.ok(&["key", "new", "agent", "--home"]);
    assert!(
        made.contains(&format!("; home {};", dir.display())),
        "{made}"
    );
    e.ok(&["key", "new", "other"]);
    // What a command writes in the key's home is there for the next command under the key.
    inside(
        &e,
        Some("agent"),
        "echo kept > ~/.agentrc; mkdir -p ~/.config/tool && echo dark > ~/.config/tool/theme",
    );
    let read = "cat ~/.agentrc ~/.config/tool/theme 2>/dev/null || echo none";
    assert_eq!(inside(&e, Some("agent"), read), "kept\ndark\n");
    // Under another key, or none, the home folder is empty, and the real one is untouched.
    assert_eq!(inside(&e, Some("other"), read), "none\n");
    assert_eq!(inside(&e, None, read), "none\n");
    assert_eq!(fs::read_dir(&e.user_home).unwrap().count(), 0);
    // It is in arbor's folder, where only the user may enter, and never among the world's changes.
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!((mode(&e.home.join("homes")), mode(&dir)), (0o700, 0o700));
    assert_eq!(fs::read_to_string(dir.join(".agentrc")).unwrap(), "kept\n");
    assert_eq!(e.ok(&["diff", "demo"]), "no changes\n");
    // `arbor keys` shows what it takes on disk, and the ledger names it.
    let keys = e.ok(&["keys"]);
    let row = |name: &str| {
        keys.lines()
            .find(|l| l.starts_with(&format!("{name} ")))
            .unwrap()
            .to_string()
    };
    assert!(
        row("agent").contains("KiB") && !row("other").contains("KiB"),
        "{keys}"
    );
    assert!(
        e.ledger_rows()
            .iter()
            .any(|r| r.contains(" key ") && r.contains("homes/agent")),
        "{:#?}",
        e.ledger_rows()
    );
    // Where the key's home would stand in for the world, or for what the key opens, it is refused.
    e.ok(&["new", "whole", "--from", e.user_home.to_str().unwrap()]);
    let out = e.arbor(&["run", "whole", "--key", "agent", "--", "true"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is your home folder"));
    for holds in [&e.user_home, &e.root] {
        let args = [
            "key",
            "new",
            "wide",
            "--home",
            "--read",
            holds.to_str().unwrap(),
        ];
        assert_eq!(e.arbor(&args).status.code(), Some(3), "{holds:?}");
    }
}

#[test]
fn a_key_s_home_lives_as_long_as_the_key_and_keys_within_it_share_it_or_have_their_own() {
    let Some(e) = Env::new("homes") else { return };
    new_world(&e);
    e.ok(&["key", "new", "agent", "--home"]);
    e.ok(&["key", "new", "short", "--within", "agent", "--for", "1m"]);
    e.ok(&["key", "new", "fresh", "--within", "agent", "--home"]);
    // A home of its own within a key that has none would open more.
    e.ok(&["key", "new", "plain"]);
    let args = ["key", "new", "more", "--within", "plain", "--home"];
    assert_eq!(e.arbor(&args).status.code(), Some(3));
    // A key made within another shares its home, unless it has its own.
    inside(&e, Some("agent"), "echo a > ~/a");
    inside(&e, Some("short"), "echo b > ~/b");
    inside(&e, Some("fresh"), "echo c > ~/c");
    let seen = |key| inside(&e, Some(key), "ls ~");
    assert_eq!(
        (seen("agent"), seen("short"), seen("fresh")),
        ("a\nb\n".into(), "a\nb\n".into(), "c\n".into())
    );
    let keys = e.ok(&["keys"]);
    assert!(
        keys.lines()
            .any(|l| l.starts_with("short ") && l.contains(" of agent ")),
        "{keys}"
    );
    // A home that is gone starts again, empty.
    fs::remove_dir_all(e.home.join("homes/fresh")).unwrap();
    assert_eq!(seen("fresh"), "");
    // Taking a key back removes its home, and those of the keys made within it.
    assert_eq!(
        e.ok(&["key", "take-back", "agent"]),
        "took back key agent, and keys fresh, short made within it; stopped 0 commands; removed 2 home folders\n"
    );
    assert_eq!(fs::read_dir(e.home.join("homes")).unwrap().count(), 0);
    assert!(
        e.ledger_rows()
            .iter()
            .any(|r| r.contains(" take-back ") && r.contains("; removed 2 home folders"))
    );
    // A new key under the same name starts with an empty one.
    e.ok(&["key", "new", "agent", "--home"]);
    assert_eq!(seen("agent"), "");
    // A home left without its key, as when the key's file is deleted by hand, is neither handed on nor deleted.
    fs::remove_file(e.home.join("keys/agent.json")).unwrap();
    let out = e.arbor(&["key", "new", "agent", "--home"]);
    assert_eq!(out.status.code(), Some(3));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("is left from an earlier key named 'agent'"),
        "{err}"
    );
    assert!(e.home.join("homes/agent").is_dir() && !e.home.join("keys/agent.json").exists());
}

#[test]
fn an_ended_key_keeps_its_home_until_it_is_taken_back() {
    let Some(e) = Env::new("homeend") else { return };
    new_world(&e);
    e.ok(&["key", "new", "brief", "--home", "--for", "2s"]);
    inside(&e, Some("brief"), "echo x > ~/x");
    std::thread::sleep(Duration::from_millis(2100));
    let out = e.arbor(&["run", "demo", "--key", "brief", "--", "true"]);
    assert_eq!(out.status.code(), Some(3));
    let home = e.home.join("homes/brief");
    assert!(home.join("x").exists());
    assert!(
        e.ok(&["key", "take-back", "brief"])
            .ends_with("; removed 1 home folder\n")
    );
    assert!(!home.exists());
}

#[test]
fn a_renewed_key_keeps_its_home_past_its_first_end() {
    let Some(e) = Env::new("renew") else { return };
    new_world(&e);
    let made = Instant::now();
    e.ok(&["key", "new", "agent", "--home", "--for", "2s"]);
    e.ok(&["key", "new", "short", "--within", "agent"]);
    inside(&e, Some("agent"), "echo kept > ~/.agentrc");
    std::thread::sleep(
        (made + Duration::from_millis(2100)).saturating_duration_since(Instant::now()),
    );
    let row = |name: &str| {
        let keys = e.ok(&["keys"]);
        let row = keys.lines().find(|l| l.starts_with(&format!("{name} ")));
        row.unwrap().to_string()
    };
    let refused = |args: &[&str]| {
        let out = e.arbor(args);
        assert_eq!(out.status.code(), Some(3), "{args:?}");
        String::from_utf8_lossy(&out.stderr).into_owned()
    };

    // Once it has ended, its commands are refused, and arbor says how to renew it.
    let err = refused(&["run", "demo", "--key", "agent", "--", "true"]);
    assert!(
        err.contains("key 'agent' ended at")
            && err.contains("`arbor key renew agent` moves its end later"),
        "{err}"
    );
    assert!(row("agent").contains(" ended "));
    // A key made within it cannot outlast it, so it waits for that one.
    let err = refused(&["key", "renew", "short"]);
    assert!(err.contains("renew that one first"), "{err}");
    // Renewed, it opens its home again, with what its commands left there.
    let said = e.ok(&["key", "renew", "agent", "--for", "1h"]);
    assert!(
        said.starts_with("renewed key agent until ") && said.contains(" instead of "),
        "{said}"
    );
    assert_eq!(inside(&e, Some("agent"), "cat ~/.agentrc"), "kept\n");
    assert!(!row("agent").contains(" ended "));
    // The key made within it keeps its own end, until it is renewed too, up to the other's end.
    assert!(row("short").contains(" ended "));
    refused(&["key", "renew", "short", "--for", "2h"]);
    e.ok(&["key", "renew", "short"]);
    assert!(!row("short").contains(" ended "));
    assert_eq!(inside(&e, Some("short"), "cat ~/.agentrc"), "kept\n");

    // An end sooner than the one it has is refused; mistakes in how renew is called change nothing.
    let err = refused(&["key", "renew", "agent", "--for", "1m"]);
    assert!(err.contains("already lasts until"), "{err}");
    for args in [
        &["key", "renew", "nosuch"][..],
        &["key", "renew"],
        &["key", "renew", "agent", "--for", "1h", "--until", "18:00"],
        &["key", "renew", "agent", "--read", "/usr"],
    ] {
        assert_eq!(e.arbor(args).status.code(), Some(2), "{args:?}");
    }
    // The ledger has each renewal, and each refusal, with its key.
    let rows = e.ledger_rows();
    let renewals: Vec<(&str, &str)> = rows
        .iter()
        .filter(|r| r.contains(" renew "))
        .map(|r| {
            let key = ["agent", "short"]
                .into_iter()
                .find(|k| r.contains(&format!(" {k} ")))
                .unwrap_or("?");
            let out = [" ok ", " refused "]
                .into_iter()
                .find(|o| r.contains(o))
                .unwrap_or("?");
            (key, out.trim())
        })
        .collect();
    assert_eq!(
        renewals,
        [
            ("short", "refused"),
            ("agent", "ok"),
            ("short", "refused"),
            ("short", "ok"),
            ("agent", "refused")
        ],
        "{rows:#?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.contains(" renew ") && r.contains("key agent until ")),
        "{rows:#?}"
    );
    // Taken back, a renewed key's home goes as any other's.
    assert!(
        e.ok(&["key", "take-back", "agent"])
            .ends_with("; removed 1 home folder\n")
    );
    assert!(!e.home.join("homes/agent").exists());
}

#[test]
fn a_command_running_under_a_key_renewed_meanwhile_runs_until_the_new_end() {
    let Some(e) = Env::new("renewrun") else {
        return;
    };
    new_world(&e);
    e.ok(&["new", "other", "--from", e.base.to_str().unwrap()]);
    let made = Instant::now();
    e.ok(&["key", "new", "agent", "--for", "2s"]);
    // One command that ends on its own after the key's first end, and one that would run on.
    let start = |world: &str, script: &str| {
        e.command(&["run", world, "--key", "agent", "--", "sh", "-c", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let short = start("demo", "touch started; sleep 4; echo done");
    let long = start("other", "touch started; sleep 30");
    wait_for(&e.home.join("worlds/demo/up/started"));
    wait_for(&e.home.join("worlds/other/up/started"));
    e.ok(&["key", "renew", "agent", "--for", "6s"]);

    let short = short.wait_with_output().unwrap();
    assert_eq!(
        (short.status.code(), String::from_utf8_lossy(&short.stdout)),
        (Some(0), "done\n".into()),
        "{}",
        String::from_utf8_lossy(&short.stderr)
    );
    // The other stops at the new end: it is the key's end that moved, not its bound.
    let long = long.wait_with_output().unwrap();
    let ran = made.elapsed();
    assert_eq!(long.status.code(), Some(137));
    assert!(String::from_utf8_lossy(&long.stderr).contains("key 'agent' ended at"));
    assert!(
        ran >= Duration::from_secs(6) && ran < Duration::from_secs(20),
        "{ran:?}"
    );
    let rows = e.ledger_rows();
    let ended = |out: &str| {
        rows.iter()
            .any(|r| r.contains(" run ") && r.contains(" agent ") && r.contains(out))
    };
    assert!(
        ended(" exit 0 ") && ended(" killed: key expired "),
        "{rows:#?}"
    );
}

#[test]
fn the_agent_s_own_logins_stay_in_its_home_while_yours_stay_hidden() {
    let Some(e) = Env::new("homesecrets") else {
        return;
    };
    new_world(&e);
    let home = &e.user_home;
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/id_ed25519"), "secret").unwrap();
    fs::write(home.join(".netrc"), "yours").unwrap();
    let cargo = home.join(".cargo");
    fs::create_dir_all(&cargo).unwrap();
    fs::write(cargo.join("config.toml"), "[build]").unwrap();
    fs::write(cargo.join("credentials.toml"), "token").unwrap();
    made_up_logins(home);
    e.ok(&[
        "key",
        "new",
        "agent",
        "--home",
        "--read",
        cargo.to_str().unwrap(),
    ]);
    // The logins an agent keeps in its home are its own to read next time...
    inside(
        &e,
        Some("agent"),
        "echo mine > ~/.netrc; mkdir ~/.ssh && echo mine > ~/.ssh/config; echo x > ~/.cargo/new; \
         mkdir -p ~/.local/share/opencode ~/.claude && echo mine > ~/.local/share/opencode/opencode.db && \
         echo mine > ~/.claude/.credentials.json",
    );
    let cat = |path: &str| {
        inside(
            &e,
            Some("agent"),
            &format!("cat {path} 2>/dev/null || echo hidden"),
        )
    };
    assert_eq!(cat("~/.netrc"), "mine\n");
    assert_eq!(cat("~/.ssh/config"), "mine\n");
    assert_eq!(cat("~/.local/share/opencode/opencode.db"), "mine\n");
    assert_eq!(cat("~/.claude/.credentials.json"), "mine\n");
    // ...while yours stay hidden, also inside what the key opens on top of its home, where writes still vanish.
    assert_eq!(cat("~/.ssh/id_ed25519"), "hidden\n");
    assert_eq!(cat("~/.local/share/opencode/auth.json"), "hidden\n");
    assert_eq!(cat("~/.cargo/config.toml"), "[build]");
    assert_eq!(cat("~/.cargo/credentials.toml"), "");
    assert_eq!(cat("~/.cargo/new"), "hidden\n");
    assert!(!cargo.join("new").exists());
    assert_eq!(fs::read_to_string(home.join(".netrc")).unwrap(), "yours");
    for (f, _) in AGENT_LOGINS {
        assert_eq!(fs::read_to_string(home.join(f)).unwrap(), "login", "{f}");
    }
}

#[test]
fn an_agent_s_own_records_stay_in_its_home_while_yours_stay_hidden() {
    let Some(e) = Env::new("homerecords") else {
        return;
    };
    new_world(&e);
    let home = &e.user_home;
    made_up_records(home);
    let claude = home.join(".claude");
    e.ok(&[
        "key",
        "new",
        "agent",
        "--home",
        "--read",
        claude.to_str().unwrap(),
    ]);
    // What an agent keeps of its sessions in its home is its own to read the next time...
    inside(
        &e,
        Some("agent"),
        "mkdir -p ~/.codex/sessions ~/.local/state/opencode && printf mine > ~/.codex/sessions/rollout.jsonl && \
         printf mine > ~/.codex/state_7.sqlite && printf mine > ~/.local/state/opencode/prompt-history.jsonl",
    );
    let files = [
        ".codex/sessions/rollout.jsonl",
        ".codex/state_7.sqlite",
        ".local/state/opencode/prompt-history.jsonl",
        // ...while yours stay hidden, also in your folder that the key opens on top of its home.
        ".claude/settings.json",
        ".claude/history.jsonl",
        ".claude/projects/-home-you-work-shop/0c1d.jsonl",
    ];
    assert_eq!(
        reads(&e, "demo", Some("agent"), &files),
        ["mine", "mine", "mine", "other", "", "-"]
    );
    for (f, _) in AGENT_RECORDS {
        assert_eq!(fs::read_to_string(home.join(f)).unwrap(), "record", "{f}");
    }
}

#[test]
fn a_world_in_the_home_folder_lies_over_the_key_s_home_and_arbor_s_folder_stays_hidden() {
    let Some(e) = Env::new("homeworld") else {
        return;
    };
    // Arbor's folder in the home folder, where it is by default, and a world there too.
    let state = e.user_home.join(".local/share/arbor");
    let shop = e.user_home.join("work/shop");
    fs::create_dir_all(&shop).unwrap();
    fs::write(shop.join("a"), "one").unwrap();
    let arbor = |args: &[&str]| {
        let out = e.command(args).env("ARBOR_HOME", &state).output().unwrap();
        assert!(
            out.status.success(),
            "arbor {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let inside =
        |key: &str, script: &str| arbor(&["run", "shop", "--key", key, "--", "sh", "-c", script]);
    arbor(&["new", "shop", "--from", shop.to_str().unwrap()]);
    arbor(&["key", "new", "agent", "--home"]);
    arbor(&["key", "new", "other", "--home"]);
    inside("other", "echo theirs > ~/note");
    // The world shows on top of the key's home; arbor's folder, with the other key's home, does not show at all.
    let looks = "cat a; echo; ls -A ~/.local/share/arbor | wc -l; \
                 cat ~/.local/share/arbor/homes/other/note 2>/dev/null || echo hidden";
    assert_eq!(
        inside(
            "agent",
            &format!("echo two > b; echo kept > ~/note; {looks}")
        ),
        "one\n0\nhidden\n"
    );
    // The world's change is in the world, the home's in the home, and neither reached a real folder.
    assert_eq!(arbor(&["diff", "shop"]), "A b\n");
    let home = state.join("homes/agent");
    assert_eq!(fs::read_to_string(home.join("note")).unwrap(), "kept\n");
    assert!(!home.join("work/shop/b").exists() && !shop.join("b").exists());
    assert!(!e.user_home.join("note").exists());
    assert_eq!(
        inside("agent", &format!("cat ~/note; {looks}")),
        "kept\none\n0\nhidden\n"
    );
}

#[test]
fn a_command_in_the_key_s_home_runs_and_one_only_in_yours_is_named_with_the_key_that_opens_it() {
    let Some(e) = Env::new("hometools") else {
        return;
    };
    new_world(&e);
    let home = &e.user_home;
    let path = path_of(&[home.join("bin")]);
    let run = |args: &[&str]| {
        let out = e.command(args).env("PATH", &path).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        (
            out.status.code(),
            String::from_utf8(out.stdout).unwrap(),
            err,
        )
    };
    e.ok(&["key", "new", "agent", "--home"]);
    // A tool the agent installs in its home is found the next time.
    let install =
        "mkdir ~/bin && printf '#!/bin/sh\\necho mine\\n' > ~/bin/mine && chmod +x ~/bin/mine";
    let (code, _, err) = run(&["run", "demo", "--key", "agent", "--", "sh", "-c", install]);
    assert_eq!(code, Some(0), "{err}");
    let (code, out, err) = run(&["run", "demo", "--key", "agent", "--", "mine"]);
    assert_eq!((code, out.as_str()), (Some(0), "mine\n"), "{err}");
    // Without the key it is nowhere, and a tool only in your home folder is named with the key option that opens it.
    let (code, _, err) = run(&["run", "demo", "--", "mine"]);
    assert_eq!(code, Some(127), "{err}");
    assert!(err.contains("world demo has no command `mine`"), "{err}");
    script(&home.join("bin/yours"), "#!/bin/sh\necho yours\n");
    let (code, _, err) = run(&["run", "demo", "--key", "agent", "--", "yours"]);
    assert_eq!(code, Some(127), "{err}");
    assert!(
        err.contains("`yours` is in ~/bin, which the world hides and key 'agent' does not open")
            && err.contains("--read ~/bin"),
        "{err}"
    );
}
