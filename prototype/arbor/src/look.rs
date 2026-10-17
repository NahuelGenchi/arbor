//! `arbor look`: the reads an agent makes most, answered as short tables of only the asked fields, several in one
//! call. The report found that 16% of an agent's steps are a read right after another one, and each step re-reads
//! the whole context; and that the usual commands print far more than is asked: `ls -la` of 100 files is 2,776
//! tokens against 401 for their names, `git log -20` 18,928 against 530, `ps aux` 9,900 against 286
//! (OPTIMIZATION.md §6). Each answer is a title line, then a header and rows separated by tabs, which cost fewer
//! tokens than aligned columns. Nothing here writes: git runs as a reader, and nothing goes to the ledger.

use std::collections::HashMap;
use std::ffi::{CStr, OsStr, OsString};
use std::fs::{self, FileType};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use lexopt::prelude::*;

use crate::error::{Error, Result};
use crate::table;

/// What `arbor look` alone prints: all an agent needs to use it, and nothing until it asks.
pub const ABOUT: &str = "\
arbor look: answers reads as short tables, several in one call.

  arbor look QUERY [+ QUERY]...        e.g.  arbor look status + log -n 5 + files src

  files [DIR]           what a folder holds, by name, 100 rows:
                        name (a folder's ends in /) | type size mode owner modified
  status                what changed in the git repository here, by path from its top, 100 rows:
                        path change | staged from
  log [REV] [PATH]...   commits, newest first, 20 rows:
                        hash subject | date author files added removed
  procs [TEXT]          processes (with TEXT, those whose command holds it), most memory first, 30 rows:
                        pid mem name | parent user cpu started cmd

After a query: -f FIELDS (a,b,c or all; by default the fields before |), -n ROWS (0 for all).
Each answer is a line `# QUERY`, which says what was cut or why the query failed, then a header and
rows, separated by tabs. Sizes and mem (memory in use) are bytes, cpu is seconds used, times are
YYYY-MM-DDTHH:MM local, modes are octal. change: added, modified, deleted, renamed, copied, typechange,
untracked, conflict. staged: yes, no, partly. arbor look only reads; inside a world it sees what the
world sees.
";

/// A kind of query: its name, every field it has with the default ones first, and how many rows it gives unless
/// asked for another number.
struct Spec {
    name: &'static str,
    fields: &'static [&'static str],
    defaults: usize,
    rows: usize,
    /// How many words may follow the name, when there is a limit.
    words: Option<usize>,
    answer: fn(&Query) -> Result<Found>,
}

const SPECS: &[Spec] = &[
    Spec {
        name: "files",
        fields: &["name", "type", "size", "mode", "owner", "modified"],
        defaults: 1,
        rows: 100,
        words: Some(1),
        answer: files,
    },
    Spec {
        name: "status",
        fields: &["path", "change", "staged", "from"],
        defaults: 2,
        rows: 100,
        words: Some(0),
        answer: status,
    },
    Spec {
        name: "log",
        fields: &[
            "hash", "subject", "date", "author", "files", "added", "removed",
        ],
        defaults: 2,
        rows: 20,
        words: None,
        answer: log,
    },
    Spec {
        name: "procs",
        fields: &[
            "pid", "mem", "name", "parent", "user", "cpu", "started", "cmd",
        ],
        defaults: 3,
        rows: 30,
        words: Some(1),
        answer: procs,
    },
];

/// One query, read and checked before any query runs.
pub struct Query {
    spec: &'static Spec,
    /// The query as it was asked, for the title of its answer.
    asked: String,
    words: Vec<OsString>,
    /// The fields to print, as indexes into the spec's, in the order asked.
    fields: Vec<usize>,
    /// At most this many rows; 0 for all.
    rows: usize,
}

/// What a query found: every field of each row, in its spec's order, and what its title line says.
struct Found {
    rows: Vec<Vec<String>>,
    notes: Vec<String>,
}

impl Query {
    fn wants(&self, field: &str) -> bool {
        self.fields.iter().any(|&i| self.spec.fields[i] == field)
    }
}

