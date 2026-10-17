//! The definitions in a source file and the lines each spans, for `arbor look outline`. Long files are where agents
//! waste the most reading: in 40 large files, the outline was 4% of a file's tokens, and the outline with two
//! functions 8% (OPTIMIZATION.md §2). Each language's reader goes line by line, with no parser: it blanks out the
//! file's comments and strings, finds definitions by their keywords at the start of a line, and finds where each ends
//! by its braces (Rust, JavaScript and TypeScript, shell), its indentation (Python) or the next heading of its level
//! (Markdown).
//!
//! What reading line by line misses or gets wrong:
//! - definitions made by macros, written after other code on their line, or inside a function's body, an object or
//!   a call (a test suite's `describe`, a route's handler); in Rust, the functions of an `extern` block;
//! - in JavaScript and TypeScript, a `/` that could start a regular expression or divide, and a quote in JSX text,
//!   which can end a definition early or late; and a function called where it is written, as compilers write classes
//!   (`var A = function () {…}()`), which is listed as a function;
//! - in Python, an f-string that holds its own quotes;
//! - in shell, a function whose body is in parentheses;
//! - in Markdown, headings inside HTML, and all but the last line of a setext heading's text.

use std::path::Path;

/// A language that `outline` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Shell,
    Markdown,
}

/// The languages, as an error names them.
pub const LANGUAGES: &str = "Rust (.rs), Python (.py), JavaScript (.js .jsx .mjs .cjs), TypeScript (.ts .tsx .mts \
                             .cts), shell (.sh .bash) and Markdown (.md), and scripts whose #! line names python, \
                             node or a shell";

/// A file's language, by its extension, or by the program its `#!` line runs.
pub fn language(path: &Path, text: &[u8]) -> Option<Lang> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let by_ext = match ext.as_deref() {
        Some("rs") => Some(Lang::Rust),
        Some("py" | "pyi") => Some(Lang::Python),
        Some("js" | "jsx" | "mjs" | "cjs") => Some(Lang::JavaScript),
        Some("ts" | "tsx" | "mts" | "cts") => Some(Lang::TypeScript),
        Some("sh" | "bash") => Some(Lang::Shell),
        Some("md" | "markdown") => Some(Lang::Markdown),
        _ => None,
    };
    by_ext.or_else(|| interpreter(text))
}

/// A script's language, from its `#!` line: `#!/bin/sh`, `#!/usr/bin/env python3`, `#!/usr/bin/env -S node -e`.
fn interpreter(text: &[u8]) -> Option<Lang> {
    let first = text.split(|b| *b == b'\n').next()?.strip_prefix(b"#!")?;
    let program = first
        .split(u8::is_ascii_whitespace)
        .filter(|w| !w.is_empty())
        .map(|w| w.rsplit(|b| *b == b'/').next().unwrap_or(w))
        .find(|w| *w != b"env" && !w.starts_with(b"-"))?;
    // python3.14 is python.
    let name: Vec<u8> = program
        .iter()
        .copied()
        .take_while(|b| !b.is_ascii_digit() && *b != b'.')
        .collect();
    match name.as_slice() {
        b"python" => Some(Lang::Python),
        b"node" | b"nodejs" => Some(Lang::JavaScript),
        b"sh" | b"bash" | b"dash" | b"ash" | b"ksh" | b"zsh" => Some(Lang::Shell),
        _ => None,
    }
}

/// A definition, and the lines it spans, counted from 1: from the comments, attributes and decorators directly above
/// it to its last line.
#[derive(Debug, PartialEq)]
pub struct Def {
    pub line: usize,
    pub end: usize,
    /// The language's word for it: `fn`, `class`, `struct`, `h2`...
    pub kind: &'static str,
    pub name: Vec<u8>,
    /// The definition it sits in, as an index into the outline.
    pub parent: Option<usize>,
    /// Its head on one line, up to where its body or value starts: `pub fn parse(args: Vec<OsString>) -> Result<..>`.
    pub head: Vec<u8>,
}

/// The definitions in `text`, in the order they start.
pub fn read(lang: Lang, text: &[u8]) -> Vec<Def> {
    let raws = split(text);
    match lang {
        Lang::Markdown => markdown(&raws),
        Lang::Python => python(&lines(&raws, python_text(&raws))),
        Lang::Rust => braces(&lines(&raws, rust_text(&raws)), lang),
        Lang::JavaScript | Lang::TypeScript => braces(&lines(&raws, js_text(&raws)), lang),
        Lang::Shell => braces(&lines(&raws, shell_text(&raws)), lang),
    }
}

/// A file's lines, without their `\n` or `\r\n`.
pub fn split(text: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = text
        .split(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .collect();
    if text.is_empty() || text.ends_with(b"\n") {
        lines.pop();
    }
    lines
}

/// What a byte of a line is: code, inside a string, or inside a comment.
#[derive(Clone, Copy, Debug, PartialEq)]
enum K {
    Code,
    Text,
    Note,
}

/// A line as the readers see it.
struct Line<'a> {
    raw: &'a [u8],
    /// The line with its strings and comments blanked out, byte for byte.
    code: Vec<u8>,
    /// The line with its comments blanked out.
    head: Vec<u8>,
    /// It starts inside a string or a here-document: the line before it goes on into it.
    open: bool,
}

impl Line<'_> {
    /// A line of comments only, which may say what the definition below it is. A script's `#!` line, the first,
    /// does not.
    fn comment(&self, n: usize) -> bool {
        self.head.trim_ascii().is_empty()
            && !self.raw.trim_ascii().is_empty()
            && !(n == 0 && self.raw.starts_with(b"#!"))
    }
}

fn lines<'a>(raws: &[&'a [u8]], kinds: Vec<(Vec<K>, bool)>) -> Vec<Line<'a>> {
    raws.iter()
        .zip(kinds)
        .map(|(raw, (k, open))| Line {
            raw,
            code: raw
                .iter()
                .zip(&k)
                .map(|(&b, &x)| if x == K::Code { b } else { b' ' })
                .collect(),
            head: raw
                .iter()
                .zip(&k)
                .map(|(&b, &x)| if x == K::Note { b' ' } else { b })
                .collect(),
            open,
        })
        .collect()
}

fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Marks the bytes `from..to` of a line, as far as it goes.
fn mark(k: &mut [K], from: usize, to: usize, x: K) {
    let to = to.min(k.len());
    if from < to {
        k[from..to].fill(x);
    }
}

/// Rust: `//` and nested `/* */` comments, strings with escapes, raw strings (`r#"…"#`), and char literals, told
/// apart from lifetimes (`'a'` is a char, `'a` a lifetime).
fn rust_text(raws: &[&[u8]]) -> Vec<(Vec<K>, bool)> {
    enum S {
        Code,
        Block(u32),
        /// In a string; for a raw one, the number of `#` that end it.
        Str(Option<usize>),
    }
    let mut s = S::Code;
    raws.iter()
        .map(|l| {
            let open = matches!(s, S::Str(_));
            let mut k = vec![K::Code; l.len()];
            let mut i = 0;
            while i < l.len() {
                let b = l[i];
                let next = l.get(i + 1).copied();
                match s {
                    S::Block(depth) => {
                        k[i] = K::Note;
                        if b == b'*' && next == Some(b'/') {
                            mark(&mut k, i, i + 2, K::Note);
                            i += 2;
                            s = if depth > 1 {
                                S::Block(depth - 1)
                            } else {
                                S::Code
                            };
                            continue;
                        }
                        if b == b'/' && next == Some(b'*') {
                            mark(&mut k, i, i + 2, K::Note);
                            i += 2;
                            s = S::Block(depth + 1);
                            continue;
                        }
                    }
                    S::Str(raw) => {
                        k[i] = K::Text;
                        if raw.is_none() && b == b'\\' {
                            mark(&mut k, i, i + 2, K::Text);
                            i += 2;
                            continue;
                        }
                        let hashes = raw.unwrap_or(0);
                        if b == b'"'
                            && l[i + 1..].iter().take_while(|b| **b == b'#').count() >= hashes
                        {
                            mark(&mut k, i, i + 1 + hashes, K::Text);
                            i += 1 + hashes;
                            s = S::Code;
                            continue;
                        }
                    }
                    S::Code => {
                        if b == b'/' && next == Some(b'/') {
                            mark(&mut k, i, l.len(), K::Note);
                            break;
                        }
                        if b == b'/' && next == Some(b'*') {
                            mark(&mut k, i, i + 2, K::Note);
                            i += 2;
                            s = S::Block(1);
                            continue;
                        }
                        if b == b'"' {
                            k[i] = K::Text;
                            s = S::Str(None);
                        } else if b == b'\'' {
                            if let Some(n) = char_literal(&l[i..]) {
                                mark(&mut k, i, i + n, K::Text);
                                i += n;
                                continue;
                            }
                        } else if matches!(b, b'r' | b'b' | b'c')
                            && (i == 0 || !word_byte(l[i - 1]))
                        {
                            // r"…", r#"…"#, br"…", cr"…"
                            let at = if b == b'r' {
                                i + 1
                            } else if next == Some(b'r') {
                                i + 2
                            } else {
                                l.len()
                            };
                            let hashes = l[at.min(l.len())..]
                                .iter()
                                .take_while(|b| **b == b'#')
                                .count();
                            if at < l.len() && l.get(at + hashes) == Some(&b'"') {
                                mark(&mut k, i, at + hashes + 1, K::Text);
                                i = at + hashes + 1;
                                s = S::Str(Some(hashes));
                                continue;
                            }
                        }
                    }
                }
                i += 1;
            }
            (k, open)
        })
        .collect()
}

