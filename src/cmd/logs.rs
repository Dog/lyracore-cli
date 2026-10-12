//! Bounded log display; complete output is streamed only on explicit request.
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAX_BYTES: u64 = 64 * 1024;
const MAX_LINES: usize = 100;

pub(super) fn show(path: &Path, all: bool, out: &mut impl Write) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 {
        return writeln!(out, "(empty)");
    }
    if all {
        std::io::copy(&mut file, out)?;
        return Ok(());
    }
    let offset = len.saturating_sub(MAX_BYTES);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES).read_to_end(&mut bytes)?;
    // Do not count the terminal newline as an extra empty line.
    let end = bytes.len() - usize::from(bytes.last() == Some(&b'\n'));
    let mut start = bytes[..end]
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, byte)| **byte == b'\n')
        .nth(MAX_LINES - 1)
        .map_or(0, |(index, _)| index + 1);
    if offset > 0 && start == 0 {
        // A byte cap may split a line. Prefer complete lines; a single huge line stays bounded.
        start = bytes[..end]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(0, |i| i + 1);
    }
    if offset > 0 || start > 0 {
        writeln!(
            out,
            "(last {MAX_LINES} lines, at most 64 KiB; use --all for the full log)"
        )?;
    }
    out.write_all(&bytes[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bounds_bytes_and_lines_while_all_preserves_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gateway.log");
        let content: String = (0..2000)
            .map(|i| format!("{i}: {}\n", "x".repeat(100)))
            .collect();
        std::fs::write(&path, &content).unwrap();
        let mut out = Vec::new();
        show(&path, false, &mut out).unwrap();
        let tail = String::from_utf8(out).unwrap();
        assert!(tail.contains("1900: "));
        assert!(!tail.contains("1899: "));
        assert!(tail.ends_with(&format!("1999: {}\n", "x".repeat(100))));
        assert_eq!(tail.lines().count(), MAX_LINES + 1);
        let mut all = Vec::new();
        show(&path, true, &mut all).unwrap();
        assert_eq!(all, content.as_bytes());
    }

    #[test]
    fn huge_lines_are_bounded_and_unterminated_short_logs_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gateway.log");
        for content in ["x".repeat(2_000_000), "first\nlast".into(), "".into()] {
            std::fs::write(&path, &content).unwrap();
            let mut out = Vec::new();
            show(&path, false, &mut out).unwrap();
            assert!(out.len() <= MAX_BYTES as usize + 100);
            if content == "first\nlast" {
                assert_eq!(out, content.as_bytes());
            }
        }
    }
}
