//! Widget tests using ratatui TestBackend
//!
//! These tests verify dashboard rendering by checking terminal buffer contents
//! at 80x24 by default (`create_terminal`); some tests use a wider backend.

// Test code: nesting limit targets production code (see src/lib.rs)
#![allow(clippy::excessive_nesting)]

use ci_tui::ui::app::StatusKind;
use ci_tui::ui::dashboard;
use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};

mod common;
use common::{
    make_test_app, make_test_app_all_passed, make_test_app_on_branch, make_test_app_running,
    make_test_app_with_statuses, make_widget_check,
};

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

/// Text of buffer row `y`
fn full_row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol().to_string())
        .collect()
}

/// Rows (at column 10) the output panel covers, for layout comparisons
fn output_rows(app: &ci_tui::ui::app::App) -> Vec<bool> {
    (0..24).map(|y| app.is_over_output(70, y)).collect()
}

/// App with the failed, fixable `fmt` check selected (`x fix` and
/// `X FIX ALL(1)` both apply)
fn fixable_failed_app() -> ci_tui::ui::app::App {
    let mut app = make_test_app();
    app.results.get_mut("fmt").unwrap().status = ci_tui::runner::CheckStatus::Failed;
    for _ in 0..10 {
        if app.selected_check().map(|c| c.id()) == Some("fmt") {
            break;
        }
        app.next_check();
    }
    assert_eq!(app.selected_check().map(|c| c.id()), Some("fmt"));
    app
}

/// Footer text: the bottom two rows of a 24-row render
fn footer_text(buffer: &Buffer) -> String {
    format!("{}\n{}", full_row(buffer, 22), full_row(buffer, 23))
}

/// #300 criterion 1: both fix hints fit at 80 columns
#[test]
fn test_footer_shows_fix_hints_at_80_columns() {
    let mut app = fixable_failed_app();
    let footer = footer_text(&render_at(&mut app, 80, 24));
    assert!(footer.contains("x fix"), "footer: {footer}");
    assert!(footer.contains("X FIX ALL(1)"), "footer: {footer}");
}

/// #300 criterion 2: state-specific hints first, generic last
#[test]
fn test_footer_orders_state_hints_before_generic() {
    let mut app = fixable_failed_app();
    let footer = footer_text(&render_at(&mut app, 80, 24));
    let hints = [
        "r retry",
        "A all files",
        "R RETRY ALL",
        "x fix",
        "X FIX ALL(1)",
        "q quit",
        "? help",
        "↑↓ select",
        "f failed",
        "c copy",
        "e expand",
    ];
    let order: Vec<usize> = hints
        .iter()
        .map(|h| footer.find(h).unwrap_or_else(|| panic!("{h} in {footer}")))
        .collect();
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "order {order:?}: {footer}"
    );
}

/// #300 criteria 2 and 3: every hint stays whole (never split across rows)
/// at every width from the minimum up
#[test]
fn test_footer_keeps_every_hint_whole_at_all_widths() {
    for width in 60..=140 {
        let mut app = fixable_failed_app();
        let footer = footer_text(&render_at(&mut app, width, 24));
        for hint in ["x fix", "X FIX ALL(1)", "q quit", "c copy", "e expand"] {
            assert!(footer.contains(hint), "width {width} lost {hint}: {footer}");
        }
    }
}

/// #300 criterion 4: when everything fits the footer is one row and the
/// main area gets the other; with two rows of hints it keeps both
#[test]
fn test_footer_rows_follow_hints_and_resize_main_area() {
    let mut one = make_test_app();
    let buf = render_at(&mut one, 80, 24);
    assert!(
        full_row(&buf, 23).contains(" quit "),
        "one-row footer on the last row"
    );
    assert!(!full_row(&buf, 22).contains(" quit "));
    // Output panel's last inner row is 21 (border at 22): hit-testing follows
    assert!(
        one.is_over_output(70, 21) && !one.is_over_output(70, 22),
        "{:?}",
        output_rows(&one)
    );

    let mut two = fixable_failed_app();
    let buf = render_at(&mut two, 80, 24);
    assert!(
        full_row(&buf, 22).contains("retry"),
        "row 22: {}",
        full_row(&buf, 22)
    );
    assert!(full_row(&buf, 23).contains("expand"));
    assert!(two.is_over_output(70, 20) && !two.is_over_output(70, 21));
}