/// The length of the char literal that starts `s` (`'x'`, `'\n'`, `'\u{1F600}'`), or None for a lifetime or a label.
fn char_literal(s: &[u8]) -> Option<usize> {
    if s.get(1) == Some(&b'\\') {
        let close = s.iter().skip(3).take(10).position(|b| *b == b'\'')?;
        return Some(close + 4);
    }
    let width = match *s.get(1)? {
        b if b < 0x80 => 1,
        b if b >= 0xf0 => 4,
        b if b >= 0xe0 => 3,
        _ => 2,
    };
    (s.get(1 + width) == Some(&b'\'')).then_some(width + 2)
}

/// Python: `#` comments, and strings in one or three quotes, with any prefix and escapes. A string in one quote ends
/// with its line, unless the line ends in a backslash.
fn python_text(raws: &[&[u8]]) -> Vec<(Vec<K>, bool)> {
    // In a string: its quote, and whether it is tripled.
    let mut s: Option<(u8, bool)> = None;
    raws.iter()
        .map(|l| {
            let open = s.is_some();
            let mut k = vec![K::Code; l.len()];
            let mut i = 0;
            while i < l.len() {
                let b = l[i];
                match s {
                    Some((q, triple)) => {
                        k[i] = K::Text;
                        if b == b'\\' {
                            mark(&mut k, i, i + 2, K::Text);
                            i += 2;
                            continue;
                        }
                        if b == q && (!triple || l[i..].starts_with(&[q, q, q])) {
                            let n = if triple { 3 } else { 1 };
                            mark(&mut k, i, i + n, K::Text);
                            i += n;
                            s = None;
                            continue;
                        }
                    }
                    None => {
                        if b == b'#' {
                            mark(&mut k, i, l.len(), K::Note);
                            break;
                        }
                        if b == b'"' || b == b'\'' {
                            let triple = l[i..].starts_with(&[b, b, b]);
                            let n = if triple { 3 } else { 1 };
                            mark(&mut k, i, i + n, K::Text);
                            i += n;
                            s = Some((b, triple));
                            continue;
                        }
                    }
                }
                i += 1;
            }
            if s.is_some_and(|(_, triple)| !triple) && !l.ends_with(b"\\") {
                s = None;
            }
            (k, open)
        })
        .collect()
}

/// JavaScript and TypeScript: `//` and `/* */` comments, strings, template literals with the code in their `${}`,
/// and regular expressions where a value cannot come before the `/`.
fn js_text(raws: &[&[u8]]) -> Vec<(Vec<K>, bool)> {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        Code,
        Block,
        Str(u8),
        Template,
    }
    let mut s = S::Code;
    // The `${` open in template literals, each with the braces opened in it since.
    let mut subs: Vec<u32> = Vec::new();
    raws.iter()
        .map(|l| {
            let open = matches!(s, S::Str(_) | S::Template) || !subs.is_empty();
            let mut k = vec![K::Code; l.len()];
            let mut i = 0;
            while i < l.len() {
                let b = l[i];
                let next = l.get(i + 1).copied();
                match s {
                    S::Block => {
                        k[i] = K::Note;
                        if b == b'*' && next == Some(b'/') {
                            mark(&mut k, i, i + 2, K::Note);
                            i += 2;
                            s = S::Code;
                            continue;
                        }
                    }
                    S::Str(q) => {
                        k[i] = K::Text;
                        if b == b'\\' {
                            mark(&mut k, i, i + 2, K::Text);
                            i += 2;
                            continue;
                        }
                        if b == q {
                            s = S::Code;
                        }
                    }
                    S::Template => {
                        k[i] = K::Text;
                        if b == b'\\' {
                            mark(&mut k, i, i + 2, K::Text);
                            i += 2;
                            continue;
                        }
                        if b == b'`' {
                            s = S::Code;
                        } else if b == b'$' && next == Some(b'{') {
                            mark(&mut k, i, i + 2, K::Text);
                            i += 2;
                            subs.push(0);
                            s = S::Code;
                            continue;
                        }
                    }
                    S::Code => match b {
                        b'/' if next == Some(b'/') => {
                            mark(&mut k, i, l.len(), K::Note);
                            break;
                        }
                        b'/' if next == Some(b'*') => {
                            mark(&mut k, i, i + 2, K::Note);
                            i += 2;
                            s = S::Block;
                            continue;
                        }
                        b'/' if regex_may_start(l, &k, i) => {
                            if let Some(n) = regex(&l[i..]) {
                                mark(&mut k, i, i + n, K::Text);
                                i += n;
                                continue;
                            }
                        }
                        b'"' | b'\'' => {
                            k[i] = K::Text;
                            s = S::Str(b);
                        }
                        b'`' => {
                            k[i] = K::Text;
                            s = S::Template;
                        }
                        b'{' => {
                            if let Some(n) = subs.last_mut() {
                                *n += 1;
                            }
                        }
                        b'}' => match subs.last_mut() {
                            Some(0) => {
                                subs.pop();
                                k[i] = K::Text;
                                s = S::Template;
                            }
                            Some(n) => *n -= 1,
                            None => {}
                        },
                        _ => {}
                    },
                }
                i += 1;
            }
            if matches!(s, S::Str(_)) && !l.ends_with(b"\\") {
                s = S::Code;
            }
            (k, open)
        })
        .collect()
}

/// Whether a `/` at `i` can start a regular expression: when no value comes before it on its line, so it cannot
/// divide.
fn regex_may_start(l: &[u8], k: &[K], i: usize) -> bool {
    let Some(p) = (0..i)
        .rev()
        .find(|&j| k[j] != K::Note && !l[j].is_ascii_whitespace())
    else {
        return true;
    };
    if k[p] == K::Text {
        return false;
    }
    if word_byte(l[p]) {
        let start = (0..=p)
            .rev()
            .take_while(|&j| word_byte(l[j]))
            .last()
            .unwrap_or(p);
        return matches!(
            &l[start..=p],
            b"return"
                | b"typeof"
                | b"case"
                | b"do"
                | b"else"
                | b"in"
                | b"of"
                | b"new"
                | b"delete"
                | b"void"
                | b"throw"
                | b"yield"
                | b"await"
                | b"instanceof"
        );
    }
    b"(,=:[!&|?{};+-*%<>~^".contains(&l[p])
}

/// The length of the regular expression that starts `s`, to its closing `/`, if it closes on this line.
fn regex(s: &[u8]) -> Option<usize> {
    let mut class = false;
    let mut i = 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => return Some(i + 1),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Shell: `#` comments where a word starts, strings in quotes, `$'…'`, escaped characters, and here-documents, whose
/// lines are all text.
fn shell_text(raws: &[&[u8]]) -> Vec<(Vec<K>, bool)> {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        Code,
        Single,
        Ansi,
        Double,
    }
    let mut s = S::Code;
    // Here-documents: the ones named on the line being read, and the one whose lines come now, by the word on their
    // last line and whether `<<-` lets that line start with tabs.
    let mut named: Vec<(Vec<u8>, bool)> = Vec::new();
    let mut doc: Option<(Vec<u8>, bool)> = None;
    raws.iter()
        .map(|l| {
            if let Some((word, tabs)) = &doc {
                let last = if *tabs {
                    &l[l.iter().take_while(|b| **b == b'\t').count()..]
                } else {
                    l
                };
                if last == word.as_slice() {
                    doc = (!named.is_empty()).then(|| named.remove(0));
                }
                return (vec![K::Text; l.len()], true);
            }
            let open = s != S::Code;
            let mut k = vec![K::Code; l.len()];
            let mut i = 0;
            while i < l.len() {
                let b = l[i];
                match s {
                    S::Single => {
                        k[i] = K::Text;
                        if b == b'\'' {
                            s = S::Code;
                        }
                    }
                    S::Ansi | S::Double => {
                        k[i] = K::Text;
                        if b == b'\\' {
                            mark(&mut k, i, i + 2, K::Text);
                            i += 2;
                            continue;
                        }
                        if (s == S::Ansi && b == b'\'') || (s == S::Double && b == b'"') {
                            s = S::Code;
                        }
                    }
                    S::Code => match b {
                        b'\\' => {
                            mark(&mut k, i, i + 2, K::Text);
                            i += 2;
                            continue;
                        }
                        b'#' if i == 0 || b" \t;&|()".contains(&l[i - 1]) => {
                            mark(&mut k, i, l.len(), K::Note);
                            break;
                        }
                        b'\'' => {
                            k[i] = K::Text;
                            s = if i > 0 && l[i - 1] == b'$' {
                                S::Ansi
                            } else {
                                S::Single
                            };
                        }
                        b'"' => {
                            k[i] = K::Text;
                            s = S::Double;
                        }
                        b'<' if l[i..].starts_with(b"<<")
                            && !l[i..].starts_with(b"<<<")
                            && (i == 0 || l[i - 1] != b'<') =>
                        {
                            if let Some((n, word, tabs)) = heredoc(&l[i..]) {
                                mark(&mut k, i, i + n, K::Text);
                                named.push((word, tabs));
                                i += n;
                                continue;
                            }
                        }
                        _ => {}
                    },
                }
                i += 1;
            }
            if !named.is_empty() {
                doc = Some(named.remove(0));
            }
            (k, open)
        })
        .collect()
}

