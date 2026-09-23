//! Widget tests using ratatui TestBackend
//!
//! These tests verify dashboard rendering by checking terminal buffer contents
//! at fixed 80x24 dimensions.

use ci_tui::ui::app::StatusKind;
use ci_tui::ui::dashboard;
use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};

mod common;
use common::{make_test_app, make_test_app_all_passed, make_test_app_running, make_widget_check};

const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;

/// Create a test terminal with TestBackend at fixed dimensions
fn create_terminal() -> Terminal<TestBackend> {
    let backend = TestBackend::new(WIDTH, HEIGHT);
    Terminal::new(backend).expect("Failed to create terminal")
}

/// Helper to find a symbol in the buffer and return its foreground color
fn find_symbol_color(buffer: &Buffer, symbol: char) -> Option<Color> {
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.symbol() == symbol.to_string() {
                return Some(cell.fg);
            }
        }
    }
    None
}

/// Helper to find all colors for a symbol in the buffer
fn find_all_symbol_colors(buffer: &Buffer, symbol: char) -> Vec<Color> {
    let mut colors = Vec::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.symbol() == symbol.to_string() {
                colors.push(cell.fg);
            }
        }
    }
    colors
}

/// Helper to check if text appears anywhere in the buffer
fn buffer_contains(buffer: &Buffer, text: &str) -> bool {
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            line.push_str(&buffer[(x, y)].symbol());
        }
        if line.contains(text) {
            return true;
        }
    }
    false
}

/// Helper to count occurrences of a symbol in the buffer
fn count_symbol(buffer: &Buffer, symbol: char) -> usize {
    let mut count = 0;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if buffer[(x, y)].symbol() == symbol.to_string() {
                count += 1;
            }
        }
    }
    count
}

// ============================================================================
// Header Tests
// ============================================================================

#[test]
fn test_header_shows_branch_name() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "feature/test"),
        "Branch name should appear in header"
    );
}

#[test]
fn test_header_shows_file_count() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "2 files"),
        "File count should appear in header"
    );
}

#[test]
fn test_header_shows_elapsed_time() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Header renders elapsed time as "<digits>s" (e.g. "0s", "12s")
    let has_elapsed = (0..HEIGHT).any(|y| {
        let line: String = (0..WIDTH)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect::<Vec<_>>()
            .join("");
        line.as_bytes()
            .windows(2)
            .any(|w| w[0].is_ascii_digit() && w[1] == b's')
    });
    assert!(
        has_elapsed,
        "Elapsed time like '0s' should appear in header"
    );
}

#[test]
fn test_header_shows_progress() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Should show "Running..." or progress indicator
    assert!(
        buffer_contains(buffer, "Running") || buffer_contains(buffer, "passed"),
        "Progress status should appear in header"
    );
}

#[test]
fn test_header_shows_on_demand_count() {
    let mut app = make_test_app();
    // Flip one check's result to OnDemand; header shows "+1 on-demand"
    app.results.get_mut("clippy").unwrap().status = ci_tui::runner::CheckStatus::OnDemand;
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "+1 on-demand"),
        "Header should show on-demand count when an on-demand check exists"
    );
}

// ============================================================================
// Checks List Tests
// ============================================================================

#[test]
fn test_checks_list_shows_check_names() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "Clippy"),
        "Clippy check should appear"
    );
    assert!(
        buffer_contains(buffer, "Format Check"),
        "Format Check should appear"
    );
    assert!(
        buffer_contains(buffer, "Unit Tests"),
        "Unit Tests check should appear"
    );
}

#[test]
fn test_status_icon_passed_is_green() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Passed check uses green checkmark (✓)
    let checkmark_color = find_symbol_color(buffer, '✓');
    assert!(
        checkmark_color.is_some(),
        "Should find checkmark symbol for passed check"
    );
    assert_eq!(
        checkmark_color.unwrap(),
        Color::Green,
        "Checkmark should be green for passed check"
    );
}

#[test]
fn test_status_icon_failed_is_red() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Failed check uses red X (✗)
    let x_color = find_symbol_color(buffer, '✗');
    assert!(x_color.is_some(), "Should find X symbol for failed check");
    assert_eq!(
        x_color.unwrap(),
        Color::Red,
        "X should be red for failed check"
    );
}

#[test]
fn test_status_icon_pending_is_gray() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Pending check uses gray circle (○)
    let circle_color = find_symbol_color(buffer, '○');
    assert!(
        circle_color.is_some(),
        "Should find circle symbol for pending check"
    );
    assert_eq!(
        circle_color.unwrap(),
        Color::DarkGray,
        "Circle should be dark gray for pending check"
    );
}