/// #300 criterion 4: a status message shows on the footer's first row and
/// does not change the layout
#[test]
fn test_status_message_does_not_move_layout() {
    for mut app in [make_test_app(), fixable_failed_app()] {
        let before = render_at(&mut app, 80, 24);
        let area = output_rows(&app);
        let first_footer_row = if full_row(&before, 22).contains("quit") {
            22
        } else {
            23
        };
        app.set_status_message(StatusKind::info(), "hello status");
        let after = render_at(&mut app, 80, 24);
        assert_eq!(output_rows(&app), area, "layout jumped");
        assert!(
            full_row(&after, first_footer_row).contains("hello status"),
            "row {first_footer_row}: {}",
            full_row(&after, first_footer_row)
        );
    }
}

/// #300 guard: with a two-row footer the search line stays on its first row
#[test]
fn test_search_footer_on_first_footer_row() {
    let mut app = fixable_failed_app();
    app.open_search();
    let buf = render_at(&mut app, 80, 24);
    assert!(
        full_row(&buf, 22).contains("Search:"),
        "{}",
        full_row(&buf, 22)
    );
    assert!(!full_row(&buf, 23).contains("quit"));
}

/// #300: fix running hides the fix hints; the footer shrinks to one row
#[test]
fn test_footer_one_row_while_fix_running() {
    let mut app = fixable_failed_app();
    app.start_fix();
    let buf = render_at(&mut app, 80, 24);
    let footer = footer_text(&buf);
    assert!(!footer.contains("fix"), "footer: {footer}");
    assert!(full_row(&buf, 23).contains("quit") && !full_row(&buf, 22).contains("quit"));
}

/// #300: nothing selectable (no checks) is a one-row footer
#[test]
fn test_footer_one_row_without_checks() {
    let mut app = make_test_app();
    app.checks.clear();
    app.results.clear();
    let buf = render_at(&mut app, 80, 24);
    assert!(
        full_row(&buf, 23).contains("quit"),
        "{}",
        full_row(&buf, 23)
    );
    assert!(!full_row(&buf, 22).contains("quit"));
}

/// #300 criterion 5: the footer no longer carries the version
#[test]
fn test_footer_does_not_show_version_info() {
    let mut app = make_test_app();
    let footer = footer_text(&render_at(&mut app, 140, 24));
    assert!(!footer.contains("built"), "footer: {footer}");
    assert!(
        !footer.contains(ci_tui::cli::VERSION_INFO),
        "footer: {footer}"
    );
}