/// Reads `QUERY [+ QUERY]...`. Every query is checked here, so a mistake in the last one stops them all before any
/// runs.
pub fn parse(args: Vec<OsString>) -> Result<Vec<Query>> {
    args.split(|a| a.as_os_str() == "+")
        .map(|words| match words {
            [] => Err(Error::usage(
                "a + with no query on one side; run `arbor look` to see how to ask",
            )),
            _ => query(words),
        })
        .collect()
}

fn query(words: &[OsString]) -> Result<Query> {
    let name = words[0].to_str().unwrap_or("");
    let spec = SPECS.iter().find(|s| s.name == name).ok_or_else(|| {
        let names: Vec<&str> = SPECS.iter().map(|s| s.name).collect();
        Error::usage(format!(
            "there is no query '{}'; arbor look answers {} (run `arbor look`)",
            cell(words[0].as_bytes()),
            names.join(", ")
        ))
    })?;
    let mut q = Query {
        spec,
        asked: words.iter().map(|w| word(w)).collect::<Vec<_>>().join(" "),
        words: Vec::new(),
        fields: (0..spec.defaults).collect(),
        rows: spec.rows,
    };
    let mut p = lexopt::Parser::from_args(words[1..].iter().cloned());
    while let Some(arg) = p.next()? {
        match arg {
            Short('f') | Long("fields") => q.fields = fields(spec, &p.value()?.string()?)?,
            Short('n') | Long("rows") => q.rows = p.value()?.parse()?,
            Value(w) if spec.words.is_none_or(|n| q.words.len() < n) => q.words.push(w),
            arg => {
                return Err(Error::usage(format!("{}: {}", spec.name, arg.unexpected())));
            }
        }
    }
    Ok(q)
}

