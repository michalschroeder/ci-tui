//! End-of-run notification (`notify: true` / `--notify`): terminal bell plus
//! an OSC 9 desktop notification, sent when a full run finishes.
//!
//! OSC 9 only (no OSC 777 / 99, no per-terminal detection): terminals that
//! do not know it ignore it. Inside tmux or GNU screen the OSC goes through
//! DCS passthrough (tmux needs `set -g allow-passthrough on`); the bell does
//! not need it.

use std::io::{self, Write};

/// Terminal multiplexer between ci-tui and the terminal
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mux {
    None,
    Tmux,
    Screen,
}

impl Mux {
    /// From the environment: non-empty `$TMUX`, else non-empty `$STY` (screen)
    fn detect() -> Self {
        let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        if set("TMUX") {
            Self::Tmux
        } else if set("STY") {
            Self::Screen
        } else {
            Self::None
        }
    }
}

/// Bytes to write for `msg`: BEL, then `ESC ] 9 ; msg BEL` with control
/// characters dropped from `msg` (they would end or corrupt the sequence).
/// Under a multiplexer the OSC is wrapped in DCS passthrough (`ESC P ... ESC \`;
/// tmux: `tmux;` prefix, inner ESCs doubled); the BEL stays outside (the
/// multiplexer forwards bells itself).
pub fn sequence(msg: &str, mux: Mux) -> String {
    let msg: String = msg.chars().filter(|c| !c.is_control()).collect();
    let osc = format!("\x1b]9;{msg}\x07");
    match mux {
        Mux::None => format!("\x07{osc}"),
        Mux::Tmux => format!("\x07\x1bPtmux;{}\x1b\\", osc.replace('\x1b', "\x1b\x1b")),
        Mux::Screen => format!("\x07\x1bP{osc}\x1b\\"),
    }
}

/// Write the notification for `msg` to `w` (the TUI's terminal) and flush
pub fn emit(w: &mut impl Write, msg: &str) -> io::Result<()> {
    w.write_all(sequence(msg, Mux::detect()).as_bytes())?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence_plain() {
        assert_eq!(
            sequence("ci-tui: 2 passed, 0 failed", Mux::None),
            "\x07\x1b]9;ci-tui: 2 passed, 0 failed\x07"
        );
    }

    #[test]
    fn test_sequence_tmux_wraps_osc_only() {
        assert_eq!(
            sequence("done", Mux::Tmux),
            "\x07\x1bPtmux;\x1b\x1b]9;done\x07\x1b\\"
        );
    }

    #[test]
    fn test_sequence_screen_wraps_osc_only() {
        assert_eq!(
            sequence("done", Mux::Screen),
            "\x07\x1bP\x1b]9;done\x07\x1b\\"
        );
    }

    #[test]
    fn test_sequence_strips_control_chars() {
        assert_eq!(
            sequence("a\x07b\x1b]c\nd", Mux::None),
            "\x07\x1b]9;ab]cd\x07"
        );
    }

    #[test]
    fn test_emit_writes_and_flushes() {
        let mut out = io::BufWriter::new(Vec::new());
        emit(&mut out, "x").unwrap();
        let written = String::from_utf8(out.buffer().to_vec()).unwrap();
        assert!(written.is_empty(), "flushed: nothing left buffered");
        assert!(out.into_inner().unwrap().starts_with(b"\x07"), "bell first");
    }
}
