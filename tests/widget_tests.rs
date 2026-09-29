//! Widget tests using ratatui TestBackend
//!
//! These tests verify dashboard rendering by checking terminal buffer contents
//! at 80x24 by default (`create_terminal`); some tests use a wider backend.

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
    (0..buffer.area.height).any(|y| row_text(buffer, y, 0, buffer.area.width).contains(text))
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
fn test_status_icon_queued_is_blue() {
    let mut app = make_test_app_running();
    app.results.get_mut("clippy").unwrap().status = ci_tui::runner::CheckStatus::Queued;
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // Queued check (waiting for a max_parallel slot) uses a blue dotted circle
    assert_eq!(find_symbol_color(buffer, '◌'), Some(Color::Blue));
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
    app.next_check(); // Move from fmt to the test group header
    app.next_check(); // Move from header to unit
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
fn test_footer_shows_help_shortcut() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        buffer_contains(buffer, "? help"),
        "Footer should show help shortcut"
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

    // Footer row only (the one with "quit"): the output panel can also print an expand hint
    let footer = (0..HEIGHT)
        .map(|y| {
            (0..WIDTH)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .find(|row| row.contains(" quit "))
        .expect("footer row with quit shortcut");
    assert!(
        footer.contains(" expand "),
        "Footer should show expand shortcut, got: {footer}"
    );
}

#[test]
fn test_footer_shows_version_info() {
    let mut app = make_test_app();
    // Version info is the footer's last span; it is truncated first on 80 cols
    let mut terminal = Terminal::new(TestBackend::new(120, HEIGHT)).unwrap();
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

/// Text of buffer row `y` between columns `x0..x1`
fn row_text(buffer: &Buffer, y: u16, x0: u16, x1: u16) -> String {
    (x0..x1)
        .map(|x| buffer[(x, y)].symbol().to_string())
        .collect()
}

/// Stats panels: top border at row 3, four inner rows 4..=7 (header is 3 rows)
const STATS_INNER_TOP: u16 = 4;
const STATS_INNER_BOTTOM: u16 = 7;

/// #192: CPU graph spans the whole panel on a wide terminal once enough
/// samples arrived (history no longer capped below panel width)
#[test]
fn test_cpu_graph_fills_full_width_on_wide_terminal() {
    let mut app = make_test_app();
    for _ in 0..600 {
        app.update_stats(100.0, 8_000_000_000, 16_000_000_000);
    }
    // CPU panel = left half (0..150): border at 0 and 149, scale in 1..5
    let mut terminal = Terminal::new(TestBackend::new(300, HEIGHT)).unwrap();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    let graph = row_text(buffer, STATS_INNER_BOTTOM, 5, 149);
    assert_eq!(graph, "█".repeat(144), "graph must fill full width");
}

/// #192: CPU panel shows a 100/50/0 y-axis scale left of the graph
#[test]
fn test_cpu_panel_shows_scale_labels() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    let scale: Vec<String> = (STATS_INNER_TOP..=STATS_INNER_BOTTOM)
        .map(|y| row_text(buffer, y, 1, 5))
        .collect();
    assert_eq!(scale, ["100 ", "    ", " 50 ", "  0 "]);
}

/// #192: MEM panel is a gauge with a readable used/total label
#[test]
fn test_mem_panel_gauge_label() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // 8e9 / 16e9 bytes = 7.5 / 14.9 GiB
    assert!(
        buffer_contains(buffer, "7.5/14.9 GiB (50%)"),
        "MEM gauge label missing"
    );
}

/// #192: zero total memory (no sample yet / broken sysinfo) renders, no panic
#[test]
fn test_mem_panel_zero_total_renders() {
    let mut app = make_test_app();
    app.update_stats(10.0, 0, 0);
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();

    assert!(buffer_contains(
        terminal.backend().buffer(),
        "0.0/0.0 GiB (0%)"
    ));
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

#[test]
fn test_footer_shows_cancel_only_for_running_check() {
    // clippy is selected and running
    let mut app = make_test_app_running();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    assert!(buffer_contains(terminal.backend().buffer(), "s cancel"));

    let mut app = make_test_app();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    assert!(!buffer_contains(terminal.backend().buffer(), "cancel"));
}

#[test]
fn test_cancelled_check_shows_label() {
    let mut app = make_test_app();
    // clippy is selected by default
    let started_at = chrono::Local::now();
    *app.results.get_mut("clippy").unwrap() =
        ci_tui::runner::CheckResult::cancelled("clippy", started_at);
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(buffer_contains(buffer, "CANCELLED"));
    assert!(buffer_contains(buffer, "cancelled by user"));
}

#[test]
fn test_output_panel_shows_streamed_stdout_and_stderr_while_running() {
    // clippy is selected and running
    let mut app = make_test_app_running();
    app.handle_runner_event(ci_tui::runner::RunnerEvent::CheckOutput {
        check_id: "clippy".to_string(),
        stdout: "Checking ci-tui\n".to_string(),
        stderr: "warning: unused variable\n".to_string(),
    });
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(buffer_contains(buffer, "RUNNING"));
    assert!(buffer_contains(buffer, "Checking ci-tui"));
    assert!(buffer_contains(buffer, "stderr"));
    assert!(buffer_contains(buffer, "warning: unused variable"));
}

// ============================================================================
// Collapsible Groups
// ============================================================================

#[test]
fn test_group_header_shows_fold_marker_and_child_count() {
    let mut app = make_test_app();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    assert!(left_column_contains(terminal.backend().buffer(), "▾ LINT"));

    app.select_first(); // lint header
    app.toggle_selected_group();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(left_column_contains(buffer, "▸ LINT (2)"));
    assert!(!left_column_contains(buffer, "Clippy"), "children hidden");
    assert!(
        left_column_contains(buffer, "Unit Tests"),
        "other group open"
    );
}

#[test]
fn test_output_panel_empty_when_header_selected() {
    let mut app = make_test_app();
    app.select_first(); // lint header
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();

    assert!(buffer_contains(
        terminal.backend().buffer(),
        "Select a check to view details"
    ));
}

#[test]
fn test_help_overlay_lists_fold_key() {
    let mut app = make_test_app();
    app.toggle_help();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();

    assert!(buffer_contains(
        terminal.backend().buffer(),
        "Space/Enter fold group"
    ));
}

// ============================================================================
// #143: NO_COLOR, small terminal, hidden stats panel
// ============================================================================

/// Render `app` on a `width` x `height` TestBackend
fn render_at(app: &mut ci_tui::ui::app::App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| dashboard::render(app, f)).unwrap();
    terminal.backend().buffer().clone()
}

fn has_color(buffer: &Buffer) -> bool {
    buffer
        .content()
        .iter()
        .any(|c| c.fg != Color::Reset || c.bg != Color::Reset)
}

#[test]
fn test_no_color_renders_no_fg_bg_colors() {
    let mut app = make_test_app();
    app.update_stats(90.0, 8_000_000_000, 16_000_000_000);
    assert!(has_color(&render_at(&mut app, WIDTH, HEIGHT)), "color on");

    app.color = false;
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    assert!(!has_color(&buffer), "every cell fg/bg must be Reset");
    assert!(buffer_contains(&buffer, "CPU"), "layout still rendered");
}

#[rstest::rstest]
#[case::narrow(59, 24)]
#[case::short(80, 20)]
fn test_too_small_terminal_shows_message_only(#[case] width: u16, #[case] height: u16) {
    let mut app = make_test_app();
    let buffer = render_at(&mut app, width, height);
    assert!(buffer_contains(&buffer, "Terminal too small"));
    assert!(buffer_contains(&buffer, &format!("{width}x{height}")));
    assert!(buffer_contains(&buffer, "need 60x21"));
    assert!(!buffer_contains(&buffer, "CPU") && !buffer_contains(&buffer, "Checks"));
}

#[test]
fn test_min_size_terminal_renders_layout() {
    let mut app = make_test_app();
    let buffer = render_at(&mut app, 60, 21);
    assert!(!buffer_contains(&buffer, "too small"));
    assert!(buffer_contains(&buffer, "CPU") && buffer_contains(&buffer, "Checks"));
}

#[test]
fn test_stats_hidden_renders_no_cpu_mem_panel() {
    let mut app = make_test_app();
    assert!(app.stats_visible());
    app.toggle_stats();
    assert!(!app.stats_visible());
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    assert!(!buffer_contains(&buffer, "CPU") && !buffer_contains(&buffer, "MEM"));
    // Main content moves up into the stats panel's rows (header is 3 rows)
    assert!(row_text(&buffer, 3, 0, WIDTH).contains("Checks"));
}

#[test]
fn test_stats_hidden_lowers_min_height_by_stats_rows() {
    let mut app = make_test_app();
    app.toggle_stats();
    assert!(!buffer_contains(&render_at(&mut app, 60, 15), "too small"));
    let buffer = render_at(&mut app, 60, 14);
    assert!(buffer_contains(&buffer, "Terminal too small"));
    assert!(buffer_contains(&buffer, "need 60x15"));
}

/// Color off: gauge fill has no bg color left, so filled cells must show as
/// `█` or reversed (under the label) and unfilled cells as neither
#[test]
fn test_no_color_header_gauge_fill_visible() {
    let mut app = make_test_app(); // 2 of 3 auto-run checks done
    app.color = false;
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    let filled = |x: u16| {
        let cell = &buffer[(x, 1)];
        cell.symbol() == "█" || cell.modifier.contains(ratatui::style::Modifier::REVERSED)
    };
    // Inner gauge row 1, cols 1..79; fill ends at 1 + round(78 * 2/3)
    let end = 1 + (78.0_f64 * 2.0 / 3.0).round() as u16;
    assert!((1..end).all(filled), "filled part distinguishable");
    assert!(!(end..WIDTH - 1).any(filled), "unfilled part plain");
}

#[rstest::rstest]
#[case::with_stats(21, true)]
#[case::without_stats(15, false)]
fn test_help_overlay_fits_min_size(#[case] height: u16, #[case] stats: bool) {
    let mut app = make_test_app();
    if !stats {
        app.toggle_stats();
    }
    app.toggle_help();
    let buffer = render_at(&mut app, 60, height);
    assert!(!buffer_contains(&buffer, "too small"));
    assert!(buffer_contains(&buffer, "Press ? or Esc to close"));
    assert!(buffer_contains(&buffer, "CPU/MEM stats"));
}

// ============================================================================
// #150: result cache
// ============================================================================

#[test]
fn test_cached_check_row_shows_cached_and_check_mark() {
    let mut app = make_test_app();
    app.mark_cached(["clippy"]);
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    assert!(buffer_contains(&buffer, "✓ Clippy cached"));
}