/// `a,b,c` or `all`, as indexes into the spec's fields.
fn fields(spec: &Spec, list: &str) -> Result<Vec<usize>> {
    if list == "all" {
        return Ok((0..spec.fields.len()).collect());
    }
    let mut out = Vec::new();
    for f in list.split(',').map(str::trim).filter(|f| !f.is_empty()) {
        let i = spec.fields.iter().position(|x| *x == f).ok_or_else(|| {
            Error::usage(format!(
                "{} has no field '{}'; its fields are {}",
                spec.name,
                table::escaped(f),
                spec.fields.join(", ")
            ))
        })?;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    if out.is_empty() {
        return Err(Error::usage(format!(
            "-f needs fields: {}, or all",
            spec.fields.join(", ")
        )));
    }
    Ok(out)
}

/// Answers each query in turn: a title line, then a header and rows. A query that fails says why in its title, and
/// the others still answer. Returns whether every query was answered.
pub fn answer(queries: &[Query], out: &mut impl Write) -> io::Result<bool> {
    let mut all = true;
    for q in queries {
        match (q.spec.answer)(q) {
            Ok(found) => {
                write!(out, "# {}", q.asked)?;
                if !found.notes.is_empty() {
                    write!(out, ": {}", found.notes.join("; "))?;
                }
                writeln!(out)?;
                let header: Vec<&str> = q.fields.iter().map(|&i| q.spec.fields[i]).collect();
                writeln!(out, "{}", header.join("\t"))?;
                for row in &found.rows {
                    let cells: Vec<&str> = q.fields.iter().map(|&i| row[i].as_str()).collect();
                    writeln!(out, "{}", cells.join("\t"))?;
                }
            }
            Err(e) => {
                all = false;
                writeln!(out, "# {}: error: {}", q.asked, e.msg)?;
            }
        }
    }
    Ok(all)
}

/// Bytes as one cell of a table: characters a terminal acts on or does not show, and bytes that are not UTF-8,
/// spelled out, so a value can neither break its row nor act on a person's terminal.
fn cell(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.push_str(&table::escaped(chunk.valid()));
        for b in chunk.invalid() {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out
}

/// A word of a query as its title shows it: in quotes when it is empty or holds a space, as it was typed.
fn word(w: &OsStr) -> String {
    let shown = cell(w.as_bytes());
    if shown.is_empty() || shown.contains(' ') {
        format!("'{shown}'")
    } else {
        shown
    }
}

/// Keeps the first `n` items, or all when `n` is 0, and says what was cut.
fn cut<T>(items: &mut Vec<T>, n: usize) -> Option<String> {
    let total = items.len();
    (n > 0 && total > n).then(|| {
        items.truncate(n);
        format!("first {n} of {total}")
    })
}

/// A local time to the minute: `2026-10-17T15:30`.
fn minute(secs: i64) -> String {
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return "?".into();
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// User names by uid, each looked up once. A uid with no name, as in a world, where the password file is hidden,
/// shows as its number.
#[derive(Default)]
struct Users(HashMap<u32, String>);

impl Users {
    fn name(&mut self, uid: u32) -> String {
        self.0
            .entry(uid)
            .or_insert_with(|| user_name(uid).unwrap_or_else(|| uid.to_string()))
            .clone()
    }
}

fn user_name(uid: u32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let rc =
        unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr().cast(), buf.len(), &mut found) };
    if rc != 0 || found.is_null() {
        return None;
    }
    Some(cell(unsafe { CStr::from_ptr(pw.pw_name) }.to_bytes()))
}

fn files(q: &Query) -> Result<Found> {
    let dir = q
        .words
        .first()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let entries = fs::read_dir(&dir).map_err(|e| {
        Error::new(match e.kind() {
            io::ErrorKind::NotFound => "no such folder".to_string(),
            io::ErrorKind::NotADirectory => "not a folder".to_string(),
            io::ErrorKind::PermissionDenied => "not allowed to read it".to_string(),
            _ => e.to_string(),
        })
    })?;
    let mut names: Vec<OsString> = entries.flatten().map(|e| e.file_name()).collect();
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let notes = cut(&mut names, q.rows).into_iter().collect();
    let mut users = Users::default();
    let rows = names
        .iter()
        .map(|name| {
            let shown = cell(name.as_bytes());
            match fs::symlink_metadata(dir.join(name)) {
                Ok(m) => vec![
                    if m.is_dir() { shown + "/" } else { shown },
                    kind(&m.file_type()).to_string(),
                    m.len().to_string(),
                    format!("{:o}", m.mode() & 0o7777),
                    users.name(m.uid()),
                    minute(m.mtime()),
                ],
                // Gone since the folder was read.
                Err(_) => {
                    let mut row = vec!["?".to_string(); 6];
                    row[0] = shown;
                    row
                }
            }
        })
        .collect();
    Ok(Found { rows, notes })
}

fn kind(t: &FileType) -> &'static str {
    if t.is_dir() {
        "dir"
    } else if t.is_file() {
        "file"
    } else if t.is_symlink() {
        "link"
    } else if t.is_fifo() {
        "pipe"
    } else if t.is_socket() {
        "socket"
    } else {
        "device"
    }
}

/// Runs git as a reader in the current folder. It takes no optional locks, so `status` does not rewrite the index;
/// its output is a pipe, so it starts no pager and prints no color. Returns what it printed, or the first line of
/// what it said when it failed.
fn git(args: &[&str], words: &[OsString]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("--no-optional-locks")
        .args(args)
        .args(words)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::new("git is not installed here"),
            _ => Error::new(format!("cannot run git: {e}")),
        })?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let first = said
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("git failed");
        let first = first
            .strip_prefix("fatal: ")
            .or_else(|| first.strip_prefix("error: "))
            .unwrap_or(first);
        return Err(Error::new(table::escaped(first)));
    }
    Ok(out.stdout)
}

/// A changed path in `git status --porcelain=v1 -z`: its two letters, for the index and the working tree, and the
/// path it was renamed or copied from.
#[derive(Debug, PartialEq)]
struct Change {
    path: Vec<u8>,
    from: Option<Vec<u8>>,
    index: u8,
    tree: u8,
}