/// A here-document's `<<WORD`, `<<-WORD`, `<<'WORD'` or `<<"WORD"`: its length, its word, and whether its last line
/// may start with tabs.
fn heredoc(s: &[u8]) -> Option<(usize, Vec<u8>, bool)> {
    let mut i = 2;
    let tabs = s.get(i) == Some(&b'-');
    if tabs {
        i += 1;
    }
    while s.get(i).is_some_and(|b| *b == b' ' || *b == b'\t') {
        i += 1;
    }
    let quote = s
        .get(i)
        .copied()
        .filter(|b| matches!(b, b'\'' | b'"' | b'\\'));
    if quote.is_some() {
        i += 1;
    }
    let start = i;
    while s
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        i += 1;
    }
    let word = s[start..i].to_vec();
    if word.is_empty() || (quote.is_none() && word[0].is_ascii_digit()) {
        return None;
    }
    if let Some(q) = quote.filter(|q| *q != b'\\') {
        if s.get(i) != Some(&q) {
            return None;
        }
        i += 1;
    }
    Some((i, word, tabs))
}

/// A cursor over a line's code, word by word.
struct Words<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Words<'a> {
    fn new(s: &'a [u8]) -> Self {
        Words { s, i: 0 }
    }

    fn space(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    /// The next byte that is not a space.
    fn peek(&mut self) -> Option<u8> {
        self.space();
        self.s.get(self.i).copied()
    }

    /// The next word, which the cursor moves past.
    /// A name, or a Rust raw one, as `r#try`.
    fn ident(&mut self) -> Option<&'a [u8]> {
        self.space();
        let start = self.i;
        if self.s[start..].starts_with(b"r#")
            && self.s.get(start + 2).is_some_and(|b| word_byte(*b))
        {
            self.i += 2;
        }
        while self.s.get(self.i).is_some_and(|b| word_byte(*b)) {
            self.i += 1;
        }
        (self.i > start).then(|| &self.s[start..self.i])
    }

    fn peek_ident(&mut self) -> Option<&'a [u8]> {
        let at = self.i;
        let w = self.ident();
        self.i = at;
        w
    }

    /// Moves past `w` when it is the next word.
    fn word(&mut self, w: &[u8]) -> bool {
        let at = self.i;
        if self.ident() == Some(w) {
            return true;
        }
        self.i = at;
        false
    }

    /// Moves past `b` when it is the next byte that is not a space.
    fn byte(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.i += 1;
            return true;
        }
        false
    }

    /// Moves past the group in parentheses, brackets or angle brackets that starts here, and all it holds.
    fn group(&mut self) -> bool {
        let Some(open) = self.peek().filter(|b| b"([<".contains(b)) else {
            return false;
        };
        let close = match open {
            b'(' => b')',
            b'[' => b']',
            _ => b'>',
        };
        let mut depth = 0;
        while let Some(&b) = self.s.get(self.i) {
            self.i += 1;
            if b == open {
                depth += 1;
            } else if b == close
                && !(close == b'>' && matches!(self.s.get(self.i - 2), Some(b'-' | b'=')))
            {
                depth -= 1;
                if depth == 0 {
                    return true;
                }
            }
        }
        false
    }

    fn rest(&self) -> &'a [u8] {
        &self.s[self.i..]
    }
}

/// Whether the `=` at `i` gives a value, and is not part of `==`, `=>`, `<=`, `!=`, `+=` and such.
fn assigns(code: &[u8], i: usize) -> bool {
    code[i] == b'='
        && !matches!(code.get(i + 1), Some(b'=' | b'>'))
        && !(i > 0 && b"=!<>+-*/%&|^:".contains(&code[i - 1]))
}

/// A name in capitals, as constants are given in Python and JavaScript: `MAX_ROWS`.
fn capitals(name: &[u8]) -> bool {
    name.iter().any(u8::is_ascii_uppercase)
        && name
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
}

/// What tells where a definition's body ends.
#[derive(Clone, Copy, PartialEq)]
enum Body {
    /// Braces. A `;` before them ends it, as in `struct S;` and `fn f();`.
    Braces,
    /// A value up to a `;`, as Rust's `const`, `static` and `type`.
    Value,
    /// A JavaScript statement, which ends at a `;`, or with a line the next one does not go on from. A function's
    /// braces hold its body.
    Statement { function: bool },
}

/// Which definitions a scope holds: a module's, or a class's members.
#[derive(Clone, Copy, PartialEq)]
enum Holds {
    Items,
    Members,
}

/// A definition found at the start of a line.
struct Item {
    kind: &'static str,
    name: Vec<u8>,
    body: Body,
    holds: Option<Holds>,
}

fn item(kind: &'static str, name: &[u8], body: Body, holds: Option<Holds>) -> Item {
    Item {
        kind,
        name: name.to_vec(),
        body,
        holds,
    }
}

/// Rust's items: `fn`, `struct`, `enum`, `union`, `trait`, `impl`, `mod`, `type`, `const`, `static` and
/// `macro_rules!`, after `pub(…)` and qualifiers such as `async`, `const` and `unsafe`.
fn rust_item(code: &[u8]) -> Option<Item> {
    let mut w = Words::new(code);
    if w.word(b"pub") && w.peek() == Some(b'(') {
        w.group();
    }
    loop {
        let at = w.i;
        match w.ident() {
            Some(b"default" | b"async" | b"unsafe" | b"safe" | b"auto" | b"extern") => {}
            Some(b"const")
                if matches!(
                    w.peek_ident(),
                    Some(b"fn" | b"async" | b"unsafe" | b"extern")
                ) => {}
            _ => {
                w.i = at;
                break;
            }
        }
    }
    let (kind, body, holds) = match w.ident()? {
        b"fn" => ("fn", Body::Braces, None),
        b"struct" => ("struct", Body::Braces, None),
        b"enum" => ("enum", Body::Braces, None),
        b"union" => ("union", Body::Braces, None),
        b"trait" => ("trait", Body::Braces, Some(Holds::Items)),
        // Its name comes from its head, once read.
        b"impl" => return Some(item("impl", b"", Body::Braces, Some(Holds::Items))),
        b"mod" => ("mod", Body::Braces, Some(Holds::Items)),
        b"type" => ("type", Body::Value, None),
        b"const" => ("const", Body::Value, None),
        b"static" => {
            w.word(b"mut");
            ("static", Body::Value, None)
        }
        b"macro_rules" if w.byte(b'!') => ("macro", Body::Braces, None),
        _ => return None,
    };
    Some(item(kind, w.ident()?, body, holds))
}

/// An `impl`'s name, from its head: `impl<T: Copy> From<T> for Wrapper<T> where …` is `From<T> for Wrapper<T>`.
fn impl_name(head: &[u8]) -> Vec<u8> {
    let mut w = Words::new(head);
    while !w.word(b"impl") {
        if w.ident().is_none() {
            return head.to_vec();
        }
    }
    if w.peek() == Some(b'<') {
        w.group();
    }
    // `impl const Trait for T`, in nightly Rust.
    w.word(b"const");
    let rest = w.rest().trim_ascii();
    let cut = rest
        .windows(7)
        .position(|x| x == b" where ")
        .unwrap_or(rest.len());
    rest[..cut].trim_ascii().to_vec()
}

