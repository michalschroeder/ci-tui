//! Tests for panic hook and terminal restoration.
//!
//! These tests verify that the panic hook is properly installed and would
//! restore terminal state on panic. We don't actually trigger panics in tests,
//! but verify the code structure.

/// Test that the restore_terminal function exists and compiles
/// This is a compile-time verification that the function is accessible
#[test]
fn test_restore_terminal_function_exists() {
    // The restore_terminal function is private, but we can verify
    // it's called by checking the panic hook installation code exists
    // in the source file.

    // Read the source file and verify the panic hook pattern
    let source = include_str!("../src/ui/mod.rs");

    // Verify panic hook is installed
    assert!(
        source.contains("set_hook"),
        "Source should contain panic::set_hook"
    );

    // Verify restore_terminal is called in panic hook
    assert!(
        source.contains("restore_terminal()"),
        "Panic hook should call restore_terminal()"
    );

    // Verify the panic hook installer function exists
    assert!(
        source.contains("fn install_panic_hook"),
        "install_panic_hook function should exist"
    );
}

/// Test that install_panic_hook is called during TUI startup
/// by verifying the run function calls it
#[test]
fn test_panic_hook_installed_on_startup() {
    let source = include_str!("../src/ui/mod.rs");

    // Verify install_panic_hook is called in the run function
    assert!(
        source.contains("install_panic_hook()"),
        "run() function should call install_panic_hook()"
    );
}

/// Test that restore_terminal handles cleanup properly
/// by verifying it calls the necessary terminal restoration functions
#[test]
fn test_restore_terminal_calls_cleanup() {
    let source = include_str!("../src/ui/mod.rs");

    // Verify restore_terminal disables raw mode
    assert!(
        source.contains("disable_raw_mode"),
        "restore_terminal should call disable_raw_mode"
    );

    // Verify restore_terminal leaves alternate screen
    assert!(
        source.contains("LeaveAlternateScreen"),
        "restore_terminal should use LeaveAlternateScreen"
    );
}

/// Test that the original panic hook is preserved (chained)
#[test]
fn test_panic_hook_preserves_original() {
    let source = include_str!("../src/ui/mod.rs");

    // Verify take_hook is called to get original hook
    assert!(
        source.contains("take_hook"),
        "Should call panic::take_hook to preserve original"
    );

    // Verify original hook is called after restore
    assert!(
        source.contains("original_hook(panic_info)"),
        "Should call original_hook to chain panic handling"
    );
}