fn status(q: &Query) -> Result<Found> {
    let out = git(&["status", "--porcelain=v1", "-z", "--branch"], &[])?;
    let (branch, mut changes) = read_status(&out);
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    let mut notes: Vec<String> = branch.into_iter().collect();
    notes.extend(cut(&mut changes, q.rows));
    let rows = changes
        .iter()
        .map(|c| {
            let (change, staged) = change(c.index, c.tree);
            vec![
                cell(&c.path),
                change.to_string(),
                staged.to_string(),
                c.from.as_deref().map_or_else(|| "-".to_string(), cell),
            ]
        })
        .collect();
    Ok(Found { rows, notes })
}

/// Reads `## BRANCH` and the changes from `git status --porcelain=v1 -z --branch`. A rename or a copy names its new
/// path, then, after the next NUL, the old one.
fn read_status(out: &[u8]) -> (Option<String>, Vec<Change>) {
    let mut branch = None;
    let mut changes = Vec::new();
    let mut parts = out.split(|b| *b == 0);
    while let Some(part) = parts.next() {
        if let Some(b) = part.strip_prefix(b"## ") {
            branch = Some(branch_note(&cell(b)));
        } else if part.len() > 3 {
            let (index, tree) = (part[0], part[1]);
            let from = matches!(index, b'R' | b'C') || matches!(tree, b'R' | b'C');
            changes.push(Change {
                path: part[3..].to_vec(),
                from: if from {
                    parts.next().map(<[u8]>::to_vec)
                } else {
                    None
                },
                index,
                tree,
            });
        }
    }
    (branch, changes)
}

/// git's branch line in plain words: `main...origin/main [ahead 1, behind 2]` is `on main, 1 ahead of and 2 behind
/// origin/main`.
fn branch_note(line: &str) -> String {
    if line == "HEAD (no branch)" {
        return "no branch".into();
    }
    if let Some(b) = line
        .strip_prefix("No commits yet on ")
        .or_else(|| line.strip_prefix("Initial commit on "))
    {
        return format!("on {b}, no commits yet");
    }
    let (names, state) = match line.split_once(" [") {
        Some((n, s)) => (n, s.trim_end_matches(']')),
        None => (line, ""),
    };
    let Some((local, upstream)) = names.split_once("...") else {
        return format!("on {names}");
    };
    if state == "gone" {
        return format!("on {local}, {upstream} is gone");
    }
    let (mut ahead, mut behind) = (None, None);
    for part in state.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = Some(n);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = Some(n);
        }
    }
    match (ahead, behind) {
        (Some(a), Some(b)) => format!("on {local}, {a} ahead of and {b} behind {upstream}"),
        (Some(a), None) => format!("on {local}, {a} ahead of {upstream}"),
        (None, Some(b)) => format!("on {local}, {b} behind {upstream}"),
        (None, None) => format!("on {local}, even with {upstream}"),
    }
}