/// JavaScript's and TypeScript's definitions: functions, classes, interfaces, types, enums, namespaces, functions
/// given to a name (`const f = (x) => …`), constants in capitals, and the names a module exports or declares; in a
/// class, its methods.
fn js_item(lines: &[Line], n: usize, holds: Holds, ts: bool) -> Option<Item> {
    if holds == Holds::Members {
        return js_member(lines, n);
    }
    let code = &lines[n].code;
    let mut w = Words::new(code);
    let exported = w.word(b"export");
    let default = w.word(b"default");
    let declared = w.word(b"declare");
    while w.word(b"async") || w.word(b"abstract") {}
    // `export default function () {}` and `export default class {}` have no name of their own.
    let anonymous = default.then_some(b"default".as_slice());
    match w.ident()? {
        b"function" => {
            w.byte(b'*');
            let name = w.ident().or(anonymous)?;
            Some(item("fn", name, Body::Braces, None))
        }
        b"class" => {
            let name = match w.ident() {
                Some(b"extends" | b"implements") | None => anonymous?,
                Some(name) => name,
            };
            Some(item("class", name, Body::Braces, Some(Holds::Members)))
        }
        b"interface" if ts => Some(item("interface", w.ident()?, Body::Braces, None)),
        b"enum" => Some(item("enum", w.ident()?, Body::Braces, None)),
        b"namespace" | b"module" if ts => {
            // `namespace A.B {`, `declare module "x" {`: a quoted name is blanked out in the code, so it comes from
            // the line itself.
            let end = code[w.i..]
                .iter()
                .position(|b| *b == b'{')
                .map_or(code.len(), |p| w.i + p);
            let name = lines[n].raw[w.i..end].trim_ascii();
            (!name.is_empty()).then(|| item("namespace", name, Body::Braces, Some(Holds::Items)))
        }
        b"type" if ts => {
            let name = w.ident()?;
            matches!(w.peek(), Some(b'=' | b'<'))
                .then(|| item("type", name, Body::Statement { function: false }, None))
        }
        kw @ (b"const" | b"let" | b"var") => {
            if kw == b"const" && w.word(b"enum") {
                return Some(item("enum", w.ident()?, Body::Braces, None));
            }
            let name = w.ident()?;
            let value = value_at(lines, n, w.i);
            if value.is_some_and(|(m, i)| function_after(lines, m, i + 1)) {
                Some(item("fn", name, Body::Statement { function: true }, None))
            } else if exported || declared || capitals(name) {
                let kind = if kw == b"const" { "const" } else { "var" };
                Some(item(kind, name, Body::Statement { function: false }, None))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// A class's methods, and its fields given a function.
fn js_member(lines: &[Line], n: usize) -> Option<Item> {
    let code = &lines[n].code;
    let mut w = Words::new(code);
    loop {
        let at = w.i;
        let modifier = matches!(
            w.ident(),
            Some(
                b"public"
                    | b"private"
                    | b"protected"
                    | b"static"
                    | b"readonly"
                    | b"abstract"
                    | b"override"
                    | b"async"
                    | b"declare"
                    | b"accessor"
                    | b"get"
                    | b"set"
            )
        );
        // A modifier comes before a name: `get(` and `static {` are not modifiers.
        if !(modifier
            && w.peek()
                .is_some_and(|b| word_byte(b) || b == b'#' || b == b'*'))
        {
            w.i = at;
            break;
        }
    }
    w.byte(b'*');
    let private = w.byte(b'#');
    let name = w.ident()?;
    let name = if private {
        [b"#", name].concat()
    } else {
        name.to_vec()
    };
    w.byte(b'?');
    w.byte(b'!');
    match w.peek()? {
        b'(' | b'<' => Some(Item {
            kind: "fn",
            name,
            body: Body::Braces,
            holds: None,
        }),
        b'=' | b':' => {
            let (m, i) = value_at(lines, n, w.i)?;
            function_after(lines, m, i + 1).then_some(Item {
                kind: "fn",
                name,
                body: Body::Statement { function: true },
                holds: None,
            })
        }
        _ => None,
    }
}

/// Whether the value from `col` on line `n` is a function: `function`, `async (x) =>`, `x =>`, `<T>(x: T): T =>`.
fn function_after(lines: &[Line], n: usize, col: usize) -> bool {
    // The code, with each byte of a string or a regular expression as `"`: a value that is one is not a function.
    let value = |l: &Line, from: usize| -> Vec<u8> {
        l.code[from..]
            .iter()
            .zip(&l.head[from..])
            .map(|(&c, &h)| {
                if c == b' ' && !h.is_ascii_whitespace() {
                    b'"'
                } else {
                    c
                }
            })
            .collect()
    };
    let mut text = value(&lines[n], col);
    let mut next = n + 1;
    // More lines only while it cannot tell, for long parameters and a long return type.
    for window in [4, 32, 256] {
        while next < lines.len() && next <= n + window {
            text.push(b' ');
            text.extend_from_slice(&value(&lines[next], 0));
            next += 1;
        }
        if let Some(function) = function_value(&text) {
            return function;
        }
    }
    false
}

/// Whether a value is a function, or `None` when its text ends before that shows.
fn function_value(text: &[u8]) -> Option<bool> {
    let mut w = Words::new(text);
    w.word(b"async");
    if w.word(b"function") {
        return Some(true);
    }
    if w.peek() == Some(b'<') && !w.group() {
        return None;
    }
    if w.peek() == Some(b'(') {
        if !w.group() {
            return None;
        }
    } else if w.ident().is_none() {
        return Some(false);
    }
    match w.peek()? {
        b'=' => Some(w.rest().starts_with(b"=>")),
        // A return type, then the arrow: `: Promise<{ a: A }> =>`.
        b':' => {
            let rest = w.rest();
            let mut depth = 0u32;
            for (i, &b) in rest.iter().enumerate() {
                match b {
                    b'=' if depth == 0 && rest.get(i + 1) == Some(&b'>') => return Some(true),
                    b'(' | b'[' | b'{' | b'<' => depth += 1,
                    b'>' if i > 0 && matches!(rest[i - 1], b'=' | b'-') => {}
                    b')' | b']' | b'}' | b'>' if depth > 0 => depth -= 1,
                    b')' | b']' | b'}' | b';' | b'=' if depth == 0 => return Some(false),
                    _ => {}
                }
            }
            None
        }
        _ => Some(false),
    }
}

/// Shell functions: `name() {`, `name ()`, `function name {` and `function name() {`.
fn shell_item(code: &[u8]) -> Option<Item> {
    let t = code.trim_ascii_start();
    let name_of = |s: &[u8]| -> Option<(usize, usize)> {
        let start = s.len() - s.trim_ascii_start().len();
        let end = s[start..]
            .iter()
            .position(|b| b.is_ascii_whitespace() || b"(){}<>|&;$'\"=`".contains(b))
            .map_or(s.len(), |p| start + p);
        (end > start).then_some((start, end))
    };
    let keyword = t.starts_with(b"function") && t.get(8).is_some_and(u8::is_ascii_whitespace);
    let s = if keyword { &t[8..] } else { t };
    let (start, end) = name_of(s)?;
    let rest = s[end..].trim_ascii_start();
    let parens = rest.starts_with(b"(") && rest[1..].trim_ascii_start().starts_with(b")");
    (keyword || parens).then(|| item("fn", &s[start..end], Body::Braces, None))
}

/// The first place from `from` on, up to line `last`, where `stop` holds of the code, outside brackets.
fn find(
    lines: &[Line],
    from: (usize, usize),
    last: usize,
    stop: impl Fn(&[u8], usize) -> bool,
) -> Option<(usize, usize)> {
    let mut depth = 0i32;
    for (n, line) in lines.iter().enumerate().take(last + 1).skip(from.0) {
        let start = if n == from.0 { from.1 } else { 0 };
        for i in start..line.code.len() {
            match line.code[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ if depth == 0 && stop(&line.code, i) => return Some((n, i)),
                _ => {}
            }
        }
    }
    None
}

/// A definition's head on one line: its text from `from` to `stop`, without comments, each run of spaces and line
/// ends as one space.
fn head(lines: &[Line], from: (usize, usize), stop: (usize, usize)) -> Vec<u8> {
    let mut joined = Vec::new();
    let last = stop.0.min(from.0 + 30);
    for (n, line) in lines.iter().enumerate().take(last + 1).skip(from.0) {
        let h = &line.head;
        let a = if n == from.0 { from.1.min(h.len()) } else { 0 };
        let b = if n == stop.0 {
            stop.1.min(h.len())
        } else {
            h.len()
        };
        if a < b {
            joined.extend_from_slice(&h[a..b]);
        }
        joined.push(b' ');
    }
    let mut out: Vec<u8> = Vec::with_capacity(joined.len());
    for &b in joined.trim_ascii() {
        if !b.is_ascii_whitespace() {
            out.push(b);
        } else if out.last() != Some(&b' ') {
            out.push(b' ');
        }
    }
    out
}

/// Where the code of a line starts.
fn indent(code: &[u8]) -> usize {
    code.len() - code.trim_ascii_start().len()
}

/// Rust, JavaScript and TypeScript, and shell, where braces hold a definition's body.
fn braces(lines: &[Line], lang: Lang) -> Vec<Def> {
    /// A bracket not closed yet, with the definition whose body it holds, and the definitions its scope holds.
    struct Open {
        b: u8,
        def: Option<usize>,
        holds: Option<Holds>,
    }
    /// A definition whose body has not started, or whose value has not ended.
    struct Pending {
        def: usize,
        base: usize,
        body: Body,
        holds: Option<Holds>,
        /// The angle brackets open in its head at its depth, which hold types there, not comparisons, and where the
        /// last one closed.
        angle: u32,
        closed: Option<(usize, usize)>,
        /// Its value has started: what follows its `=` is not a type, until an `as` or a `satisfies`.
        valued: bool,
        cast: bool,
    }
    let js = matches!(lang, Lang::JavaScript | Lang::TypeScript);
    let mut defs: Vec<Def> = Vec::new();
    // Where each definition's head starts on its line, and where it stops, once found.
    let mut starts: Vec<(usize, usize)> = Vec::new();
    let mut stops: Vec<Option<(usize, usize)>> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut pending: Option<Pending> = None;
    // The first of the comment, attribute and decorator lines right above this one.
    let mut lead: Option<usize> = None;
    // The depth at which an attribute that spans lines opened.
    let mut attr: Option<usize> = None;
    // The last two bytes of code that are not spaces.
    let mut prev = [b' '; 2];
    let mut last = 0;
    for (n, line) in lines.iter().enumerate() {
        if line.head.trim_ascii().is_empty() {
            lead = line.comment(n).then(|| lead.unwrap_or(n));
            continue;
        }
        let in_attr = attr.is_some_and(|d| stack.len() > d);
        let holds = stack.last().map_or(Some(Holds::Items), |o| o.holds);
        // Not inside a head's angle brackets, as in `<\n  const P extends A,\n>`, nor in a value or a statement that
        // goes on, as in `type T = Map<\n  impl Fn(),\n>;`.
        let free = pending
            .as_ref()
            .is_none_or(|p| stack.len() == p.base && p.angle == 0 && p.body == Body::Braces);
        let found = match holds.filter(|_| free && !in_attr) {
            Some(holds) => match lang {
                Lang::Rust => rust_item(&line.code),
                Lang::Shell => shell_item(&line.code),
                _ => js_item(lines, n, holds, lang == Lang::TypeScript),
            },
            None => None,
        };
        if let Some(it) = found {
            // A head that never found its body ends before the next definition.
            if let Some(p) = pending.take() {
                defs[p.def].end = last + 1;
            }
            let parent = stack.iter().rev().find_map(|o| o.def);
            pending = Some(Pending {
                def: defs.len(),
                base: stack.len(),
                body: it.body,
                holds: it.holds,
                angle: 0,
                closed: None,
                valued: false,
                cast: false,
            });
            defs.push(Def {
                line: lead.unwrap_or(n) + 1,
                end: n + 1,
                kind: it.kind,
                name: it.name,
                parent,
                head: Vec::new(),
            });
            starts.push((n, indent(&line.code)));
            stops.push(None);
        }
        let depth = stack.len();
        for (i, &b) in line.code.iter().enumerate() {
            let at_base = pending.as_mut().filter(|p| stack.len() == p.base);
            // In a head's type, as `Vec<u8>` and `Service<S, { a: A }>`.
            let typed = lang != Lang::Shell
                && at_base.as_ref().is_some_and(|p| match p.body {
                    Body::Braces => true,
                    Body::Statement { .. } => defs[p.def].kind == "type" || !p.valued || p.cast,
                    Body::Value => false,
                });
            match b {
                b'<' if typed => at_base.unwrap().angle += 1,
                // Not the `>` of `=>` or `->`.
                b'>' if typed && !(i > 0 && matches!(line.code[i - 1], b'=' | b'-')) => {
                    let p = at_base.unwrap();
                    if p.angle > 0 {
                        p.angle -= 1;
                        p.closed = Some((n, i));
                    }
                }
                b'{' => {
                    // A brace in a type, as in `(): { a: A } {` and `x is { a: A }`, does not start the body.
                    let typing = js
                        && (matches!(prev[1], b':' | b'|' | b'&')
                            || prev == *b"=>"
                            || keyword_before(&line.code, i));
                    let body = at_base.is_some_and(|p| match p.body {
                        Body::Braces => p.angle == 0 && !typing,
                        Body::Statement { function } => {
                            function && p.valued && (prev == *b"=>" || prev[1] == b')')
                        }
                        Body::Value => false,
                    });
                    if body {
                        let p = pending.take().unwrap();
                        stops[p.def].get_or_insert((n, i));
                        stack.push(Open {
                            b,
                            def: Some(p.def),
                            holds: p.holds,
                        });
                    } else {
                        stack.push(Open {
                            b,
                            def: None,
                            holds: None,
                        });
                    }
                }
                b'(' | b'[' => stack.push(Open {
                    b,
                    def: None,
                    holds: None,
                }),
                b')' | b']' => {
                    let opener = if b == b')' { b'(' } else { b'[' };
                    if stack.last().is_some_and(|o| o.b == opener) {
                        stack.pop();
                    }
                }
                b'}' => {
                    while let Some(o) = stack.pop() {
                        if o.b == b'{' {
                            if let Some(d) = o.def {
                                defs[d].end = n + 1;
                            }
                            break;
                        }
                    }
                    if pending.as_ref().is_some_and(|p| stack.len() < p.base) {
                        let p = pending.take().unwrap();
                        defs[p.def].end = n + 1;
                    }
                }
                b';' => {
                    if at_base.is_some() {
                        let p = pending.take().unwrap();
                        stops[p.def].get_or_insert((n, i));
                        defs[p.def].end = n + 1;
                    }
                }
                b'=' => {
                    if let Some(p) = at_base.filter(|p| p.angle == 0 && assigns(&line.code, i)) {
                        if matches!(p.body, Body::Statement { .. }) {
                            p.valued = true;
                        }
                        let value =
                            matches!(p.body, Body::Value | Body::Statement { function: false });
                        if value && stops[p.def].is_none() {
                            stops[p.def] = Some((n, i));
                        }
                    }
                }
                // What follows `as` or `satisfies` is a type, as in `x satisfies Codec<\n  A\n>`.
                b's' if js && line.code.get(i + 1).is_none_or(|&b| !word_byte(b)) => {
                    let before = &line.code[..=i];
                    let cast = before.ends_with(b"as") || before.ends_with(b"satisfies");
                    if let Some(p) = at_base.filter(|p| p.valued)
                        && cast
                        && keyword_before(before, i + 1)
                    {
                        p.cast = true;
                    }
                }
                _ => {}
            }
            if !b.is_ascii_whitespace() {
                prev = [prev[1], b];
            }
        }
        // A JavaScript statement may end with its line, and so may a function's head with no body, as an overload
        // or an abstract method.
        let ends = pending.as_ref().is_some_and(|p| {
            let bracket = p.closed.filter(|c| c.0 == n).map(|c| c.1);
            let signature = js && p.body == Body::Braces && defs[p.def].kind == "fn";
            stack.len() == p.base
                && p.angle == 0
                && (matches!(p.body, Body::Statement { .. }) || signature)
                && statement_ends(lines, n, bracket)
                && !(signature && next_code(lines, n) == Some(b'{'))
        });
        if ends {
            let p = pending.take().unwrap();
            defs[p.def].end = n + 1;
        }
        let t = line.code.trim_ascii_start();
        let leads = in_attr
            || match lang {
                Lang::Rust => t.starts_with(b"#["),
                Lang::JavaScript | Lang::TypeScript => t.starts_with(b"@"),
                _ => false,
            };
        if leads && !in_attr && stack.len() > depth {
            attr = Some(depth);
        }
        if attr.is_some_and(|d| stack.len() <= d) {
            attr = None;
        }
        lead = leads.then(|| lead.unwrap_or(n));
        last = n;
    }
    if let Some(p) = pending {
        defs[p.def].end = last + 1;
    }
    for o in stack {
        if let Some(d) = o.def {
            defs[d].end = last + 1;
        }
    }
    for (d, def) in defs.iter_mut().enumerate() {
        let from = starts[d];
        let stop = stops[d].unwrap_or((from.0, lines[from.0].raw.len()));
        def.head = head(lines, from, stop);
        if def.kind == "impl" {
            def.name = impl_name(&def.head);
        }
        def.end = def.end.max(from.0 + 1);
    }
    defs
}

/// Words that a line cannot end a statement with, and that a line which goes on with the one before may start with.
const ENDS_NOT: [&[u8]; 13] = [
    b"extends",
    b"keyof",
    b"typeof",
    b"infer",
    b"instanceof",
    b"in",
    b"as",
    b"is",
    b"satisfies",
    b"new",
    b"await",
    b"throw",
    b"delete",
];
const GOES_ON: [&[u8]; 4] = [b"as", b"satisfies", b"extends", b"instanceof"];

/// Whether the code before `i` ends in a keyword that something must follow, as `extends` and `is`, and not in a
/// property named as one, as `self.await`.
fn keyword_before(code: &[u8], i: usize) -> bool {
    let end = code[..i]
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |p| p + 1);
    let start = code[..end]
        .iter()
        .rposition(|&b| !word_byte(b))
        .map_or(0, |p| p + 1);
    start < end && ENDS_NOT.contains(&&code[start..end]) && !code[..start].ends_with(b".")
}

/// Whether a line's last `>` closes an angle bracket opened on it: as many open before it as close.
fn closes_angle(code: &[u8]) -> bool {
    let mut open = 0i32;
    for (i, &b) in code.iter().enumerate() {
        let before = i.checked_sub(1).map(|p| code[p]);
        match b {
            b'<' if code.get(i + 1) != Some(&b'=') => open += 1,
            b'>' if !matches!(before, Some(b'=' | b'-')) => open -= 1,
            _ => {}
        }
    }
    open >= 0
}

/// Where the first or the last byte of a line is, outside comments.
fn edge(l: &Line, last: bool) -> Option<usize> {
    let mut at = l
        .head
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.is_ascii_whitespace())
        .map(|(i, _)| i);
    if last { at.next_back() } else { at.next() }
}

/// The first byte of code after line `n`, outside comments: a space for a string.
fn next_code(lines: &[Line], n: usize) -> Option<u8> {
    lines[n + 1..]
        .iter()
        .find_map(|l| edge(l, false).map(|i| l.code[i]))
}

/// Whether a JavaScript statement ends with line `n`: it ends in `;`, or neither it nor the next line shows that the
/// next one goes on with it. A string or a regular expression at either end is not an operator, nor is a `>` that
/// closes a type's angle bracket, at `bracket`; and a line that ends inside a string goes on.
fn statement_ends(lines: &[Line], n: usize, bracket: Option<usize>) -> bool {
    if lines.get(n + 1).is_some_and(|l| l.open) {
        return false;
    }
    let code = &lines[n].code;
    if let Some(i) = edge(&lines[n], true) {
        match code[i] {
            b';' => return true,
            b'>' if code[..i].ends_with(b"=") => return false,
            // A type's `>`, as in `x as Map<K, V>`.
            b'>' if bracket == Some(i) || closes_angle(&code[..=i]) => {}
            // `i++` ends it, as TypeScript's `x!` does.
            b'+' | b'-' if code[..i].ends_with(&[code[i]]) => {}
            b if b"=,([{+-*/%&|?:.<>^".contains(&b) => return false,
            b if word_byte(b) && keyword_before(code, i + 1) => return false,
            _ => {}
        }
    }
    let Some((l, i)) = lines[n + 1..]
        .iter()
        .find_map(|l| Some((l, edge(l, false)?)))
    else {
        return true;
    };
    let word = &l.code[i..l.code[i..]
        .iter()
        .position(|&b| !word_byte(b))
        .map_or(l.code.len(), |p| i + p)];
    !(b".?:+-*/%&|=,)]>".contains(&l.code[i]) || GOES_ON.contains(&word))
}

/// Where the `=` is that gives a declaration its value, from `col` on line `n` on: past its type, which may span lines
/// and hold brackets, and before its statement ends.
fn value_at(lines: &[Line], n: usize, col: usize) -> Option<(usize, usize)> {
    let mut depth = 0u32;
    for (m, line) in lines.iter().enumerate().skip(n) {
        let code = &line.code;
        let mut closed = None;
        for i in if m == n { col } else { 0 }..code.len() {
            match code[i] {
                b'(' | b'[' | b'{' | b'<' => depth += 1,
                b'>' if i > 0 && matches!(code[i - 1], b'=' | b'-') => {}
                b')' | b']' | b'}' | b'>' if depth > 0 => {
                    depth -= 1;
                    closed = Some(i);
                }
                b';' if depth == 0 => return None,
                _ if depth == 0 && assigns(code, i) => return Some((m, i)),
                _ => {}
            }
        }
        if depth == 0 && !line.head.trim_ascii().is_empty() && statement_ends(lines, m, closed) {
            return None;
        }
    }
    None
}

/// The width of a line's indentation, with tabs to the next multiple of 8, as Python counts it.
fn indentation(raw: &[u8]) -> usize {
    let mut width = 0;
    for &b in raw {
        match b {
            b' ' => width += 1,
            b'\t' => width = (width / 8 + 1) * 8,
            _ => break,
        }
    }
    width
}

/// Python's `def`, `async def` and `class`, and at the top of a module, `type` aliases and constants in capitals.
fn python_item(t: &[u8], top: bool) -> Option<(&'static str, &[u8])> {
    let mut w = Words::new(t);
    w.word(b"async");
    if w.word(b"def") {
        return Some(("fn", w.ident()?));
    }
    if w.word(b"class") {
        return Some(("class", w.ident()?));
    }
    if !top {
        return None;
    }
    let mut w = Words::new(t);
    if w.word(b"type")
        && let Some(name) = w.ident().filter(|_| matches!(w.peek(), Some(b'=' | b'[')))
    {
        return Some(("type", name));
    }
    let mut w = Words::new(t);
    let name = w.ident()?;
    let given = match w.peek() {
        Some(b':') => true,
        Some(b'=') => w.rest().get(1) != Some(&b'='),
        _ => false,
    };
    (given && capitals(name)).then_some(("const", name))
}

/// Python, where indentation holds a definition's body. A logical line goes on while brackets are open, after a
/// backslash, and through a string that spans lines.
fn python(lines: &[Line]) -> Vec<Def> {
    let mut defs: Vec<Def> = Vec::new();
    // Classes and functions not ended yet: the indentation of their first line, and the definition.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut lead: Option<usize> = None;
    // The last line of the last logical line of code.
    let mut last = 0;
    let mut n = 0;
    while n < lines.len() {
        let start = n;
        let mut depth = 0i32;
        loop {
            let code = &lines[n].code;
            for &b in code {
                match b {
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => depth = (depth - 1).max(0),
                    _ => {}
                }
            }
            n += 1;
            let backslash = code.trim_ascii_end().ends_with(b"\\");
            if n >= lines.len() || !(depth > 0 || backslash || lines[n].open) {
                break;
            }
        }
        let end = n - 1;
        let first = &lines[start];
        let t = first.code.trim_ascii();
        // Blank, or comments only. A line that is only a string, such as a docstring, is code.
        if first.head.trim_ascii().is_empty() {
            lead = first.comment(start).then(|| lead.unwrap_or(start));
            continue;
        }
        let width = indentation(first.raw);
        while let Some(&(w, d)) = open.last() {
            if width > w {
                break;
            }
            defs[d].end = last + 1;
            open.pop();
        }
        let in_function = open.last().is_some_and(|&(_, d)| defs[d].kind == "fn");
        if !in_function && t.starts_with(b"@") {
            lead.get_or_insert(start);
            last = end;
            continue;
        }
        if let Some((kind, name)) = python_item(t, open.is_empty()).filter(|_| !in_function) {
            let at = indent(&first.code);
            let body = kind == "fn" || kind == "class";
            let stop = if body {
                find(lines, (start, at), end, |c, i| c[i] == b':')
            } else {
                find(lines, (start, at), end, assigns)
            };
            let stop = stop.unwrap_or((end, lines[end].raw.len()));
            defs.push(Def {
                line: lead.unwrap_or(start) + 1,
                end: end + 1,
                kind,
                name: name.to_vec(),
                parent: open.last().map(|&(_, d)| d),
                head: head(lines, (start, at), stop),
            });
            if body {
                open.push((width, defs.len() - 1));
            }
        }
        lead = None;
        last = end;
    }
    for (_, d) in open {
        defs[d].end = last + 1;
    }
    defs
}

/// Markdown's headings: `#` to `######`, and text underlined with `=` or `-`, outside fenced code and front matter.
/// A heading's section ends at the next heading of its level or above.
fn markdown(raws: &[&[u8]]) -> Vec<Def> {
    const LEVELS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];
    let blank = |l: &[u8]| l.trim_ascii().is_empty();
    // The last line that is not blank before line `n`, counted from 1.
    let before = |n: usize| (0..n).rev().find(|&i| !blank(raws[i])).map_or(n, |i| i + 1);
    /// A line without the up to 3 spaces a heading or a fence may start with; None for an indented line, code.
    fn unindent(l: &[u8]) -> Option<&[u8]> {
        let spaces = l.iter().take_while(|b| **b == b' ').count();
        (spaces < 4 && l.get(spaces) != Some(&b'\t')).then(|| &l[spaces..])
    }
    let mut defs: Vec<Def> = Vec::new();
    let mut open: Vec<(usize, usize)> = Vec::new();
    // In fenced code: the fence's character and length.
    let mut fence: Option<(u8, usize)> = None;
    let mut n = 0;
    if raws.first().is_some_and(|l| l.trim_ascii_end() == b"---") {
        n = 1;
        while n < raws.len() {
            let t = raws[n].trim_ascii_end();
            n += 1;
            if t == b"---" || t == b"..." {
                break;
            }
        }
    }
    while n < raws.len() {
        let raw = raws[n];
        let t = unindent(raw);
        let run = |t: &[u8], c: u8| t.iter().take_while(|b| **b == c).count();
        if let Some((c, len)) = fence {
            if t.is_some_and(|t| run(t, c) >= len && t[run(t, c)..].trim_ascii().is_empty()) {
                fence = None;
            }
            n += 1;
            continue;
        }
        if let Some(t) = t {
            let c = t.first().copied().filter(|c| *c == b'`' || *c == b'~');
            if let Some(c) =
                c.filter(|c| run(t, *c) >= 3 && !(*c == b'`' && t[run(t, *c)..].contains(&b'`')))
            {
                fence = Some((c, run(t, c)));
                n += 1;
                continue;
            }
        }
        let atx = t.and_then(|t| {
            let level = run(t, b'#');
            let rest = &t[level..];
            ((1..=6).contains(&level) && (rest.is_empty() || rest[0] == b' ' || rest[0] == b'\t'))
                .then(|| {
                    let text = rest.trim_ascii();
                    // A closing run of #, after a space.
                    let close =
                        text.len() - run(&text.iter().rev().copied().collect::<Vec<_>>(), b'#');
                    let text = if close == 0 || text[close - 1] == b' ' {
                        text[..close].trim_ascii()
                    } else {
                        text
                    };
                    (n, level, text)
                })
        });
        let setext = || {
            let t = t?.trim_ascii_end();
            let c = *t.first().filter(|c| **c == b'=' || **c == b'-')?;
            let paragraph = n > 0 && {
                let p = unindent(raws[n - 1]).unwrap_or(b"").trim_ascii();
                !p.is_empty()
                    && !b"#>-*+|<`~".contains(&p[0])
                    && !(p[0].is_ascii_digit()
                        && p.iter()
                            .find(|b| !b.is_ascii_digit())
                            .is_some_and(|b| *b == b'.' || *b == b')'))
            };
            (paragraph && run(t, c) == t.len() && (c == b'=' || t.len() >= 2)).then(|| {
                (
                    n - 1,
                    if c == b'=' { 1 } else { 2 },
                    raws[n - 1].trim_ascii(),
                )
            })
        };
        if let Some((start, level, text)) = atx.or_else(setext) {
            while let Some(&(l, d)) = open.last() {
                if l < level {
                    break;
                }
                defs[d].end = before(start).max(defs[d].line);
                open.pop();
            }
            defs.push(Def {
                line: start + 1,
                end: n + 1,
                kind: LEVELS[level - 1],
                name: text.to_vec(),
                parent: open.last().map(|&(_, d)| d),
                head: raws[start].trim_ascii().to_vec(),
            });
            open.push((level, defs.len() - 1));
        }
        n += 1;
    }
    let end = before(raws.len());
    for (_, d) in open {
        defs[d].end = end.max(defs[d].end);
    }
    defs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An outline as text, one definition a line: `line-end kind name < parent: head`.
    fn shown(lang: Lang, text: &str) -> Vec<String> {
        let defs = read(lang, text.as_bytes());
        defs.iter()
            .map(|d| {
                let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
                let parent = d
                    .parent
                    .map_or(String::new(), |p| format!(" < {}", s(&defs[p].name)));
                format!(
                    "{}-{} {} {}{}: {}",
                    d.line,
                    d.end,
                    d.kind,
                    s(&d.name),
                    parent,
                    s(&d.head)
                )
            })
            .collect()
    }

    #[test]
    fn tells_a_language_by_extension_or_first_line() {
        let lang = |path: &str, text: &str| language(Path::new(path), text.as_bytes());
        assert_eq!(lang("src/main.rs", ""), Some(Lang::Rust));
        assert_eq!(lang("a/b.PY", ""), Some(Lang::Python));
        assert_eq!(lang("app.tsx", ""), Some(Lang::TypeScript));
        assert_eq!(lang("x.mjs", ""), Some(Lang::JavaScript));
        assert_eq!(lang("README.md", ""), Some(Lang::Markdown));
        assert_eq!(lang("run", "#!/usr/bin/env bash\necho"), Some(Lang::Shell));
        assert_eq!(
            lang("tool", "#!/usr/bin/python3.14 -I\n"),
            Some(Lang::Python)
        );
        assert_eq!(
            lang("tool", "#!/usr/bin/env -S node --no-warnings\n"),
            Some(Lang::JavaScript)
        );
        assert_eq!(lang("main.c", "int main;"), None);
        assert_eq!(lang("notes", "plain words"), None);
        assert_eq!(lang("tool", "#!/usr/bin/perl\n"), None);
    }

    #[test]
    fn reads_rust_items_their_spans_and_heads() {
        let text = r##"//! A module.

use std::fmt;

/// A thing, with a } in its docs.
#[derive(Debug)]
pub struct Thing {
    name: String, // and { in a comment
}

pub(crate) const BRACES: &str = "{ not a body }";
static mut COUNT: [u8; 2] = [0; 2];

impl fmt::Display for Thing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = '{';
        let s = r#"a "}" here"#;
        write!(f, "{}{}{}", self.name, c, s)
    }
}

pub async fn parse<'a>(
    words: &'a [String], // the words
) -> Option<&'a str>
where
    'a: 'a,
{
    fn inner() {}
    /* fn fake() { /* nested */ */
    words.first().map(|w| w.as_str())
}

