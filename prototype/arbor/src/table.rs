//! Output as short tables of only the asked fields: easy to scan, and cheap for a model to read. The report
//! measured a 100-file listing at 6,283 tokens as pretty JSON and 401 as names only (OPTIMIZATION.md §6).

use std::io::{self, Write};

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
