//! Output as short tables of only the asked fields: easy to scan, and cheap for a model to read. The report
//! measured a 100-file listing at 6,283 tokens as pretty JSON and 401 as names only (OPTIMIZATION.md §6). And text
//! that came from a command in a world, shown so that it cannot act on the person's terminal.

use std::io::{self, Write};

/// True for a character a terminal acts on or does not show: control characters (escape sequences, newlines,
/// backspace), and the invisible ones that change how the text around them looks, such as direction overrides and
/// zero-width spaces. Text from a command in a world must not move the cursor or reorder what a person reads.
pub fn unseen(c: char) -> bool {
    c.is_control()
        || matches!(
            c as u32,
            0x00AD
                | 0x061C
                | 0x180E
                | 0x200B..=0x200F
                | 0x2028..=0x202E
                | 0x2060..=0x206F
                | 0xFEFF
                | 0xFFF9..=0xFFFB
                | 0xE0000..=0xE007F
        )
}

/// How an `unseen` character is spelled out: `\n`, `\e`, `\x7f`, `\u202e`, as bash's `$'…'` quoting reads it.
fn spelled(c: char, out: &mut String) {
    match c {
        '\n' => out.push_str("\\n"),
        '\t' => out.push_str("\\t"),
        '\r' => out.push_str("\\r"),
        '\x1b' => out.push_str("\\e"),
        c if (c as u32) < 0x80 => out.push_str(&format!("\\x{:02x}", c as u32)),
        c if (c as u32) <= 0xFFFF => out.push_str(&format!("\\u{:04x}", c as u32)),
        c => out.push_str(&format!("\\U{:08x}", c as u32)),
    }
}

/// `text` with every `unseen` character spelled out, so it shows as one line that cannot change the terminal.
pub fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if unseen(c) {
            spelled(c, &mut out);
        } else {
            out.push(c);
        }
    }
    out
}

/// Bytes in bash's `$'…'` quoting: backslashes and quotes escaped, and `unseen` characters and bytes that are not
/// UTF-8 spelled out. A shell reads it back as exactly those bytes.
pub fn ansi_quoted(bytes: &[u8]) -> String {
    let mut out = String::from("$'");
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '\'' => out.push_str("\\'"),
                c if unseen(c) => spelled(c, &mut out),
                c => out.push(c),
            }
        }
        for b in chunk.invalid() {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out.push('\'');
    out
}

/// Prints rows under a header, columns separated by two spaces, the last column never padded.
pub fn print(header: &[&str], rows: &[Vec<String>]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    write_table(&mut out, header, rows)
}

pub fn write_table(out: &mut impl Write, header: &[&str], rows: &[Vec<String>]) -> io::Result<()> {
    let cols = header.len();
    let mut width: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(cols) {
            width[i] = width[i].max(cell.chars().count());
        }
    }
    let line = |out: &mut dyn Write, cells: &mut dyn Iterator<Item = &str>| -> io::Result<()> {
        let mut s = String::new();
        for (i, cell) in cells.enumerate() {
            if i + 1 < cols {
                s.push_str(cell);
                s.push_str(&" ".repeat(width[i] - cell.chars().count() + 2));
            } else {
                s.push_str(cell);
            }
        }
        writeln!(out, "{}", s.trim_end())
    };
    line(out, &mut header.iter().copied())?;
    for row in rows {
        line(out, &mut row.iter().map(String::as_str))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spells_out_what_a_terminal_would_act_on() {
        assert_eq!(escaped("plain words, café"), "plain words, café");
        assert_eq!(
            escaped("a\nb\x1b[2Jc\u{202e}d\u{200b}e\x7f\u{9b}"),
            "a\\nb\\e[2Jc\\u202ed\\u200be\\x7f\\u009b"
        );
        assert_eq!(escaped("\u{e0041}"), "\\U000e0041");
        assert_eq!(
            ansi_quoted(b"it's a\\b\n\xff\x1b"),
            "$'it\\'s a\\\\b\\n\\xff\\e'"
        );
    }

    #[test]
    fn aligns_columns() {
        let mut out = Vec::new();
        super::write_table(
            &mut out,
            &["name", "state", "base"],
            &[
                vec!["a".into(), "open".into(), "~/x".into()],
                vec!["longer".into(), "kept".into(), "~/y".into()],
            ],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "name    state  base\na       open   ~/x\nlonger  kept   ~/y\n"
        );
    }
}