/// What happened to a path, from git's two letters, and whether the change is staged.
fn change(index: u8, tree: u8) -> (&'static str, &'static str) {
    match (index, tree) {
        (b'?', b'?') => ("untracked", "no"),
        (b'!', b'!') => ("ignored", "no"),
        (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => ("conflict", "no"),
        _ => {
            let letter = if tree == b'D' || index == b' ' {
                tree
            } else {
                index
            };
            let word = match letter {
                b'A' => "added",
                b'D' => "deleted",
                b'R' => "renamed",
                b'C' => "copied",
                b'T' => "typechange",
                _ => "modified",
            };
            let staged = match (index != b' ', tree != b' ') {
                (true, false) => "yes",
                (true, true) => "partly",
                (false, _) => "no",
            };
            (word, staged)
        }
    }
}

fn log(q: &Query) -> Result<Found> {
    let mut args = vec![
        "log",
        "--no-color",
        "--no-show-signature",
        "--no-ext-diff",
        "--no-textconv",
        "--date=format-local:%Y-%m-%dT%H:%M",
        // The subject last: it may hold anything, so it takes the rest of the line.
        "--format=%x1e%h%x1f%ad%x1f%an%x1f%s",
    ];
    // One more than asked, to tell whether there were more.
    let max = format!("--max-count={}", q.rows + 1);
    if q.rows > 0 {
        args.push(&max);
    }
    if ["files", "added", "removed"].iter().any(|f| q.wants(f)) {
        args.push("--numstat");
    }
    args.push("--end-of-options");
    let mut rows = read_log(&git(&args, &q.words)?);
    let mut notes = Vec::new();
    if q.rows > 0 && rows.len() > q.rows {
        rows.truncate(q.rows);
        notes.push(format!("first {}, and more", q.rows));
    }
    Ok(Found { rows, notes })
}

/// Reads commits printed as `\x1e` hash `\x1f` date `\x1f` author `\x1f` subject, each followed by its `--numstat`
/// lines when asked for: lines added, lines removed and the path, separated by tabs, `-` for a binary file.
fn read_log(out: &[u8]) -> Vec<Vec<String>> {
    out.split(|b| *b == 0x1e)
        .skip(1)
        .map(|commit| {
            let mut lines = commit.split(|b| *b == b'\n');
            let head: Vec<String> = lines
                .next()
                .unwrap_or_default()
                .splitn(4, |b| *b == 0x1f)
                .map(cell)
                .collect();
            let field = |i: usize| head.get(i).cloned().unwrap_or_default();
            let (mut files, mut added, mut removed) = (0u64, 0u64, 0u64);
            for line in lines.filter(|l| !l.is_empty()) {
                let parts: Vec<&[u8]> = line.splitn(3, |b| *b == b'\t').collect();
                if let [a, r, _path] = parts[..] {
                    let n = |v: &[u8]| {
                        std::str::from_utf8(v)
                            .ok()
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(0)
                    };
                    files += 1;
                    added += n(a);
                    removed += n(r);
                }
            }
            vec![
                field(0),
                field(3),
                field(1),
                field(2),
                files.to_string(),
                added.to_string(),
                removed.to_string(),
            ]
        })
        .collect()
}

/// What `/proc/PID/stat` says of a process. Its name comes in parentheses and may hold spaces and parentheses
/// itself, so the fields are counted from the last `)`.
#[derive(Debug, PartialEq)]
struct Stat {
    name: Vec<u8>,
    parent: u32,
    cpu_ticks: u64,
    start_ticks: u64,
    rss_pages: u64,
}

fn read_stat(s: &[u8]) -> Option<Stat> {
    let open = s.iter().position(|b| *b == b'(')?;
    let close = s.iter().rposition(|b| *b == b')')?;
    let rest = std::str::from_utf8(s.get(close + 1..)?).ok()?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    // After the name: state (field 3 in proc(5)), parent (4), ..., utime (14), stime (15), ..., starttime (22),
    // vsize (23), rss (24).
    let n = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok());
    Some(Stat {
        name: s.get(open + 1..close)?.to_vec(),
        parent: n(1)? as u32,
        cpu_ticks: n(11)? + n(12)?,
        start_ticks: n(19)?,
        rss_pages: n(21)?,
    })
}

/// When the computer started, in seconds since 1970, from `/proc/stat`.
fn boot_time() -> i64 {
    fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())
        })
        .unwrap_or(0)
}

