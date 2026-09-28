//! Color on/off: `--no-color` / `NO_COLOR` (<https://no-color.org>) decision
//! and ANSI stripping for console output (`--simple`, `--fix`).
//!
//! Console modes keep their inline `\x1b[..m` escapes and print through
//! [`cprintln!`], which strips them when color is off. A process-wide switch
//! (set once in `main`) avoids threading a flag through every print site.
//! The TUI takes the flag explicitly (`App::color`) instead.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};

/// Env var that disables color when set to a non-empty value
pub const NO_COLOR_ENV: &str = "NO_COLOR";

/// Console color switch; on by default (tests, library callers)
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Whether to use color: off with `--no-color`, or when `NO_COLOR` is set
/// and non-empty (an empty value does not disable color, per no-color.org).
pub fn should_color(no_color_flag: bool, no_color_env: Option<&OsStr>) -> bool {
    !no_color_flag && no_color_env.is_none_or(OsStr::is_empty)
}

/// Set the console color switch read by [`cprintln!`]
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Current console color switch
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// `s` as is with color on; with color off, without ANSI CSI sequences
/// (`ESC [` params, final byte `@`..=`~`), incl. escapes in check output.
pub fn paint(s: &str, color: bool) -> Cow<'_, str> {
    if color || !s.contains('\x1b') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' || chars.peek() != Some(&'[') {
            out.push(c);
            continue;
        }
        // Skip `[`, params, and the final byte
        chars.next();
        let _ = chars.by_ref().find(|c| ('@'..='~').contains(c));
    }
    Cow::Owned(out)
}

/// `println!` honoring the console color switch ([`enabled`])
macro_rules! cprintln {
    () => {
        println!()
    };
    ($($arg:tt)*) => {
        println!(
            "{}",
            $crate::color::paint(&format!($($arg)*), $crate::color::enabled())
        )
    };
}
pub(crate) use cprintln;

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case::default(false, None, true)]
    #[case::flag(true, None, false)]
    #[case::env_set(false, Some("1"), false)]
    #[case::env_any_value(false, Some("false"), false)]
    #[case::env_empty(false, Some(""), true)]
    #[case::flag_and_empty_env(true, Some(""), false)]
    fn test_should_color(#[case] flag: bool, #[case] env: Option<&str>, #[case] expected: bool) {
        assert_eq!(should_color(flag, env.map(OsStr::new)), expected);
    }

    #[test]
    fn test_paint_strips_csi_when_off() {
        let s = "\x1b[1;31mred\x1b[0m plain \x1b[2Kx";
        assert_eq!(paint(s, false), "red plain x");
        assert_eq!(paint(s, true), s);
    }

    #[test]
    fn test_paint_keeps_non_escape_text() {
        assert_eq!(paint("✓ ok [1] ~", false), "✓ ok [1] ~");
    }
}
