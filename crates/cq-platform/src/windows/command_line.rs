//! A program's command line as one string, for the API that starts it.

use std::path::Path;

/// The longest command line `CreateProcessWithTokenW` takes, terminator
/// included to be safe; it answers a longer one with "the parameter is
/// incorrect".
pub(super) const LIMIT: usize = 1024;

/// Whether `line` is too long for that call, counting the terminator among
/// its characters to be safe.
pub(super) fn too_long(line: &str) -> bool {
    line.encode_utf16().count() >= LIMIT
}

/// One argument as the C runtime parses it back: quoted when it holds a
/// space or a quote (or is empty), with the quotes and the backslashes that
/// lead up to one escaped, and any trailing backslashes doubled so they do
/// not escape the closing quote.
fn push_quoted(line: &mut String, arg: &str) {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        line.push_str(arg);
        return;
    }
    line.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                line.push('"');
                backslashes = 0;
            }
            _ => {
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push(c);
                backslashes = 0;
            }
        }
    }
    line.extend(std::iter::repeat_n('\\', backslashes * 2));
    line.push('"');
}

/// The command line a program is started with: itself, then the recorded
/// arguments after the first (which is the program).
pub(super) fn of(exe: &Path, args: &[String]) -> String {
    let mut line = String::new();
    push_quoted(&mut line, &exe.to_string_lossy());
    for arg in args.iter().skip(1) {
        line.push(' ');
        push_quoted(&mut line, arg);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quoted(arg: &str) -> String {
        let mut line = String::new();
        push_quoted(&mut line, arg);
        line
    }

    #[test]
    fn arguments_are_quoted_so_the_program_reads_back_what_was_recorded() {
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted(""), r#""""#);
        assert_eq!(quoted("two words"), r#""two words""#);
        assert_eq!(quoted(r#"say "hi""#), r#""say \"hi\"""#);
        // Backslashes only matter before a quote, and at the end.
        assert_eq!(quoted(r"C:\a\b"), r"C:\a\b");
        assert_eq!(quoted(r"C:\My Dir\"), r#""C:\My Dir\\""#);
        assert_eq!(quoted(r#"a\"b"#), r#""a\\\"b""#);
        let line = of(
            Path::new(r"C:\Program Files\App\app.exe"),
            &["app.exe".into(), "--profile".into(), "My Profile".into()],
        );
        assert_eq!(
            line,
            r#""C:\Program Files\App\app.exe" --profile "My Profile""#
        );
    }

    #[test]
    fn a_line_is_too_long_from_the_limit_on() {
        assert!(!too_long(&"x".repeat(LIMIT - 1)));
        assert!(too_long(&"x".repeat(LIMIT)));
        // Counted in UTF-16 units, as the API counts characters: two each.
        assert!(too_long(&"\u{1F600}".repeat(LIMIT / 2)));
    }
}