#[test]
fn test_status_icon_running_is_yellow() {
    let mut app = make_test_app_running();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Running check uses yellow dot (●)
    let dot_color = find_symbol_color(buffer, '●');
    assert!(
        dot_color.is_some(),
        "Should find dot symbol for running check"
    );
    assert_eq!(
        dot_color.unwrap(),
        Color::Yellow,
        "Dot should be yellow for running check"
    );
}

#[test]
fn test_checks_list_shows_group_names() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Groups appear in uppercase
    assert!(
        buffer_contains(buffer, "LINT") || buffer_contains(buffer, "lint"),
        "Lint group should appear"
    );
    assert!(
        buffer_contains(buffer, "TEST") || buffer_contains(buffer, "test"),
        "Test group should appear"
    );
}

#[test]
fn test_failed_filter_hides_non_failed_checks() {
    let mut app = make_test_app();
    app.toggle_failed_filter();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Only the failed check remains visible
    assert!(
        buffer_contains(buffer, "Unit Tests"),
        "Failed check should be rendered"
    );
    assert!(
        !buffer_contains(buffer, "Format Check"),
        "Passed check should be hidden under Failed filter"
    );
    assert!(
        !buffer_contains(buffer, "Clippy"),
        "Pending check should be hidden under Failed filter"
    );
    // Group with no failed checks hides its header entirely
    assert!(
        !buffer_contains(buffer, "LINT"),
        "Group with no visible items should not render its header"
    );
}

// ============================================================================
// Status Colors Verification
// ============================================================================

#[test]
fn test_all_status_colors_present_in_mixed_state() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Should have green (passed), red (failed), and gray (pending)
    let has_green = find_symbol_color(buffer, '✓') == Some(Color::Green);
    let has_red = find_symbol_color(buffer, '✗') == Some(Color::Red);
    let has_gray = find_symbol_color(buffer, '○') == Some(Color::DarkGray);

    assert!(has_green, "Should have green checkmark for passed check");
    assert!(has_red, "Should have red X for failed check");
    assert!(has_gray, "Should have gray circle for pending check");
}

#[test]
fn test_all_passed_state_shows_only_green() {
    let mut app = make_test_app_all_passed();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Count checkmarks - should have 3 (one for each check)
    let checkmark_count = count_symbol(buffer, '✓');
    assert!(
        checkmark_count >= 3,
        "Should have at least 3 checkmarks for 3 passed checks, found {}",
        checkmark_count
    );

    // Verify ALL checkmarks are green
    // Note: The header might have a checkmark with gauge background color,
    // so we check that at least 3 checkmarks are green (one for each check)
    let checkmark_colors = find_all_symbol_colors(buffer, '✓');
    assert!(
        checkmark_colors.len() >= 3,
        "Should have found at least 3 checkmarks, found {}",
        checkmark_colors.len()
    );
    let green_count = checkmark_colors
        .iter()
        .filter(|&c| *c == Color::Green)
        .count();
    assert!(
        green_count >= 3,
        "At least 3 checkmarks should be green (one per check), found {} green out of {} total",
        green_count,
        checkmark_colors.len()
    );
}

// ============================================================================
// Output Panel Tests
// ============================================================================

#[test]
fn test_output_panel_shows_selected_check_name() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // First check (clippy) should be selected by default
    // The name should appear in the output panel title
    assert!(
        buffer_contains(buffer, "Clippy"),
        "Selected check name should appear in output panel"
    );
}

#[test]
fn test_output_panel_shows_error_for_failed_check() {
    let mut app = make_test_app();
    // Select the failed check (unit tests)
    app.next_check(); // Move from clippy to fmt
    app.next_check(); // Move from fmt to unit
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Should show FAILED status
    assert!(
        buffer_contains(buffer, "FAILED"),
        "Failed check should show FAILED status"
    );
    assert!(
        buffer_contains(buffer, "stderr") || buffer_contains(buffer, "STDERR"),
        "Failed check should show stderr section"
    );
}

#[test]
fn test_output_panel_shows_passed_status() {
    let mut app = make_test_app();
    // Select the passed check (fmt)
    app.next_check(); // Move from clippy to fmt
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "PASSED"),
        "Passed check should show PASSED status"
    );
}

#[test]
fn test_output_panel_shows_pending_status() {
    let mut app = make_test_app();
    // Clippy is pending by default and should be selected
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "PENDING") || buffer_contains(buffer, "Waiting"),
        "Pending check should show pending status"
    );
}

// ============================================================================
// Footer Tests
// ============================================================================

