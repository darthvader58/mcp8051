//! Shell-quoting, **for display only**.
//!
//! The `command` field of an envelope exists so a human can paste it into a
//! terminal and see the same thing happen. It is never fed back to a shell by
//! this server: children are always spawned with a direct argv, so a filename
//! containing `;` is an odd filename rather than a command injection.

/// Characters safe to leave bare in a POSIX shell word.
fn is_safe(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-')
}

/// Quote one argument the way `/bin/sh` would need it.
pub fn shell_quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(is_safe) {
        return arg.to_string();
    }
    // Single quotes protect everything except a single quote itself, which is
    // spliced as: close quote, escaped quote, reopen quote.
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for c in arg.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Render an argv as a copy-pasteable command line.
pub fn render(program: &str, args: &[String]) -> String {
    let mut out = shell_quote(program);
    for a in args {
        out.push(' ');
        out.push_str(&shell_quote(a));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_are_left_alone() {
        assert_eq!(
            render("sdcc", &["-mmcs51".into(), "a.c".into()]),
            "sdcc -mmcs51 a.c"
        );
    }

    #[test]
    fn spaces_and_quotes_are_contained() {
        assert_eq!(shell_quote("my file.c"), "'my file.c'");
        assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
        assert_eq!(shell_quote("a;rm -rf /"), "'a;rm -rf /'");
        assert_eq!(shell_quote(""), "''");
    }
}