pub trait Shape {
    fn area(&self) -> f64;
    fn name(&self) -> &str {
        "shape"
    }
}

impl<T: Copy> From<T> for Thing where T: fmt::Debug {
    fn from(_: T) -> Self { todo!() }
}

type Pair = (u8, u8);
struct Unit;

macro_rules! twice {
    ($e:expr) => {
        $e;
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(
        unix,
        ignore
    )]
    fn works() {}
}
"##;
        assert_eq!(
            shown(Lang::Rust, text),
            [
                "5-9 struct Thing: pub struct Thing",
                "11-11 const BRACES: pub(crate) const BRACES: &str",
                "12-12 static COUNT: static mut COUNT: [u8; 2]",
                "14-20 impl fmt::Display for Thing: impl fmt::Display for Thing",
                "15-19 fn fmt < fmt::Display for Thing: fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result",
                "22-31 fn parse: pub async fn parse<'a>( words: &'a [String], ) -> Option<&'a str> where 'a: 'a,",
                "33-38 trait Shape: pub trait Shape",
                "34-34 fn area < Shape: fn area(&self) -> f64",
                "35-37 fn name < Shape: fn name(&self) -> &str",
                "40-42 impl From<T> for Thing: impl<T: Copy> From<T> for Thing where T: fmt::Debug",
                "41-41 fn from < From<T> for Thing: fn from(_: T) -> Self",
                "44-44 type Pair: type Pair",
                "45-45 struct Unit: struct Unit",
                "47-51 macro twice: macro_rules! twice",
                "53-63 mod tests: mod tests",
                "57-62 fn works < tests: fn works()",
            ]
        );
    }

    #[test]
    fn reads_rust_generics_that_hold_braces_and_raw_names() {
        let text = r#"macro_rules! r#try {
    ($e:expr) => {};
}
pub type SplitPaths<'a> = Map<
    Split<'a, u8>,
    impl FnMut(&u8) -> bool,
