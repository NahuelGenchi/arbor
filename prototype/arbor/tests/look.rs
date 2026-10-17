//! `arbor look` end to end: reads answered as tables, several in one call, on a throwaway folder and git repository,
//! and inside a world. Only the last test needs bwrap, and it skips like `common` says.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::{Env, new_world};

/// A throwaway folder, removed when the test ends.
struct Folder(PathBuf);

impl Folder {
    fn new(tag: &str) -> Folder {
        let dir = std::env::temp_dir().join(format!("arbor-look-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Folder(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// git, and the git that `arbor look` runs, with none of this computer's settings.
fn quiet(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("TZ", "UTC")
        .stdin(Stdio::null())
}

fn git(dir: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    for (var, value) in [
        ("GIT_AUTHOR_NAME", "Ana"),
        ("GIT_AUTHOR_EMAIL", "ana@example.com"),
        ("GIT_COMMITTER_NAME", "Ana"),
        ("GIT_COMMITTER_EMAIL", "ana@example.com"),
    ] {
        cmd.env(var, value);
    }
    let out = quiet(cmd.current_dir(dir).args(args)).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", text(&out.stderr));
    text(&out.stdout)
}

fn look(dir: &Path, args: &[&str]) -> Output {
    quiet(
        Command::new(env!("CARGO_BIN_EXE_arbor"))
            .current_dir(dir)
            .arg("look")
            .args(args),
    )
    .env("ARBOR_HOME", dir.join("no-arbor-home"))
    .output()
    .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn status_log_and_files_in_one_call() {
    let f = Folder::new("repo");
    let repo = &f.0;
    git(repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), "one\n").unwrap();
    fs::write(repo.join("b.txt"), "two\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "first: two files"]);
    fs::write(repo.join("a.txt"), "one\nmore\n").unwrap();
    fs::remove_file(repo.join("b.txt")).unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", "second\tline"]);
    fs::write(repo.join("a.txt"), "changed\n").unwrap();
    fs::create_dir(repo.join("docs")).unwrap();
    fs::write(repo.join("docs/new.md"), "new\n").unwrap();
    let commits = git(
        repo,
        &[
            "log",
            "--date=format-local:%Y-%m-%dT%H:%M",
            "--format=%h %ad",
        ],
    );
    let [(second, d2), (first, d1)] = commits
        .lines()
        .map(|l| l.split_once(' ').unwrap())
        .collect::<Vec<_>>()[..]
    else {
        panic!("{commits}")
    };
    let index = fs::read(repo.join(".git/index")).unwrap();
    let out = look(
        repo,
        &[
            "status", "+", "log", "-f", "all", "+", "log", "-n", "1", "a.txt", "+", "files",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        format!(
            "# status: on main\npath\tchange\na.txt\tmodified\ndocs/\tuntracked\n\
             # log -f all\nhash\tsubject\tdate\tauthor\tfiles\tadded\tremoved\n\
             {second}\tsecond\\tline\t{d2}\tAna\t2\t1\t1\n{first}\tfirst: two files\t{d1}\tAna\t2\t2\t0\n\
             # log -n 1 a.txt: first 1, and more\nhash\tsubject\n{second}\tsecond\\tline\n\
             # files\nname\n.git/\na.txt\ndocs/\n"
        )
    );
    // Reading changed nothing: git rewrote no index.
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
}

#[test]
fn a_failing_query_says_why_and_the_others_still_answer() {
    let f = Folder::new("fail");
    fs::write(f.0.join("only"), "").unwrap();
    let out = look(
        &f.0,
        &[
            "files",
            "none",
            "+",
            "status",
            "+",
            "files",
            "-f",
            "name,size",
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    let shown = text(&out.stdout);
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], "# files none: error: no such folder");
    assert!(
        lines[1].starts_with("# status: error: not a git repository"),
        "{shown}"
    );
    assert_eq!(
        lines[2..],
        ["# files -f name,size", "name\tsize", "only\t0"]
    );
}

#[test]
fn a_usage_error_runs_no_query() {
    let f = Folder::new("usage");
    for (args, said) in [
        (
            &["files", "+", "files", "-f", "colour"][..],
            "files has no field 'colour'",
        ),
        (&["files", "+", "sizes"], "there is no query 'sizes'"),
        (&["files", "+"], "a + with no query"),
        (&["log", "--output=x"], "log: invalid option '--output'"),
    ] {
        let out = look(&f.0, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(text(&out.stdout), "", "{args:?}");
        assert!(text(&out.stderr).contains(said), "{}", text(&out.stderr));
    }
    assert!(!f.0.join("x").exists());
    // Alone, it says how to ask.
    let out = look(&f.0, &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).starts_with("arbor look: answers reads as short tables"));
}

#[test]
fn procs_finds_a_process_by_its_command() {
    let f = Folder::new("procs");
    let mut child = Command::new("sleep").arg("29.0517").spawn().unwrap();
    let out = look(&f.0, &["procs", "29.0517", "-f", "pid,name,parent,cmd"]);
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        format!(
            "# procs 29.0517 -f pid,name,parent,cmd\npid\tname\tparent\tcmd\n{}\tsleep\t{}\tsleep 29.0517\n",
            child.id(),
            std::process::id()
        )
    );
    // By default, every process, the most memory first: this test's own is among them.
    let out = look(&f.0, &["procs", "-n", "0"]);
    let shown = text(&out.stdout);
    assert!(
        shown.starts_with("# procs -n 0\npid\tmem\tname\n"),
        "{shown}"
    );
    let me = std::process::id().to_string();
    assert!(
        shown.lines().any(|l| l.split('\t').next() == Some(&me)),
        "{shown}"
    );
}

#[test]
fn inside_a_world_it_sees_the_world() {
    let Some(e) = Env::new("look") else { return };
    new_world(&e);
    let out = e.arbor(&[
        "run",
        "demo",
        "--",
        "sh",
        "-c",
        "echo new > made && rm src/gone && arbor look files + files src + procs -f name; echo end",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    let procs = shown
        .strip_prefix("# files\nname\nmade\nsrc/\n# files src\nname\na\n# procs -f name\nname\n")
        .and_then(|rest| rest.strip_suffix("end\n"))
        .unwrap_or_else(|| panic!("{shown}"));
    // The cell's own processes only: its shell, and none of this computer's.
    let names: Vec<&str> = procs.lines().collect();
    assert!(names.contains(&"sh") && names.len() < 5, "{shown}");
    // The real folder is as it was.
    assert!(!e.base.join("made").exists() && e.base.join("src/gone").exists());
}