#[test]
fn test_footer_status_icon_colored_by_kind() {
    let cases = [
        (StatusKind::info(), 'ℹ', Color::Cyan),
        (StatusKind::Error, '✗', Color::Red),
        (StatusKind::progress_for("phpunit"), '⟳', Color::Yellow),
    ];
    for (kind, icon, color) in cases {
        let mut app = make_test_app();
        app.set_status_message(kind, "msg");
        let mut terminal = create_terminal();
        terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
        let buffer = terminal.backend().buffer();

        // Footer is the lowest row showing the icon (check list may use ✗ too)
        let cell = (0..buffer.area.height)
            .rev()
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .map(|pos| &buffer[pos])
            .find(|c| c.symbol() == icon.to_string())
            .unwrap_or_else(|| panic!("footer should show {icon}"));
        assert_eq!(cell.fg, color, "{icon} color");
    }
}

#[test]
fn test_footer_shows_quit_shortcut() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "quit"),
        "Footer should show quit shortcut"
    );
}

#[test]
fn test_footer_shows_navigation_shortcuts() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "select"),
        "Footer should show select shortcut"
    );
}

#[test]
fn test_footer_shows_filter_shortcuts() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "failed") || buffer_contains(buffer, "all"),
        "Footer should show filter shortcuts"
    );
}

#[test]
fn test_footer_shows_expand_shortcut() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "expand") || buffer_contains(buffer, "collapse"),
        "Footer should show expand shortcut"
    );
}

#[test]
fn test_footer_shows_version_info() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "built"),
        "Footer should show build info"
    );
}

// ============================================================================
// System Stats Tests
// ============================================================================

#[test]
fn test_system_stats_cpu_appears() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(buffer_contains(buffer, "CPU"), "CPU stats should appear");
}

#[test]
fn test_system_stats_memory_appears() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(buffer_contains(buffer, "MEM"), "Memory stats should appear");
}

// ============================================================================
// Files List Tests
// ============================================================================

#[test]
fn test_files_list_shows_changed_files() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "main.rs"),
        "Changed files should appear"
    );
    assert!(
        buffer_contains(buffer, "lib.rs"),
        "Changed files should appear"
    );
}

#[test]
fn test_files_list_shows_count() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "Files (2)"),
        "Files count should appear"
    );
}

// ============================================================================
// Regression Tests
// ============================================================================

/// Helper: text of the left (checks) column of the buffer
fn left_column_contains(buffer: &Buffer, text: &str) -> bool {
    (0..buffer.area.height).any(|y| {
        let line: String = (0..WIDTH * 4 / 10)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect();
        line.contains(text)
    })
}

#[test]
fn test_multibyte_command_truncation_does_not_panic() {
    let mut app = make_test_app();
    let check = &mut app.checks[0];
    check.resolved_command = format!("clippy {}", "src/Zażółć_gęślą.rs ".repeat(20));
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    assert!(buffer_contains(terminal.backend().buffer(), "[e=expand]"));
}

#[test]
fn test_checks_list_scrolls_to_selected_check() {
    let mut app = make_test_app();
    for i in 0..6 {
        let id = format!("extra{}", i);
        app.checks.push(make_widget_check(
            &id,
            "test",
            &format!("Extra {}", i),
            false,
        ));
        app.results
            .insert(id.clone(), ci_tui::runner::CheckResult::pending(&id));
    }
    // next_check stops at the last item
    for _ in 0..app.checks.len() {
        app.next_check();
    }
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    assert!(
        left_column_contains(terminal.backend().buffer(), "Extra 5"),
        "selected last check must be scrolled into view"
    );

    // Moving up inside the visible window must not scroll the list back.
    // Without a persisted offset, ratatui recomputes from the top and the
    // bottom rows (Extra 5) drop out of view.
    for _ in 0..3 {
        app.previous_check();
    }
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    assert!(
        left_column_contains(terminal.backend().buffer(), "Extra 5"),
        "list offset must persist while the selection stays visible"
    );
}

#[test]
fn test_timed_out_check_shows_distinct_icon_and_label() {
    let mut app = make_test_app();
    // clippy is selected by default
    let clippy = app.results.get_mut("clippy").unwrap();
    clippy.status = ci_tui::runner::CheckStatus::TimedOut;
    clippy.error_output = "timed out after 1s".to_string();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert_eq!(find_symbol_color(buffer, '⧗'), Some(Color::Magenta));
    assert!(buffer_contains(buffer, "TIMED OUT"));
    assert!(buffer_contains(buffer, "timed out after 1s"));
}