>;
impl<T: Copy> const Default for Cell<T> {
    fn default() -> Self {
        todo!()
    }
}
impl<const N: usize> Trait for Arr<{ N }> {
    fn f() {}
}
fn bound<T>(t: T)
where
    T: Copy,
{
}
"#;
        assert_eq!(
            shown(Lang::Rust, text),
            [
                "1-3 macro r#try: macro_rules! r#try",
                "4-7 type SplitPaths: pub type SplitPaths<'a>",
                "8-12 impl Default for Cell<T>: impl<T: Copy> const Default for Cell<T>",
                "9-11 fn default < Default for Cell<T>: fn default() -> Self",
                "13-15 impl Trait for Arr<{ N }>: impl<const N: usize> Trait for Arr<{ N }>",
                "14-14 fn f < Trait for Arr<{ N }>: fn f()",
                "16-20 fn bound: fn bound<T>(t: T) where T: Copy,",
            ]
        );
    }

    #[test]
    fn reads_python_by_indentation() {
        let text = r##"#!/usr/bin/env python3
"""A module that names def fake(): in its docstring."""
import os

MAX_ROWS = 100
LIMITS: dict[str, int] = {
    'a': 1,
}
lower = 2


# Finds things.
@cache
@other(
    arg=1,
)
def find(path: str = "#",
         *, deep: bool = False) -> list[str]:
    """Docs, with
def not_this():
at the left."""
    def helper():
        return 1
    return [path]

class Shelf(Base, metaclass=Meta):
    class Inner:
        pass

    async def fetch(self, key):  # a comment
        x = {
'a': 1}
        return x

    # the end of Shelf

type Pair = tuple[int, int]
def one_line(): return 1
"##;
        assert_eq!(
            shown(Lang::Python, text),
            [
                "5-5 const MAX_ROWS: MAX_ROWS",
                "6-8 const LIMITS: LIMITS: dict[str, int]",
                "12-24 fn find: def find(path: str = \"#\", *, deep: bool = False) -> list[str]",
                "26-33 class Shelf: class Shelf(Base, metaclass=Meta)",
                "27-28 class Inner < Shelf: class Inner",
                "30-33 fn fetch < Shelf: async def fetch(self, key)",
                "37-37 type Pair: type Pair",
                "38-38 fn one_line: def one_line()",
            ]
        );
    }

    #[test]
    fn reads_javascript_and_typescript() {
        let text = r#"import fs from 'fs';

/** Counts {braces} in a doc comment. */
export async function count(text, re = /[{]/g) {
  const inner = () => 1;
  return `${text.length} {${ {a: 1}.a }}`.length;
}

export const MAX_ROWS = 100;
const config = { a: 1 };
const double = (x) => x * 2;
export const load = async (path: string): Promise<string> => {
  return fs.readFileSync(path, 'utf8');
};
let quote = "it's";

export default class Store extends Base {
  #items = [];
  static count = 0;
  constructor(items) {
    super();
  }
  get size() { return this.#items.length; }
  async *each() {}
  handle = (e) => {
    return e;
  };
  #hidden(x: number): void {}
}

interface Shape {
  area(): number;
}
type Pair<T> =
  | [T, T]
  | null;
enum Color { Red, Green }
declare module "pkg" {
  export function f(): void;
}
describe('x', () => {
  function notListed() {}
});
"#;
        assert_eq!(
            shown(Lang::TypeScript, text),
            [
                "3-7 fn count: export async function count(text, re = /[{]/g)",
                "9-9 const MAX_ROWS: export const MAX_ROWS",
                "11-11 fn double: const double = (x) => x * 2",
                "12-14 fn load: export const load = async (path: string): Promise<string> =>",
                "17-29 class Store: export default class Store extends Base",
                "20-22 fn constructor < Store: constructor(items)",
                "23-23 fn size < Store: get size()",
                "24-24 fn each < Store: async *each()",
                "25-27 fn handle < Store: handle = (e) =>",
                "28-28 fn #hidden < Store: #hidden(x: number): void",
                "31-33 interface Shape: interface Shape",
                "34-36 type Pair: type Pair<T>",
                "37-37 enum Color: enum Color",
                "38-40 namespace \"pkg\": declare module \"pkg\"",
                "39-39 fn f < \"pkg\": export function f(): void",
            ]
        );
    }

    #[test]
    fn reads_typescript_types_that_span_lines_and_hold_braces() {
        let text = r#"export const empty: <A = never>() => Array<A> = () => []
export const map: {
  <A, B>(f: (a: A) => B): (self: Array<A>) => Array<B>
} = dual(2, (self, f) => self.map(f))
export class Svc extends Service<Svc, {
  run(): void
}>()("svc") {
  abstract toJSON(): unknown
  [inspect]() {
    return 1
  }
  static create = <S>(
    s: S
  ): Obj<
    S,
    Out<S>
  > => {
    return new Obj(s)
  }
}
export function isObject(x: unknown): x is { [k: string]: unknown } {
  return true
}
export type Flatten<T> = T extends
  Array<infer A> ? A
  : never
export namespace Gen {
  export type Return<A, E = never> = Generator<A, E>
}
const identity = <A>(): S<A> => self as S<A>
export const when = <
  R,
  const P extends R,
>(p: P) => p
export const codec = make({}) satisfies Codec<
  A,
  B
>
export const el = document.getElementById("x")!
export function over(a: string): void
export function over(a: unknown) {}
"#;
        assert_eq!(
            shown(Lang::TypeScript, text),
            [
                "1-1 fn empty: export const empty: <A = never>() => Array<A> = () => []",
                "2-4 const map: export const map: { <A, B>(f: (a: A) => B): (self: Array<A>) => Array<B> }",
                "5-20 class Svc: export class Svc extends Service<Svc, { run(): void }>()(\"svc\")",
                "8-8 fn toJSON < Svc: abstract toJSON(): unknown",
                "12-19 fn create < Svc: static create = <S>( s: S ): Obj< S, Out<S> > =>",
                "21-23 fn isObject: export function isObject(x: unknown): x is { [k: string]: unknown }",
                "24-26 type Flatten: export type Flatten<T>",
                "27-29 namespace Gen: export namespace Gen",
                "28-28 type Return < Gen: export type Return<A, E = never>",
                "30-30 fn identity: const identity = <A>(): S<A> => self as S<A>",
                "31-34 fn when: export const when = <",
                "35-38 const codec: export const codec",
                "39-39 const el: export const el",
                "40-40 fn over: export function over(a: string): void",
                "41-41 fn over: export function over(a: unknown)",
            ]
        );
    }

    #[test]
    fn ends_javascript_statements_at_strings_and_templates() {
        let text = r#"const LDD_PATH = '/usr/bin/ldd'
function family() {
  return 1
}
const isWin = (o) => o.platform === 'win32'

exports.isWin = isWin
const MSG = 'a' +
  'b'
const usage = () => `
usage: ${name}
`
const RE = /\/win-(x86|x64)\//
module.exports = { usage }
"#;
        assert_eq!(
            shown(Lang::JavaScript, text),
            [
                "1-1 const LDD_PATH: const LDD_PATH",
                "2-4 fn family: function family()",
                "5-5 fn isWin: const isWin = (o) => o.platform === 'win32'",
                "8-9 const MSG: const MSG",
                "10-12 fn usage: const usage = () => `",
                "13-13 const RE: const RE",
            ]
        );
    }

    #[test]
    fn reads_shell_functions_past_heredocs_and_comments() {
        let text = r#"#!/bin/sh
set -e

# Says hello.
# Twice.
hello() {
  echo "}" '{' ${x} \{
  cat <<-EOF
	fake() {
	EOF
}

function bye {
  : # }
}
function both() { :; }
later ()
{
  cat <<'END'
}
END
}
echo "done() {"
"#;
        assert_eq!(
            shown(Lang::Shell, text),
            [
                "4-11 fn hello: hello()",
                "13-15 fn bye: function bye",
                "16-16 fn both: function both()",
                "17-22 fn later: later ()",
            ]
        );
    }

    #[test]
    fn reads_markdown_headings_outside_code() {
        let text = "---\ntitle: x\n# not a heading\n---\n\n# Title #\n\nIntro.\n\n## Part one\n\n```sh\n# a comment\n```\n\n\
                    Setext part\n-----------\n\nText.\n\n### Deep ###\n\n    # indented code\n\nTwo\n===\n\nEnd.\n";
        assert_eq!(
            shown(Lang::Markdown, text),
            [
                "6-23 h1 Title: # Title #",
                "10-14 h2 Part one < Title: ## Part one",
                "16-23 h2 Setext part < Title: Setext part",
                "21-23 h3 Deep < Setext part: ### Deep ###",
                "25-28 h1 Two: Two",
            ]
        );
    }

    #[test]
    fn splits_lines_as_a_file_has_them() {
        assert_eq!(split(b""), Vec::<&[u8]>::new());
        assert_eq!(split(b"a\r\nb"), [b"a".as_slice(), b"b"]);
        assert_eq!(split(b"a\n\n"), [b"a".as_slice(), b""]);
    }
}