/// #300 criterion 6: the help overlay shows the version, also at the
/// minimum size (60x13: the overlay stays 13 rows)
#[test]
fn test_help_overlay_shows_version_info() {
    for (w, h) in [(60, 13), (80, 24)] {
        let mut app = make_test_app();
        app.toggle_help();
        let buf = render_at(&mut app, w, h);
        let text = (0..h)
            .map(|y| full_row(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        let version = format!("v{}", ci_tui::cli::VERSION_INFO);
        assert!(
            text.contains(&version),
            "{w}x{h} missing {version}:\n{text}"
        );
        assert!(text.contains("Esc to close"), "{w}x{h}:\n{text}");
        assert!(text.contains(" Help "), "{w}x{h}:\n{text}");
    }
}

// ============================================================================
// System Stats Tests
// ============================================================================

#[test]
fn test_system_stats_cpu_appears() {
    let mut app = make_test_app();
    app.toggle_stats();
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(
        row_text(buffer, 1, 0, WIDTH).contains("┌ CPU 78% "),
        "CPU panel should appear"
    );
}

#[test]
fn test_system_stats_memory_appears() {
    let mut app = make_test_app();
    app.toggle_stats();
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

/// Stats panels: top border at row 1, four inner rows 2..=5 (header is 1 row)
const STATS_INNER_TOP: u16 = 2;
const STATS_INNER_BOTTOM: u16 = 5;

/// #192: CPU graph spans the whole panel on a wide terminal once enough
/// samples arrived (history no longer capped below panel width)
#[test]
fn test_cpu_graph_fills_full_width_on_wide_terminal() {
    let mut app = make_test_app();
    app.toggle_stats();
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
    app.toggle_stats();
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
    app.toggle_stats();
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
    app.toggle_stats();
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
    *app.results.get_mut("clippy").unwrap() = ci_tui::runner::CheckResult::cancelled("clippy");
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

/// Color off, graph panel open or closed: header figures, and with the panel
/// its sparkline and MEM gauge (`mark_gauge_fill` path), carry no fg/bg
#[rstest::rstest]
#[case::panel_closed(false)]
#[case::panel_open(true)]
fn test_no_color_renders_no_fg_bg_colors(#[case] panel: bool) {
    let mut app = make_test_app();
    if panel {
        app.toggle_stats();
    }
    app.update_stats(90.0, 8_000_000_000, 16_000_000_000);
    assert!(has_color(&render_at(&mut app, WIDTH, HEIGHT)), "color on");

    app.color = false;
    // Wide enough for the figures beside the running status
    let buffer = render_at(&mut app, 120, HEIGHT);
    assert!(!has_color(&buffer), "every cell fg/bg must be Reset");
    let header = full_row(&buffer, 0);
    assert!(header.contains("CPU 90% · MEM 50%"), "figures: {header}");
    if panel {
        assert!(buffer_contains(&buffer, "┌ CPU 90% "), "CPU panel rendered");
        assert!(buffer_contains(&buffer, "GiB (50%)"), "MEM gauge rendered");
    }
}

/// #291: fresh start (graph panel closed) needs 60x13, with the panel 60x19
#[rstest::rstest]
#[case::narrow(59, 24, false, "need 60x13")]
#[case::short(80, 12, false, "need 60x13")]
#[case::panel_narrow(59, 24, true, "need 60x19")]
#[case::panel_short(60, 18, true, "need 60x19")]
fn test_too_small_terminal_shows_message_only(
    #[case] width: u16,
    #[case] height: u16,
    #[case] panel: bool,
    #[case] need: &str,
) {
    let mut app = make_test_app();
    if panel {
        app.toggle_stats();
    }
    let buffer = render_at(&mut app, width, height);
    assert!(buffer_contains(&buffer, "Terminal too small"));
    assert!(buffer_contains(&buffer, &format!("{width}x{height}")));
    assert!(buffer_contains(&buffer, need));
    assert!(!buffer_contains(&buffer, "CPU") && !buffer_contains(&buffer, "Checks"));
}

/// #291: a fresh start renders the dashboard at 60x13
#[test]
fn test_min_size_terminal_renders_layout() {
    let mut app = make_test_app();
    let buffer = render_at(&mut app, 60, 13);
    assert!(!buffer_contains(&buffer, "too small"));
    assert!(buffer_contains(&buffer, "Checks"));
}

/// #291: with the panel open, 60x19 renders it (CPU block + MEM gauge)
#[test]
fn test_min_size_with_stats_panel_renders_panel() {
    let mut app = make_test_app();
    app.toggle_stats();
    let buffer = render_at(&mut app, 60, 19);
    assert!(!buffer_contains(&buffer, "too small"));
    assert!(row_text(&buffer, 1, 0, 60).contains("┌ CPU 78% "));
    assert!(buffer_contains(&buffer, "MEM") && buffer_contains(&buffer, "GiB"));
    assert!(buffer_contains(&buffer, "Checks"));
}

/// #291: by default no graph panel; main content right under the header,
/// figures only in the header
#[test]
fn test_stats_hidden_renders_no_cpu_mem_panel() {
    let mut app = make_test_app();
    assert!(!app.stats_visible());
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    assert!(!buffer_contains(&buffer, "GiB"), "no MEM gauge");
    assert!((1..HEIGHT).all(|y| !row_text(&buffer, y, 0, WIDTH).contains("CPU")));
    assert!(row_text(&buffer, 1, 0, WIDTH).contains("Checks"));
}

#[rstest::rstest]
#[case::with_stats(19, true)]
#[case::without_stats(13, false)]
fn test_help_overlay_fits_min_size(#[case] height: u16, #[case] stats: bool) {
    let mut app = make_test_app();
    if stats {
        app.toggle_stats();
    }
    app.toggle_help();
    let buffer = render_at(&mut app, 60, height);
    assert!(!buffer_contains(&buffer, "too small"));
    assert!(buffer_contains(&buffer, "Esc to close"));
    assert!(buffer_contains(&buffer, "CPU/MEM graph"));
    assert!(buffer_contains(&buffer, "quit"), "last table row not cut");
    let version = format!("v{}", ci_tui::cli::VERSION_INFO);
    assert!(buffer_contains(&buffer, &version), "version title");
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

// ============================================================================
// allow_failure (#151)
// ============================================================================

/// Mark check `id` of `app` `allow_failure`
fn allow_failure(app: &mut ci_tui::ui::app::App, id: &str) {
    let check = app.checks.iter_mut().find(|c| c.id() == id).unwrap();
    check.definition.allow_failure = true;
}

#[test]
fn test_allowed_failure_icons_are_yellow() {
    let mut app = make_test_app(); // unit failed
    allow_failure(&mut app, "unit");
    allow_failure(&mut app, "clippy");
    app.results.get_mut("clippy").unwrap().status = ci_tui::runner::CheckStatus::TimedOut;
    app.next_check(); // fmt selected: the panel shows no failure icon
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    // #297: allowed failure and allowed timeout both a yellow `⚠`
    let colors = find_all_symbol_colors(buffer, '⚠');
    assert_eq!(colors, [Color::Yellow, Color::Yellow], "unit and clippy");
    assert_eq!(find_symbol_color(buffer, '✗'), None);
    assert_eq!(find_symbol_color(buffer, '⧗'), None);
}

/// #297: a check a failed pre-command blocked is a red `⊘` in the list and
/// `⊘ NOT RUN` in the output panel, not a failure's `✗ FAILED`
#[test]
fn test_not_run_check_shows_red_slashed_circle() {
    let mut app = make_test_app(); // clippy selected
    *app.results.get_mut("clippy").unwrap() =
        ci_tui::runner::CheckResult::blocked("clippy", "Not run: pre-command failed");
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(buffer_contains(buffer, "⊘ NOT RUN"));
    assert!(!buffer_contains(buffer, "FAILED"));
    let colors = find_all_symbol_colors(buffer, '⊘');
    assert_eq!(colors, [Color::Red, Color::Red], "list and panel");
}

#[test]
fn test_output_panel_shows_allowed_failure() {
    let mut app = make_test_app();
    allow_failure(&mut app, "unit");
    // No error lines: the panel would open scrolled to the first one
    app.results.get_mut("unit").unwrap().error_output = "boom\n".to_string();
    app.next_check(); // fmt
    app.next_check(); // test group header
    app.next_check(); // unit
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(buffer_contains(buffer, "⚠ FAILED (allowed)"));
    let colors = find_all_symbol_colors(buffer, '⚠');
    assert_eq!(colors, [Color::Yellow, Color::Yellow], "list and panel");
}

#[test]
fn test_output_panel_shows_allowed_timeout() {
    let mut app = make_test_app(); // clippy selected
    allow_failure(&mut app, "clippy");
    app.results.get_mut("clippy").unwrap().status = ci_tui::runner::CheckStatus::TimedOut;
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    assert!(buffer_contains(
        terminal.backend().buffer(),
        "⚠ TIMED OUT (allowed)"
    ));
}

#[test]
fn test_header_allowed_failure_only_is_yellow_warning() {
    let mut app = make_test_app_all_passed();
    allow_failure(&mut app, "unit");
    app.results.get_mut("unit").unwrap().status = ci_tui::runner::CheckStatus::Failed;
    let mut terminal = create_terminal();
    terminal.draw(|f| dashboard::render(&mut app, f)).unwrap();
    let buffer = terminal.backend().buffer();

    assert!(buffer_contains(buffer, "2/3 passed, 1 warning"));
    assert!(
        !buffer_contains(buffer, "1 warning in "),
        "elapsed shown once"
    );
    assert_eq!(status_cell(buffer, "2/3 passed").bg, Color::Yellow);
}

// ============================================================================
// #292: one-row header
// ============================================================================

/// Finished app with one failed check: status `✗ 2/3 passed, 1 failed`
fn failed_finished_app() -> ci_tui::ui::app::App {
    failed_finished_app_on("feature/test")
}

fn failed_finished_app_on(branch: &str) -> ci_tui::ui::app::App {
    let mut app = make_test_app_on_branch(branch);
    app.run.all_finished = true;
    app.results.values_mut().for_each(|r| {
        r.status = ci_tui::runner::CheckStatus::Passed;
    });
    app.results.get_mut("unit").unwrap().status = ci_tui::runner::CheckStatus::Failed;
    app
}

/// Column (in cells) where `needle` starts in `row` (every char before it is
/// one cell wide in these tests)
fn col_of(row: &str, needle: &str) -> u16 {
    let at = row
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in {row:?}"));
    row[..at].chars().count() as u16
}

/// First cell of `needle` in header row 0
fn status_cell<'a>(buffer: &'a Buffer, needle: &str) -> &'a ratatui::buffer::Cell {
    let row = full_row(buffer, 0);
    &buffer[(col_of(&row, needle), 0)]
}

#[test]
fn test_header_is_one_row_with_stats_panel_below() {
    let mut app = make_test_app();
    app.toggle_stats();
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    let header = full_row(&buffer, 0);
    assert!(header.contains("feature/test"), "{header}");
    assert!(header.contains("2 files vs main"), "{header}");
    assert!(header.contains("Running..."), "{header}");
    assert_eq!(
        buffer[(0, 1)].symbol(),
        "┌",
        "row 1: {}",
        full_row(&buffer, 1)
    );
    assert!(full_row(&buffer, 1).contains("CPU"));
}

#[test]
fn test_header_row_layout_shows_elapsed_once_after_status() {
    let mut app = failed_finished_app();
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    assert_eq!(
        full_row(&buffer, 0),
        " feature/test · 2 files vs main · ✗ 2/3 passed, 1 failed · 0s  CPU 78% · MEM 50%"
    );
}

/// #291: figures right-aligned in the header, panel open or closed
#[rstest::rstest]
#[case::closed(false)]
#[case::open(true)]
fn test_header_shows_cpu_mem_figures_right_aligned(#[case] panel: bool) {
    let mut app = make_test_app();
    if panel {
        app.toggle_stats();
    }
    let header = full_row(&render_at(&mut app, 120, HEIGHT), 0);
    assert!(header.ends_with(" CPU 78% · MEM 50%"), "{header:?}");
    assert!(header.contains("Running..."), "{header:?}");
}

/// #291: before the first sample (or without sysinfo) no figures, rather
/// than a misleading `CPU 0% · MEM 0%`
#[rstest::rstest]
#[case::no_sample(None)]
#[case::zero_total(Some((10.0, 0, 0)))]
fn test_header_shows_no_figures_without_stats_sample(#[case] sample: Option<(f32, u64, u64)>) {
    let passed = statuses(1, ci_tui::runner::CheckStatus::Passed, false);
    let mut app = make_test_app_with_statuses(&passed);
    if let Some((cpu, used, total)) = sample {
        app.update_stats(cpu, used, total);
    }
    let header = full_row(&render_at(&mut app, 120, HEIGHT), 0);
    assert!(
        !header.contains("CPU") && !header.contains("MEM"),
        "{header:?}"
    );
}

#[rstest::rstest]
#[case::all_passed(make_test_app_all_passed(), "✓ All 3 checks passed", Color::Green)]
#[case::failed(failed_finished_app(), "✗ 2/3 passed, 1 failed", Color::Red)]
#[case::running(make_test_app_running(), "Running...", Color::Yellow)]
fn test_header_status_segment_keeps_color(
    #[case] mut app: ci_tui::ui::app::App,
    #[case] status: &str,
    #[case] bg: Color,
) {
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    let row = full_row(&buffer, 0);
    let first = col_of(&row, status);
    for x in first..first + status.chars().count() as u16 {
        assert_eq!(buffer[(x, 0)].bg, bg, "col {x}: {row}");
    }
    // Branch part carries no status background
    assert_ne!(buffer[(1, 0)].bg, bg);
}

#[test]
fn test_header_status_distinguishable_without_color() {
    let mut app = failed_finished_app();
    app.color = false;
    let buffer = render_at(&mut app, WIDTH, HEIGHT);
    let reversed = |x: u16| {
        buffer[(x, 0)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    };
    let row = full_row(&buffer, 0);
    let first = col_of(&row, "✗ 2/3");
    let end = first + "✗ 2/3 passed, 1 failed".chars().count() as u16;
    assert!((first..end).all(reversed), "status reversed: {row}");
    assert!(!(0..first).any(reversed), "branch part plain: {row}");
    assert!(!(end..WIDTH).any(reversed), "elapsed part plain: {row}");
}

/// Parts vanish in order elapsed, CPU/MEM figures (#291), files, branch as
/// the branch grows; the status counts stay whole
#[test]
fn test_header_truncation_order_elapsed_figures_files_branch() {
    // Per branch length: [elapsed, figures, files, branch] shown
    let mut seen = Vec::new();
    for len in 1..70 {
        let mut app = failed_finished_app_on(&"b".repeat(len));
        let row = full_row(&render_at(&mut app, 90, 13), 0);
        assert!(row.contains("✗ 2/3 passed, 1 failed"), "status cut: {row}");
        seen.push([
            row.contains(" · 0s"),
            row.contains("CPU 78% · MEM 50%"),
            row.contains("files vs"),
            row.contains(&"b".repeat(len)),
        ]);
    }
    let shown: Vec<usize> = (0..4)
        .map(|i| seen.iter().filter(|r| r[i]).count())
        .collect();
    assert!(shown.windows(2).all(|w| w[0] < w[1]), "{shown:?} {seen:?}");
    for (len, row) in seen.iter().enumerate() {
        for i in 0..3 {
            assert!(
                !row[i] || row[i + 1],
                "part {i} outlived {} at {}",
                i + 1,
                len + 1
            );
        }
    }
}

#[test]
fn test_header_markers_and_status_survive_at_min_width() {
    let mut app = failed_finished_app_on("feature/a-rather-long-branch-name");
    app.watching = true;
    app.filter_notice = Some("Filtered: --only php-lint".to_string());
    let row = full_row(&render_at(&mut app, 60, 13), 0);
    // Compact status (counts kept) makes room for the whole filter marker
    assert!(row.contains("✗ 2/3, 1 fail"), "{row}");
    assert!(row.contains("watching"), "{row}");
    assert!(row.contains("Filtered: --only"), "{row}");
    assert!(!row.contains(" 0s"), "elapsed dropped first: {row}");
}

/// Min-width (60) row of `app` with `watching` and a `--only` filter
fn min_width_row(mut app: ci_tui::ui::app::App) -> String {
    app.watching = true;
    app.filter_notice = Some("Filtered: --only php-lint".to_string());
    full_row(&render_at(&mut app, 60, 13), 0)
}

fn statuses(
    n: usize,
    status: ci_tui::runner::CheckStatus,
    allow: bool,
) -> Vec<(ci_tui::runner::CheckStatus, bool)> {
    vec![(status, allow); n]
}

/// Review of #359: every count survives at the minimum width, however long
#[test]
fn test_header_mixed_counts_all_survive_at_min_width() {
    use ci_tui::runner::CheckStatus::*;
    let mut all = statuses(100, Passed, false);
    all.extend(statuses(100, Failed, false));
    all.extend(statuses(100, Failed, true));
    all.extend(statuses(100, Cancelled, false));
    all.extend(statuses(100, Pending, false));
    let row = min_width_row(make_test_app_with_statuses(&all));
    for token in ["100/500", "100 fail", "100 warn", "100 canc", "100 pend"] {
        assert!(row.contains(token), "{token:?} missing: {row}");
    }
    assert!(!row.contains('…'), "status clipped: {row}");
}

#[test]
fn test_header_retry_pending_after_finish_keeps_filter_and_counts() {
    use ci_tui::runner::CheckStatus::*;
    let mut all = statuses(3, Passed, false);
    all.extend(statuses(1, Failed, false));
    all.extend(statuses(2, Pending, false));
    let row = min_width_row(make_test_app_with_statuses(&all));
    for token in ["3/6", "1 fail", "2 pend", "watching", "Filtered: --only"] {
        assert!(row.contains(token), "{token:?} missing: {row}");
    }
}

#[test]
fn test_header_on_demand_count_survives_at_min_width() {
    use ci_tui::runner::CheckStatus::*;
    let mut all = statuses(2, Passed, false);
    all.extend(statuses(2, OnDemand, false));
    let row = min_width_row(make_test_app_with_statuses(&all));
    for token in ["All 2", "+2", "Filtered: --only"] {
        assert!(row.contains(token), "{token:?} missing: {row}");
    }
}

/// Review of #359: the reported two-check running state kept no filter
#[test]
fn test_header_running_state_keeps_filter_at_min_width() {
    let mut app = make_test_app();
    app.results.get_mut("fmt").unwrap().status = ci_tui::runner::CheckStatus::OnDemand;
    app.results.get_mut("unit").unwrap().status = ci_tui::runner::CheckStatus::OnDemand;
    app.results.get_mut("clippy").unwrap().status = ci_tui::runner::CheckStatus::Running;
    let row = min_width_row(app);
    for token in ["0/1", "+2", "Filtered: --only"] {
        assert!(row.contains(token), "{token:?} missing: {row}");
    }
}

/// Review of #359: a joined emoji is cut as one unit, by the same width
/// the row is measured with
#[test]
fn test_header_filter_cut_keeps_zwj_emoji_whole() {
    use ci_tui::runner::CheckStatus::*;
    let mut all = statuses(10, Passed, false);
    all.extend(statuses(10, Failed, false));
    let mut app = make_test_app_with_statuses(&all);
    app.watching = true;
    app.filter_notice = Some("Filtered: --only abcdefghi👩‍💻-long-check-name".to_string());
    let row = full_row(&render_at(&mut app, 60, 13), 0);
    assert!(row.contains("✗ 10/20, 10 fail"), "{row}");
    assert!(row.contains("watching"), "{row}");
    // (the cell after a wide char reads as a space in the buffer)
    assert!(row.contains("Filtered: --only abcdefghi👩‍💻"), "{row}");
    assert!(
        row.trim_end().ends_with('…') && !row.contains("-long"),
        "{row}"
    );
}

#[test]
fn test_header_wide_branch_name_keeps_status_and_watching() {
    let mut app = failed_finished_app_on(&"ブランチ".repeat(12));
    app.watching = true;
    let row = full_row(&render_at(&mut app, 60, 13), 0);
    assert!(row.contains("✗ 2/3 passed, 1 failed"), "{row}");
    assert!(row.contains("watching"), "{row}");
}

/// The panels start right under the 1-row header: with stats the main
/// area begins at row 7, without (default) at row 1
#[test]
fn test_hit_test_areas_start_two_rows_higher() {
    let mut app = make_test_app();
    render_at(&mut app, WIDTH, HEIGHT);
    assert!(app.is_over_output(70, 2) && !app.is_over_output(70, 0));
    app.toggle_stats();
    render_at(&mut app, WIDTH, HEIGHT);
    assert!(
        app.is_over_output(70, 8) && !app.is_over_output(70, 6),
        "{:?}",
        output_rows(&app)
    );
}
