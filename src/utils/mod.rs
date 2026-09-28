//! Internal utilities for CI-TUI.
//!
//! This module contains shared utility functions used across the crate.
//! All functions are `pub(crate)` - not part of the public API.

pub(crate) mod docker;
pub(crate) mod shell;
pub(crate) mod time;

/// Put `cmd` in its own process group, outside the terminal's foreground
/// group: a Ctrl-C meant for `$PAGER` (TUI suspended) then does not kill it.
/// Its stdin must not be the terminal (a background group reading it gets
/// SIGTTIN).
pub(crate) fn own_process_group(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    cmd
}