fn holds(hay: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

fn procs(q: &Query) -> Result<Found> {
    let text = q.words.first().map(|t| t.as_bytes());
    let tick = (unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).max(1) as u64;
    let page = (unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).max(1) as u64;
    let boot = boot_time();
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc")?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        // A process may end between the listing and the reads: it is left out.
        let Some(stat) = fs::read(entry.path().join("stat"))
            .ok()
            .and_then(|s| read_stat(&s))
        else {
            continue;
        };
        let args = fs::read(entry.path().join("cmdline")).unwrap_or_default();
        let cmd = args
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .collect::<Vec<_>>()
            .join(&b' ');
        let uid = entry.metadata().map(|m| m.uid()).ok();
        found.push((pid, stat, cmd, uid));
    }
    // This process is not what was asked for, and with TEXT, neither is the shell that runs this query: up the chain
    // of parents, the one whose command holds `look` and `procs` holds TEXT only because it holds the query.
    let me = std::process::id();
    let parents: HashMap<u32, u32> = found.iter().map(|p| (p.0, p.1.parent)).collect();
    let mut above = Vec::new();
    let mut up = me;
    while let Some(&parent) = parents.get(&up) {
        if parent <= 1 || above.contains(&parent) {
            break;
        }
        above.push(parent);
        up = parent;
    }
    found.retain(|(pid, stat, cmd, _)| {
        *pid != me
            && text.is_none_or(|t| {
                (holds(cmd, t) || holds(&stat.name, t))
                    && !(above.contains(pid) && holds(cmd, b"look") && holds(cmd, b"procs"))
            })
    });
    found.sort_by(|a, b| b.1.rss_pages.cmp(&a.1.rss_pages).then(a.0.cmp(&b.0)));
    let notes = cut(&mut found, q.rows).into_iter().collect();
    let mut users = Users::default();
    let rows = found
        .iter()
        .map(|(pid, stat, cmd, uid)| {
            let name = cell(&stat.name);
            vec![
                pid.to_string(),
                (stat.rss_pages * page).to_string(),
                name.clone(),
                stat.parent.to_string(),
                uid.map_or_else(|| "?".to_string(), |u| users.name(u)),
                format!("{:.2}", stat.cpu_ticks as f64 / tick as f64),
                minute(boot + (stat.start_ticks / tick) as i64),
                // A kernel thread has no command line: it shows as its name in brackets, as ps shows it.
                if cmd.is_empty() {
                    format!("[{name}]")
                } else {
                    cell(cmd)
                },
            ]
        })
        .collect();
    Ok(Found { rows, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<OsString> {
        line.split(' ').map(OsString::from).collect()
    }

    fn asked(q: &Query) -> (String, Vec<&str>, usize) {
        let fields = q.fields.iter().map(|&i| q.spec.fields[i]).collect();
        (q.asked.clone(), fields, q.rows)
    }

    #[test]
    fn reads_queries_fields_rows_and_plus() {
        let qs = parse(args(
            "files src + log -n 5 -f hash,date,hash HEAD x + procs -f all",
        ))
        .unwrap();
        assert_eq!(
            qs.iter().map(asked).collect::<Vec<_>>(),
            [
                ("files src".into(), vec!["name"], 100),
                (
                    "log -n 5 -f hash,date,hash HEAD x".into(),
                    vec!["hash", "date"],
                    5
                ),
                (
                    "procs -f all".into(),
                    vec![
                        "pid", "mem", "name", "parent", "user", "cpu", "started", "cmd"
                    ],
                    30
                ),
            ]
        );
        assert_eq!(qs[0].words, ["src"]);
        assert_eq!(qs[1].words, ["HEAD", "x"]);
        let q = &parse(args("status --fields=change,path -n0")).unwrap()[0];
        assert_eq!(asked(q).1, ["change", "path"]);
        assert_eq!(q.rows, 0);
        // A mistake anywhere is a usage error, before anything runs.
        for bad in [
            "file",
            "files a b",
            "status x",
            "files -f nope",
            "files -f ,",
            "log -n x",
            "files + + log",
            "files +",
            "procs -x",
        ] {
            let e = parse(args(bad)).err().unwrap_or_else(|| panic!("{bad}"));
            assert_eq!(e.code, crate::error::USAGE, "{bad}: {}", e.msg);
        }
        let e = parse(args("files -f name,colour")).err().unwrap();
        assert_eq!(
            e.msg,
            "files has no field 'colour'; its fields are name, type, size, mode, owner, modified"
        );
        assert_eq!(
            parse(args("files a b")).err().unwrap().msg,
            "files: unexpected argument \"b\""
        );
    }

    #[test]
    fn prints_titles_tab_separated_rows_and_failures() {
        let mut out = Vec::new();
        let dir = std::env::temp_dir().join(format!("arbor-look-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a\tb\n"), "12345").unwrap();
        fs::write(dir.join("z"), "").unwrap();
        std::os::unix::fs::symlink("z", dir.join("link")).unwrap();
        let d = dir.to_str().unwrap();
        let qs = parse(args(&format!(
            "files {d} -n 3 + files {d}/none + files {d} -f name,type,size"
        )))
        .unwrap();
        assert!(!answer(&qs, &mut out).unwrap());
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "# files {d} -n 3: first 3 of 4\nname\na\\tb\\n\nlink\nsub/\n\
                 # files {d}/none: error: no such folder\n\
                 # files {d} -f name,type,size\nname\ttype\tsize\n\
                 a\\tb\\n\tfile\t5\nlink\tlink\t1\nsub/\tdir\t{}\nz\tfile\t0\n",
                fs::metadata(dir.join("sub")).unwrap().len()
            )
        );
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(cell(b"caf\xc3\xa9 \xff\x1b[2J"), "café \\xff\\e[2J");
    }

    #[test]
    fn reads_git_status_in_plain_words() {
        let out =
            b"## main...origin/main [ahead 1, behind 2]\0 M src/a.rs\0A  new.rs\0AM half.rs\0\
                    R  to.rs\0from.rs\0?? notes.txt\0UU both.rs\0 D gone.rs\0";
        let (branch, changes) = read_status(out);
        assert_eq!(
            branch.as_deref(),
            Some("on main, 1 ahead of and 2 behind origin/main")
        );
        let words: Vec<(String, &str, &str, Option<String>)> = changes
            .iter()
            .map(|c| {
                let (w, s) = change(c.index, c.tree);
                (
                    String::from_utf8_lossy(&c.path).into_owned(),
                    w,
                    s,
                    c.from
                        .as_ref()
                        .map(|f| String::from_utf8_lossy(f).into_owned()),
                )
            })
            .collect();
        assert_eq!(
            words,
            [
                ("src/a.rs".into(), "modified", "no", None),
                ("new.rs".into(), "added", "yes", None),
                ("half.rs".into(), "added", "partly", None),
                ("to.rs".into(), "renamed", "yes", Some("from.rs".into())),
                ("notes.txt".into(), "untracked", "no", None),
                ("both.rs".into(), "conflict", "no", None),
                ("gone.rs".into(), "deleted", "no", None),
            ]
        );
        assert_eq!(change(b'M', b'D'), ("deleted", "partly"));
        for (line, note) in [
            ("main", "on main"),
            ("main...origin/main", "on main, even with origin/main"),
            (
                "main...origin/main [ahead 3]",
                "on main, 3 ahead of origin/main",
            ),
            (
                "main...origin/main [behind 1]",
                "on main, 1 behind origin/main",
            ),
            ("main...origin/main [gone]", "on main, origin/main is gone"),
            ("HEAD (no branch)", "no branch"),
            ("No commits yet on main", "on main, no commits yet"),
        ] {
            assert_eq!(branch_note(line), note);
        }
    }

    #[test]
    fn reads_commits_and_the_lines_they_changed() {
        let out = b"\x1eabc1234\x1f2026-10-17T15:30\x1fAna\x1ffix: a\x1fb\tc\n\n3\t1\tsrc/a.rs\n-\t-\tlogo.png\n\
                    \x1edef5678\x1f2026-10-16T09:00\x1fBo\x1fMerge branch x\n";
        assert_eq!(
            read_log(out),
            [
                vec![
                    "abc1234",
                    "fix: a\\x1fb\\tc",
                    "2026-10-17T15:30",
                    "Ana",
                    "2",
                    "3",
                    "1"
                ],
                vec![
                    "def5678",
                    "Merge branch x",
                    "2026-10-16T09:00",
                    "Bo",
                    "0",
                    "0",
                    "0"
                ],
            ]
        );
    }

    #[test]
    fn reads_a_process_from_proc() {
        let stat = b"42 (a (b) c) S 7 42 42 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 1 0 1234 9000000 300 rest";
        assert_eq!(
            read_stat(stat),
            Some(Stat {
                name: b"a (b) c".to_vec(),
                parent: 7,
                cpu_ticks: 300,
                start_ticks: 1234,
                rss_pages: 300,
            })
        );
        assert_eq!(read_stat(b"42 (x) S 7"), None);
        // This test's own process, read as `procs` reads every other one.
        let me = fs::read(format!("/proc/{}/stat", std::process::id())).unwrap();
        let me = read_stat(&me).unwrap();
        assert!(me.rss_pages > 0 && me.parent > 0);
        assert!(boot_time() > 1_000_000_000);
    }
}
