//! End-of-run notification (`notify: true` / `--notify`): terminal bell plus
//! an OSC 9 desktop notification, sent when a full run finishes.
//!
//! OSC 9 only (no OSC 777 / 99, no per-terminal detection): terminals that
//! do not know it ignore it. Inside tmux the OSC goes through DCS passthrough
//! (needs `set -g allow-passthrough on`); the bell does not need it.

use std::io::{self, Write};

/// Bytes to write for `msg`: BEL, then `ESC ] 9 ; msg BEL` with control
/// characters dropped from `msg` (they would end or corrupt the sequence).
/// `in_tmux` wraps the OSC in tmux DCS passthrough (`ESC P tmux; ... ESC \`,
/// inner ESCs doubled); the BEL stays outside (tmux forwards bells itself).
pub fn sequence(msg: &str, in_tmux: bool) -> String {
    let msg: String = msg.chars().filter(|c| !c.is_control()).collect();
    let osc = format!("\x1b]9;{msg}\x07");
    if in_tmux {
        format!("\x07\x1bPtmux;{}\x1b\\", osc.replace('\x1b', "\x1b\x1b"))
    } else {
        format!("\x07{osc}")
    }
}

/// True inside tmux (non-empty `$TMUX`)
fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some_and(|v| !v.is_empty())
}

/// Write the notification for `msg` to `w` (the TUI's terminal) and flush
pub fn emit(w: &mut impl Write, msg: &str) -> io::Result<()> {
    w.write_all(sequence(msg, in_tmux()).as_bytes())?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence_plain() {
        assert_eq!(
            sequence("ci-tui: 2 passed, 0 failed", false),
            "\x07\x1b]9;ci-tui: 2 passed, 0 failed\x07"
        );
    }

    #[test]
    fn test_sequence_tmux_wraps_osc_only() {
        assert_eq!(
            sequence("done", true),
            "\x07\x1bPtmux;\x1b\x1b]9;done\x07\x1b\\"
        );
    }

    #[test]
    fn test_sequence_strips_control_chars() {
        assert_eq!(sequence("a\x07b\x1b]c\nd", false), "\x07\x1b]9;ab]cd\x07");
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
