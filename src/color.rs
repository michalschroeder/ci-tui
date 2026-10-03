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
/// (`ESC [`, param/intermediate bytes `0x20..=0x3F`, final byte
/// `0x40..=0x7E`) and OSC sequences (`ESC ]` up to BEL or `ESC \`, e.g.
/// hyperlinks, window titles), incl. escapes in check output. A truncated
/// CSI ends at the first other char, an unterminated OSC at a newline; both
/// are kept as text.
pub fn paint(s: &str, color: bool) -> Cow<'_, str> {
    if color || !s.contains('\x1b') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
        } else if chars.next_if_eq(&'[').is_some() {
            skip_csi(&mut chars);
        } else if chars.next_if_eq(&']').is_some() {
            skip_osc(&mut chars);
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Skip a CSI body after `ESC [`: params / intermediates, then the final byte
fn skip_csi(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars.next_if(|c| (' '..='?').contains(c)).is_some() {}
    let _ = chars.next_if(|c| ('@'..='~').contains(c));
}

/// Skip an OSC body after `ESC ]`: through BEL or `ESC \`, or up to (not
/// incl.) a newline when unterminated
fn skip_osc(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(c) = chars.next_if(|&c| c != '\n') {
        if c == '\x07' || c == '\x1b' && chars.next_if_eq(&'\\').is_some() {
            return;
        }
    }
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

/// `print!` honoring the console color switch ([`enabled`])
macro_rules! cprint {
    ($($arg:tt)*) => {
        print!(
            "{}",
            $crate::color::paint(&format!($($arg)*), $crate::color::enabled())
        )
    };
}
pub(crate) use cprint;

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
    fn test_paint_strips_osc_when_off() {
        let link = "\x1b]8;;https://x\x1b\\file.rs\x1b]8;;\x1b\\:1";
        assert_eq!(paint(link, false), "file.rs:1");
        assert_eq!(paint("\x1b]0;title\x07ok", false), "ok");
        // Unterminated: ends at the newline, which is kept
        assert_eq!(paint("a\x1b]0;title\nb", false), "a\nb");
        assert_eq!(paint(link, true), link);
    }

    #[test]
    fn test_paint_malformed_csi_keeps_following_text() {
        // Truncated CSI: `\n` is not a param/intermediate byte, so it ends it
        assert_eq!(paint("\x1b[31\nerror", false), "\nerror");
        assert_eq!(paint("a\x1b[", false), "a");
    }

    #[test]
    fn test_paint_keeps_non_escape_text() {
        assert_eq!(paint("✓ ok [1] ~", false), "✓ ok [1] ~");
    }
}
