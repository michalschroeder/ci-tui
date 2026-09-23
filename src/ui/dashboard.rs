//! Rendering logic for the TUI dashboard using ratatui widgets.
//!
//! This module contains all the rendering functions that draw the UI
//! components: header with progress, system stats sparklines, check list,
//! file list, output panel, and footer with keyboard shortcuts.
//!
//! # Layout
//!
//! The UI is divided into four horizontal sections:
//! 1. **Header**: Progress gauge with branch info and elapsed time
//! 2. **System stats**: CPU and memory sparklines
//! 3. **Main content**: Split into checks list, files list, and output panel
//! 4. **Footer**: Keyboard shortcuts and version info

use super::app::{App, PreCommandState, SelectableItem, StatusKind};
use crate::checks::CheckFiles;
use crate::runner::{CheckResult, CheckStatus};
use crate::utils::time;
use ansi_to_tui::IntoText;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Sparkline, Wrap},
    Frame,
};
use std::collections::VecDeque;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const GIT_HASH: &str = env!("CI_TUI_GIT_HASH");
const BUILD_DATE: &str = env!("CI_TUI_BUILD_DATE");
/// Minimum height (rows, incl. borders) of the checks panel
const CHECKS_PANEL_MIN_HEIGHT: u16 = 8;
/// Checks panel takes at most this fraction of the main-area height
const CHECKS_PANEL_MAX_HEIGHT_RATIO: f32 = 0.7;
/// Rows consumed by a bordered block (top + bottom border)
const PANEL_BORDER_ROWS: usize = 2;
/// Columns consumed by a bordered block (left + right border)
const PANEL_BORDER_COLS: u16 = 2;
/// Columns before a check name: border, space, icon, space
const CHECK_NAME_LEAD_COLS: u16 = 4;
/// Columns reserved beside a check name (icon, padding, duration)
const CHECK_NAME_RESERVED_COLS: u16 = 15;
/// Minimum columns granted to a check name before truncation
const CHECK_NAME_MIN_COLS: usize = 20;
/// Max files listed inline in the output header before truncating
const FILES_PREVIEW_COUNT: usize = 3;
/// Max stderr lines shown per failed fix in fix-all results
const FIX_ERROR_PREVIEW_LINES: usize = 5;

/// Prepare sparkline data from history, filling width with oldest data on left
fn prepare_sparkline_data(history: &VecDeque<f32>, width: usize) -> Vec<u64> {
    let history_len = history.len();
    if history_len >= width {
        history
            .iter()
            .skip(history_len - width)
            .map(|&v| v.max(1.0) as u64)
            .collect()
    } else {
        let mut data = vec![1u64; width - history_len];
        data.extend(history.iter().map(|&v| v.max(1.0) as u64));
        data
    }
}

/// Get color based on usage percentage thresholds
fn get_usage_color(usage: f32) -> Color {
    match usage {
        x if x > 80.0 => Color::Red,
        x if x > 50.0 => Color::Yellow,
        _ => Color::Green,
    }
}

/// Get status icon and style for a check status
fn get_status_display(status: Option<&CheckStatus>) -> (&'static str, Style) {
    match status {
        Some(CheckStatus::Passed) => ("✓", Style::default().fg(Color::Green)),
        Some(CheckStatus::Failed) => ("✗", Style::default().fg(Color::Red)),
        Some(CheckStatus::TimedOut) => ("⧗", Style::default().fg(Color::Magenta)),
        Some(CheckStatus::Cancelled) => ("⊗", Style::default().fg(Color::Gray)),
        Some(CheckStatus::Running) => ("●", Style::default().fg(Color::Yellow)),
        Some(CheckStatus::Pending) => ("○", Style::default().fg(Color::DarkGray)),
        Some(CheckStatus::Skipped) => ("⊘", Style::default().fg(Color::DarkGray)),
        Some(CheckStatus::OnDemand) => ("◇", Style::default().fg(Color::Cyan)),
        None => ("?", Style::default().fg(Color::DarkGray)),
    }
}

/// Header label once all checks finished (without the on-demand suffix)
pub(crate) fn finished_status_text(
    counts: &super::app::StatusCounts,
    total: usize,
    elapsed: &str,
) -> String {
    let (passed, failed, cancelled) = (counts.passed, counts.failed, counts.cancelled);
    if failed == 0 && cancelled == 0 {
        return format!("✓ All {} checks passed in {}", total, elapsed);
    }
    let mark = if failed > 0 { "✗ " } else { "" };
    let cancelled_text = if cancelled > 0 {
        format!(", {} cancelled", cancelled)
    } else {
        String::new()
    };
    format!(
        "{}{}/{} passed, {} failed{} in {}",
        mark, passed, total, failed, cancelled_text, elapsed
    )
}

/// Longest prefix of `s` at most `max` terminal columns wide (wide CJK/emoji
/// chars count as 2; never splits a UTF-8 char)
fn prefix_width(s: &str, max: usize) -> &str {
    let mut width = 0;
    for (i, c) in s.char_indices() {
        width += c.width().unwrap_or(0);
        if width > max {
            return &s[..i];
        }
    }
    s
}

/// Longest suffix of `s` at most `max` terminal columns wide
fn suffix_width(s: &str, max: usize) -> &str {
    let mut width = 0;
    for (i, c) in s.char_indices().rev() {
        width += c.width().unwrap_or(0);
        if width > max {
            return &s[i + c.len_utf8()..];
        }
    }
    s
}

pub fn render(app: &mut App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header with progress
            Constraint::Length(6), // System stats (taller for better graphs)
            Constraint::Min(10),   // Main content (checks + output)
            Constraint::Length(2), // Footer/help
        ])
        .split(frame.area());

    render_header(app, frame, chunks[0]);
    render_system_stats(app, frame, chunks[1]);
    render_main(app, frame, chunks[2]);
    render_footer(app, frame, chunks[3]);
}

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let counts = app.count_by_status();
    let on_demand = counts.on_demand;
    let total = app.checks.len();
    let auto_run_total = total - on_demand;
    let completed = counts.completed();
    let ratio = if auto_run_total > 0 {
        completed as f64 / auto_run_total as f64
    } else {
        1.0 // All checks are on-demand, show as complete
    };

    // Format elapsed time
    let elapsed = app.elapsed_time();
    let elapsed_str = format_elapsed(elapsed);

    let branch_info = format!(
        " {} │ {} files vs {} │ {} ",
        app.current_branch,
        app.changed_files.len(),
        app.changed_files.base_ref,
        elapsed_str
    );

    let on_demand_text = if on_demand > 0 {
        format!(" +{} on-demand", on_demand)
    } else {
        String::new()
    };
    let status_text = if app.run.all_finished {
        finished_status_text(&counts, auto_run_total, &elapsed_str) + &on_demand_text
    } else if counts.pending > 0 {
        format!(
            "Running... {}/{} ({} in progress){}",
            completed, auto_run_total, counts.pending, on_demand_text
        )
    } else {
        format!(
            "Preparing... {}/{}{}",
            completed, auto_run_total, on_demand_text
        )
    };

    let color = if app.run.all_finished {
        if counts.failed > 0 {
            Color::Red
        } else if counts.cancelled > 0 {
            Color::Yellow
        } else {
            Color::Green
        }
    } else {
        Color::Yellow
    };

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(branch_info))
        .gauge_style(Style::default().fg(color).bg(Color::DarkGray))
        .ratio(ratio)
        .label(status_text);

    frame.render_widget(gauge, area);
}

fn render_system_stats(app: &App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Create blocks first to calculate inner width
    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" CPU {:.0}% ", app.cpu_usage()));
    let mem_block = Block::default().borders(Borders::ALL).title(format!(
        " MEM {:.1}/{:.1}GiB ({:.0}%) ",
        app.mem_used_gib(),
        app.mem_total_gib(),
        app.mem_usage()
    ));

    // Get actual inner width (after borders)
    let cpu_inner = cpu_block.inner(chunks[0]);
    let mem_inner = mem_block.inner(chunks[1]);

    // CPU sparkline
    let cpu_data = prepare_sparkline_data(&app.sys.cpu_history, cpu_inner.width as usize);
    let cpu_sparkline = Sparkline::default()
        .block(cpu_block)
        .data(&cpu_data)
        .max(100)
        .style(Style::default().fg(get_usage_color(app.cpu_usage())))
        .bar_set(symbols::bar::NINE_LEVELS);
    frame.render_widget(cpu_sparkline, chunks[0]);

    // Memory sparkline
    let mem_data = prepare_sparkline_data(&app.sys.mem_history, mem_inner.width as usize);
    let mem_sparkline = Sparkline::default()
        .block(mem_block)
        .data(&mem_data)
        .max(100)
        .style(Style::default().fg(get_usage_color(app.mem_usage())))
        .bar_set(symbols::bar::NINE_LEVELS);
    frame.render_widget(mem_sparkline, chunks[1]);
}

fn render_main(app: &mut App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40), // Wider left panel
            Constraint::Percentage(60),
        ])
        .split(area);

    // Calculate dynamic height for checks based on content
    // Count total items: groups + pre-commands + checks
    let groups = app.groups();
    let total_check_items = groups.len() + app.pre_commands.len() + app.checks.len();
    // borders + min height, capped at a fraction of available height
    let checks_height = ((total_check_items + PANEL_BORDER_ROWS) as u16)
        .max(CHECKS_PANEL_MIN_HEIGHT)
        .min((area.height as f32 * CHECKS_PANEL_MAX_HEIGHT_RATIO) as u16);

    // Left side: checks + files with dynamic height
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(checks_height), // Dynamic checks height
            Constraint::Min(5),                // Files get remaining space
        ])
        .split(chunks[0]);

    render_checks_list(app, frame, left_chunks[0]);
    render_files_list(app, frame, left_chunks[1]);
    render_output(app, frame, chunks[1]);
}

/// Render a pre-command as a list item
fn render_pre_command_item(pre_cmd: &PreCommandState, is_selected: bool) -> ListItem<'static> {
    use super::app::PreCommandStatus;

    let (icon, icon_style) = match pre_cmd.status {
        PreCommandStatus::Pending => ("◦", Style::default().fg(Color::DarkGray)),
        PreCommandStatus::Running => ("⚡", Style::default().fg(Color::Yellow)),
        PreCommandStatus::Passed => ("✓", Style::default().fg(Color::Green)),
        PreCommandStatus::Failed => ("✗", Style::default().fg(Color::Red)),
    };

    let duration = if pre_cmd.duration_ms > 0 {
        format!(" {}", time::format(pre_cmd.duration_ms))
    } else {
        String::new()
    };

    let name_style = if is_selected {
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::ITALIC | Modifier::REVERSED)
    } else {
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::ITALIC)
    };

    ListItem::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(icon, icon_style),
        Span::raw(" "),
        Span::styled(pre_cmd.name.clone(), name_style),
        Span::styled(duration, Style::default().fg(Color::DarkGray)),
    ]))
}

/// Render a check as a list item
fn render_check_item(
    app: &App,
    check: &crate::checks::CheckToRun,
    area: Rect,
    is_selected: bool,
) -> ListItem<'static> {
    let result = app.results.get(check.id());
    let (icon, icon_style) = get_status_display(result.map(|r| &r.status));

    let duration = result
        .map(|r| {
            if r.duration_ms > 0 {
                format!(" {}", time::format(r.duration_ms))
            } else {
                String::new()
            }
        })
        .unwrap_or_default();

    let is_on_demand = result
        .map(|r| r.status == CheckStatus::OnDemand)
        .unwrap_or(false);

    // Fade out checks that are not relevant in current context
    let should_fade = is_on_demand;

    let name_style = if is_selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else if should_fade {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default()
    };

    // Truncate name if needed - calculate based on available width. The
    // minimum never exceeds what fits in the panel.
    let available_width = area.width.saturating_sub(CHECK_NAME_RESERVED_COLS) as usize;
    let fit_width = area
        .width
        .saturating_sub(PANEL_BORDER_COLS + CHECK_NAME_LEAD_COLS) as usize;
    let max_name_len = available_width.max(CHECK_NAME_MIN_COLS.min(fit_width));
    let name = if check.name().width() > max_name_len {
        format!(
            "{}…",
            prefix_width(check.name(), max_name_len.saturating_sub(1))
        )
    } else {
        check.name().to_string()
    };

    let final_icon_style = if should_fade && !is_selected {
        icon_style.add_modifier(Modifier::DIM)
    } else {
        icon_style
    };

    ListItem::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(icon, final_icon_style),
        Span::raw(" "),
        Span::styled(name, name_style),
        Span::styled(duration, Style::default().fg(Color::DarkGray)),
    ]))
}

fn render_checks_list(app: &mut App, frame: &mut Frame, area: Rect) {
    let (items, selected_row) = build_checks_list_items(app, area);

    let filter_info = match app.view.status_filter {
        super::app::StatusFilter::All => "",
        super::app::StatusFilter::Failed => " [failed]",
    };

    // scroll_padding keeps the neighbor rows (e.g. group headers) in view
    let list = List::new(items).scroll_padding(1).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Checks{} ", filter_info)),
    );

    // Persist the scroll offset so the list scrolls only when the selection
    // leaves the visible window
    let mut state = ListState::default()
        .with_offset(app.view.checks_list_offset)
        .with_selected(selected_row);
    frame.render_stateful_widget(list, area, &mut state);
    app.view.checks_list_offset = state.offset();
}

/// Build the checks list rows and the row index of the selected item.
///
/// Rows come from [`App::selectable_items`] (plus a header whenever the group
/// changes), so the list and the selection can never disagree.
fn build_checks_list_items(app: &App, area: Rect) -> (Vec<ListItem<'static>>, Option<usize>) {
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_row = None;
    let mut current_group = None;

    for (idx, item) in app.selectable_items().enumerate() {
        let group = match item {
            SelectableItem::PreCommand(pc) => pc.group.as_str(),
            SelectableItem::Check(check) => check.group(),
        };
        if current_group != Some(group) {
            current_group = Some(group);
            items.push(group_header_item(app, group));
        }

        let is_selected = idx == app.view.selected_check;
        if is_selected {
            selected_row = Some(items.len());
        }
        items.push(match item {
            SelectableItem::PreCommand(pc) => render_pre_command_item(pc, is_selected),
            SelectableItem::Check(check) => render_check_item(app, check, area, is_selected),
        });
    }

    (items, selected_row)
}

/// Group header row; highlighted while the group runs
fn group_header_item(app: &App, group: &str) -> ListItem<'static> {
    let color = if app.run.current_group.as_deref() == Some(group) {
        Color::Yellow
    } else {
        Color::Cyan
    };
    let group_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    let display_name = app.group_display_name(group);
    ListItem::new(Line::from(vec![
        Span::styled(format!("─ {} ", display_name.to_uppercase()), group_style),
        Span::styled("───────────", Style::default().fg(Color::DarkGray)),
    ]))
}

fn render_files_list(app: &App, frame: &mut Frame, area: Rect) {
    // Calculate available width (subtract 2 for borders, 1 for padding)
    let available_width = area.width.saturating_sub(3) as usize;

    let items: Vec<ListItem> = app
        .changed_files
        .files
        .iter()
        .map(|f| {
            // Color by file type from config
            let color_name = app.config.get_file_color(f);
            let color = color_from_name(color_name);
            let style = Style::default().fg(color);

            // Smart path truncation: keep filename visible, truncate directory path
            let display_name = truncate_path(f, available_width);

            ListItem::new(Span::styled(display_name, style))
        })
        .collect();

    // Build legend from extensions actually present in changed files
    let legend_line = build_file_color_legend(app);

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Files ({}) ", app.changed_files.len()))
            .title_bottom(legend_line),
    );

    frame.render_widget(list, area);
}

/// Build color legend line for file extensions present in changed files
fn build_file_color_legend(app: &App) -> Line<'static> {
    use std::collections::BTreeSet;

    // Collect unique extensions from changed files
    let extensions: BTreeSet<String> = app
        .changed_files
        .files
        .iter()
        .filter_map(|f| f.rsplit('.').next().map(|ext| ext.to_lowercase()))
        .filter(|ext| !ext.contains('/')) // Skip files without extensions
        .collect();

    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];

    for ext in extensions {
        // Create a fake path with the extension to get its color
        let fake_path = format!("file.{}", ext);
        let color_name = app.config.get_file_color(&fake_path);
        let color = color_from_name(color_name);
        spans.push(Span::styled(
            format!(".{}", ext),
            Style::default().fg(color),
        ));
        spans.push(Span::raw(" "));
    }

    // If no colored extensions found, return empty line
    if spans.len() <= 1 {
        return Line::from("");
    }

    Line::from(spans)
}

/// Convert color name from config to ratatui Color
fn color_from_name(name: &str) -> Color {
    match name.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::DarkGray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        _ => Color::White,
    }
}

/// Format elapsed duration in human-readable format
pub fn format_elapsed(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        let mins = secs / 60;
        let remaining_secs = secs % 60;
        format!("{}m {}s", mins, remaining_secs)
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{}h {}m", hours, mins)
    }
}

/// Smart path truncation that keeps the filename visible and truncates directory path
fn truncate_path(path: &str, max_width: usize) -> String {
    if path.width() <= max_width {
        return path.to_string();
    }

    // Find the last path separator to split filename from directory
    if let Some(last_sep) = path.rfind('/') {
        let filename = &path[last_sep + 1..];
        let dir_path = &path[..last_sep];
        let filename_len = filename.width();

        // If filename alone is too long, truncate it
        if filename_len >= max_width {
            return format!("…{}", suffix_width(filename, max_width.saturating_sub(1)));
        }

        // Calculate space for directory (max_width - filename - "/" - "…")
        let dir_space = max_width.saturating_sub(filename_len + 2);

        if dir_space < 3 {
            // Not enough space for directory, just show filename
            return filename.to_string();
        }

        // Truncate directory from the left, keeping the rightmost part
        let truncated_dir = if dir_path.width() > dir_space {
            format!("…{}", suffix_width(dir_path, dir_space - 1))
        } else {
            dir_path.to_string()
        };

        format!("{}/{}", truncated_dir, filename)
    } else {
        // No separator, just truncate from the left
        format!("…{}", suffix_width(path, max_width.saturating_sub(1)))
    }
}

/// What the output panel shows. Shared by rendering and scroll clamping so
/// the max scroll offset always matches the drawn text.
enum OutputView<'a> {
    FixAllResults,
    FixAllRunning,
    FixResult(&'a CheckResult),
    FixRunning,
    PreCommand(&'a PreCommandState),
    Check(Option<&'a crate::checks::CheckToRun>),
}

fn output_view(app: &App) -> OutputView<'_> {
    if !app.fix.all_results.is_empty() && !app.fix.all_running {
        OutputView::FixAllResults
    } else if app.fix.all_running {
        OutputView::FixAllRunning
    } else if let Some(result) = app.fix.result.as_ref() {
        OutputView::FixResult(result)
    } else if app.fix.running {
        OutputView::FixRunning
    } else {
        match app.selected_item() {
            Some(SelectableItem::PreCommand(pc)) => OutputView::PreCommand(pc),
            Some(SelectableItem::Check(check)) => OutputView::Check(Some(check)),
            None => OutputView::Check(None),
        }
    }
}

/// Count the screen rows the output panel currently draws (after wrapping
/// at the panel's inner width).
///
/// Used by [`App`] scroll clamping. Running panels do not scroll (0).
pub fn output_line_count(app: &App, width: u16) -> usize {
    let text = match output_view(app) {
        OutputView::FixAllResults => fix_all_results_text(app),
        OutputView::FixResult(result) => fix_result_text(result),
        OutputView::PreCommand(pc) => pre_command_output_text(pc),
        OutputView::Check(Some(check)) => match app.results.get(check.id()) {
            Some(result) => build_check_output_text(app, check, result, width),
            None => return 0,
        },
        OutputView::FixAllRunning | OutputView::FixRunning | OutputView::Check(None) => return 0,
    };
    output_paragraph(&text).line_count(width.saturating_sub(PANEL_BORDER_COLS))
}

/// Wrapped paragraph of ANSI output text (no block)
fn output_paragraph(raw_output: &str) -> Paragraph<'static> {
    Paragraph::new(raw_output.into_text().unwrap_or_default()).wrap(Wrap { trim: false })
}

/// Render scrollable ANSI output. The title gets `[row/total]` when the
/// wrapped text overflows the panel.
fn render_scrollable(app: &App, frame: &mut Frame, area: Rect, raw_output: &str, title: &str) {
    let paragraph = output_paragraph(raw_output);
    let total_lines = paragraph.line_count(area.width.saturating_sub(PANEL_BORDER_COLS));
    let visible_lines = app.view.output_visible_lines;
    let title = if total_lines > visible_lines && visible_lines > 0 {
        let current_line = app.view.output_scroll + 1;
        format!(" {} [{}/{}] ", title, current_line, total_lines)
    } else {
        format!(" {} ", title)
    };
    // ratatui scrolls by u16; larger offsets saturate instead of wrapping
    let scroll = u16::try_from(app.view.output_scroll).unwrap_or(u16::MAX);
    let paragraph = paragraph
        .block(Block::default().borders(Borders::ALL).title(title))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

/// Build the fix-all results text
fn fix_all_results_text(app: &App) -> String {
    let mut raw_output = String::with_capacity(512);

    let passed = app
        .fix
        .all_results
        .iter()
        .filter(|r| r.status == CheckStatus::Passed)
        .count();
    let failed = app
        .fix
        .all_results
        .iter()
        .filter(|r| r.status == CheckStatus::Failed)
        .count();

    if failed == 0 {
        raw_output.push_str(&format!(
            "\x1b[32m✓ All {} fixes completed successfully!\x1b[0m\n\n",
            passed
        ));
    } else {
        raw_output.push_str(&format!(
            "\x1b[33m● {}/{} fixes completed, {} failed\x1b[0m\n\n",
            passed,
            app.fix.all_results.len(),
            failed
        ));
    }

    for result in &app.fix.all_results {
        let status_icon = if result.status == CheckStatus::Passed {
            "\x1b[32m✓\x1b[0m"
        } else {
            "\x1b[31m✗\x1b[0m"
        };
        raw_output.push_str(&format!("{} {}\n", status_icon, result.check_id));

        let show_errors = result.status == CheckStatus::Failed && !result.error_output.is_empty();
        if show_errors {
            let error_lines: String = result
                .error_output
                .lines()
                .take(FIX_ERROR_PREVIEW_LINES)
                .map(|line| format!("  \x1b[31m{}\x1b[0m\n", line))
                .collect();
            raw_output.push_str(&error_lines);
        }
    }

    raw_output
}

/// Render fix-all results (completed)
fn render_fix_all_results(app: &App, frame: &mut Frame, area: Rect) {
    render_scrollable(
        app,
        frame,
        area,
        &fix_all_results_text(app),
        "Fix All Results",
    );
}

/// Render fix-all running status
fn render_fix_all_running(app: &App, frame: &mut Frame, area: Rect) {
    let progress = format!("{}/{}", app.fix.all_results.len(), app.fix.all_total);
    let mut raw_output = format!("\x1b[33m● Running fix all... {}\x1b[0m\n\n", progress);

    for result in &app.fix.all_results {
        let status_icon = if result.status == CheckStatus::Passed {
            "\x1b[32m✓\x1b[0m"
        } else {
            "\x1b[31m✗\x1b[0m"
        };
        raw_output.push_str(&format!("{} {}\n", status_icon, result.check_id));
    }
    raw_output.push_str("\n\x1b[90mPlease wait...\x1b[0m");

    let paragraph = Paragraph::new(raw_output.into_text().unwrap_or_default())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Running Fix All "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Build the single fix result text
fn fix_result_text(fix_result: &CheckResult) -> String {
    let mut raw_output = String::with_capacity(512);

    let status_line = if fix_result.status == CheckStatus::Passed {
        "\x1b[32m✓ Fix completed successfully!\x1b[0m\n\n"
    } else {
        "\x1b[31m✗ Fix failed\x1b[0m\n\n"
    };
    raw_output.push_str(status_line);
    raw_output.push_str(&fix_result.output);

    if fix_result.status == CheckStatus::Failed && !fix_result.error_output.is_empty() {
        raw_output.push_str("\n\x1b[31m─── STDERR ───\x1b[0m\n");
        raw_output.push_str(fix_result.error_output.trim_end());
    }

    raw_output
}

/// Render single fix result
fn render_fix_result(app: &App, fix_result: &CheckResult, frame: &mut Frame, area: Rect) {
    render_scrollable(app, frame, area, &fix_result_text(fix_result), "Fix Result");
}

/// Render fix running status
fn render_fix_running(frame: &mut Frame, area: Rect) {
    let paragraph = Paragraph::new(
        "\x1b[33m● Running fix command...\x1b[0m\n\nPlease wait..."
            .into_text()
            .unwrap_or_default(),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Running Fix "),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Build the output string for a check with results
fn build_check_output_text(
    app: &App,
    check: &crate::checks::CheckToRun,
    result: &crate::runner::CheckResult,
    width: u16,
) -> String {
    let mut raw_output = String::with_capacity(1024);

    // Command at the top - truncated by default, full on 'e' toggle
    let command = &check.resolved_command;
    let max_cmd_len = (width as usize).saturating_sub(10);

    raw_output.push_str("\x1b[90m$\x1b[0m \x1b[93m");
    if app.view.show_full_command || command.width() <= max_cmd_len {
        raw_output.push_str(command);
    } else {
        raw_output.push_str(prefix_width(command, max_cmd_len.saturating_sub(3)));
        raw_output.push_str("...\x1b[0m \x1b[90m[e=expand]");
    }
    raw_output.push_str("\x1b[0m\n\n");

    // Status header
    let status_line = match result.status {
        CheckStatus::Passed => "\x1b[32m✓ PASSED\x1b[0m",
        CheckStatus::Failed => "\x1b[31m✗ FAILED\x1b[0m",
        CheckStatus::TimedOut => "\x1b[35m⧗ TIMED OUT\x1b[0m",
        CheckStatus::Cancelled => "\x1b[37m⊗ CANCELLED\x1b[0m",
        CheckStatus::Running => "\x1b[33m◉ RUNNING...\x1b[0m",
        CheckStatus::Pending => "\x1b[90m○ PENDING\x1b[0m",
        CheckStatus::Skipped => "\x1b[90m⊘ SKIPPED\x1b[0m",
        CheckStatus::OnDemand => "\x1b[36m◇ ON-DEMAND\x1b[0m",
    };
    raw_output.push_str(status_line);

    // Show hints
    if result.status == CheckStatus::OnDemand {
        raw_output.push_str("  \x1b[33m← press 't' to run\x1b[0m");
    } else if result.status.is_failure() && check.has_fix() {
        raw_output.push_str("  \x1b[33m← press 'x' to fix\x1b[0m");
    } else if result.status == CheckStatus::Cancelled {
        raw_output.push_str("  \x1b[33m← press 'r' to retry\x1b[0m");
    } else if result.status == CheckStatus::Running {
        raw_output.push_str("  \x1b[33m← press 's' to cancel\x1b[0m");
    }
    raw_output.push_str("\n\n");

    // Files
    append_files_section(&mut raw_output, app, check);

    // Command output
    if !result.output.is_empty() {
        raw_output.push_str(&result.output);
    }

    // Stderr for failed checks
    if result.status.is_failure() && !result.error_output.is_empty() {
        raw_output.push_str("\n\x1b[31m── stderr ──\x1b[0m\n");
        raw_output.push_str(result.error_output.trim_end());
    }

    if result.status == CheckStatus::Pending {
        raw_output.push_str("\x1b[90mWaiting to run...\x1b[0m");
    }

    raw_output
}

/// Append the files section to the output string
fn append_files_section(raw_output: &mut String, app: &App, check: &crate::checks::CheckToRun) {
    let Some(CheckFiles::Files(files)) = app
        .checks
        .iter()
        .find(|c| c.id() == check.id())
        .map(|c| &c.files)
    else {
        return;
    };

    if files.is_empty() {
        return;
    }

    if app.view.show_full_command {
        raw_output.push_str("\x1b[90mFiles:\x1b[0m\n");
        for file in files {
            raw_output.push_str(&format!("\x1b[90m  - {}\x1b[0m\n", file));
        }
        raw_output.push('\n');
    } else {
        let display_files = if files.len() <= FILES_PREVIEW_COUNT {
            files.join(", ")
        } else {
            files
                .iter()
                .take(FILES_PREVIEW_COUNT)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        };
        raw_output.push_str(&format!("\x1b[90mFiles: {}\x1b[0m\n\n", display_files));
    }
}

/// Render check output details
fn render_check_output(
    app: &App,
    check: Option<&crate::checks::CheckToRun>,
    frame: &mut Frame,
    area: Rect,
) {
    let Some(check) = check else {
        let block = Block::default().borders(Borders::ALL).title(" Output ");
        let paragraph = Paragraph::new("Select a check to view details")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    };

    let Some(result) = app.results.get(check.id()) else {
        let title = format!(" {} ", check.name());
        let block = Block::default().borders(Borders::ALL).title(title);
        let paragraph = Paragraph::new("Select a check to view details")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    };

    let raw_output = build_check_output_text(app, check, result, area.width);
    render_scrollable(app, frame, area, &raw_output, check.name());
}

/// Build the pre-command output text
fn pre_command_output_text(pre_cmd: &PreCommandState) -> String {
    let mut raw_output = String::with_capacity(512);

    // Command info
    raw_output.push_str(&format!(
        "\x1b[90mPre-command for group:\x1b[0m \x1b[93m{}\x1b[0m\n\n",
        pre_cmd.group
    ));

    // Status header
    let status_line = match pre_cmd.status {
        super::app::PreCommandStatus::Passed => "\x1b[32m✓ PASSED\x1b[0m",
        super::app::PreCommandStatus::Failed => "\x1b[31m✗ FAILED\x1b[0m",
        super::app::PreCommandStatus::Running => "\x1b[33m⚡ RUNNING...\x1b[0m",
        super::app::PreCommandStatus::Pending => "\x1b[90m◦ PENDING\x1b[0m",
    };
    raw_output.push_str(status_line);

    // Duration
    if pre_cmd.duration_ms > 0 {
        raw_output.push_str(&format!(
            " \x1b[90m({})\x1b[0m",
            time::format(pre_cmd.duration_ms)
        ));
    }
    raw_output.push_str("\n\n");

    // Output
    if !pre_cmd.output.is_empty() {
        raw_output.push_str(&pre_cmd.output);
    } else if pre_cmd.status == super::app::PreCommandStatus::Pending {
        raw_output.push_str("\x1b[90mWaiting to run...\x1b[0m");
    }

    raw_output
}

/// Render pre-command output details
fn render_pre_command_output(app: &App, pre_cmd: &PreCommandState, frame: &mut Frame, area: Rect) {
    let raw_output = pre_command_output_text(pre_cmd);
    render_scrollable(app, frame, area, &raw_output, &pre_cmd.name);
}

fn render_output(app: &mut App, frame: &mut Frame, area: Rect) {
    // Update the visible lines for scroll calculations (subtract 2 for borders)
    app.set_output_visible_lines(area.height.saturating_sub(2) as usize);
    app.output_area_width = area.width;
    app.clamp_output_scroll();

    // Dispatch to appropriate sub-renderer based on state
    match output_view(app) {
        OutputView::FixAllResults => render_fix_all_results(app, frame, area),
        OutputView::FixAllRunning => render_fix_all_running(app, frame, area),
        OutputView::FixResult(result) => render_fix_result(app, result, frame, area),
        OutputView::FixRunning => render_fix_running(frame, area),
        OutputView::PreCommand(pc) => render_pre_command_output(app, pc, frame, area),
        OutputView::Check(check) => render_check_output(app, check, frame, area),
    }
}

/// Build keyboard shortcut spans for the footer
fn build_footer_shortcuts(app: &App) -> Vec<Span<'static>> {
    let caps = app.selected_capabilities();
    let expand_label = if app.view.show_full_command {
        "collapse"
    } else {
        "expand"
    };

    let filter_spans = if app.view.status_filter == crate::ui::app::StatusFilter::Failed {
        vec![
            Span::styled("a", Style::default().fg(Color::Yellow)),
            Span::raw(" all  "),
        ]
    } else {
        vec![
            Span::styled("f", Style::default().fg(Color::Yellow)),
            Span::raw(" failed  "),
        ]
    };

    let mut spans = vec![
        Span::styled(" q", Style::default().fg(Color::Yellow)),
        Span::raw(" quit  "),
        Span::styled("↑↓", Style::default().fg(Color::Yellow)),
        Span::raw(" select  "),
    ];
    spans.extend(filter_spans);
    spans.extend(vec![
        Span::styled("c", Style::default().fg(Color::Yellow)),
        Span::raw(" copy  "),
        Span::styled("e", Style::default().fg(Color::Yellow)),
        Span::raw(format!(" {}  ", expand_label)),
    ]);

    if caps.can_trigger {
        spans.push(Span::styled(
            "t",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" run test", Style::default().fg(Color::Cyan)));
        spans.push(Span::raw("  "));
    }

    if caps.can_cancel {
        spans.push(Span::styled("s", Style::default().fg(Color::Red)));
        spans.push(Span::raw(" cancel  "));
    }

    if caps.can_retry {
        spans.push(Span::styled("r", Style::default().fg(Color::Cyan)));
        spans.push(Span::raw(" retry  "));
    }

    if caps.can_run_all_files {
        spans.push(Span::styled(
            "A",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" all files  "));
    }

    if app.can_retry_all() {
        spans.push(Span::styled(
            "R",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" RETRY ALL  "));
    }

    if caps.can_fix {
        spans.push(Span::styled(
            "x",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" fix", Style::default().fg(Color::Green)));
        spans.push(Span::raw("  "));
    }

    let fixable_count = app.get_fixable_checks().len();
    if fixable_count > 0 && !app.fix.running && !app.fix.all_running {
        spans.push(Span::styled(
            "X",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" FIX ALL({})", fixable_count),
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::raw("  "));
    }

    spans.push(Span::styled(
        format!("  {} (built {})", GIT_HASH, BUILD_DATE),
        Style::default().fg(Color::DarkGray),
    ));

    spans
}

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    if let Some(ref msg) = app.view.status_message {
        let (icon, color) = match msg.kind {
            StatusKind::Info { .. } => (" ℹ ", Color::Cyan),
            StatusKind::Error => (" ✗ ", Color::Red),
            StatusKind::Progress { .. } => (" ⟳ ", Color::Yellow),
        };
        let status_line = Line::from(vec![
            Span::styled(
                icon,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg.text.clone(), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled(
                "[press any key to dismiss]",
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        let paragraph = Paragraph::new(status_line);
        frame.render_widget(paragraph, area);
    } else {
        let spans = build_footer_shortcuts(app);
        let help = Line::from(spans);
        let paragraph = Paragraph::new(help);
        frame.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_width_respects_char_boundaries() {
        assert_eq!(prefix_width("Zażółć", 3), "Zaż");
        assert_eq!(prefix_width("abc", 10), "abc");
        assert_eq!(prefix_width("abc", 0), "");
    }

    #[test]
    fn test_suffix_width_respects_char_boundaries() {
        assert_eq!(suffix_width("Zażółć", 3), "ółć");
        assert_eq!(suffix_width("abc", 10), "abc");
        assert_eq!(suffix_width("abc", 0), "");
    }

    #[test]
    fn test_width_helpers_count_wide_chars_as_two_columns() {
        assert_eq!(prefix_width("日本語テスト", 5), "日本");
        assert_eq!(suffix_width("日本語テスト", 5), "スト");
    }

    #[test]
    fn test_truncate_path_wide_chars_fit_width() {
        let out = truncate_path("src/日本語/テストファイル名前.rs", 12);
        assert!(out.width() <= 12, "{}", out);
    }

    #[test]
    fn test_truncate_path_multibyte() {
        let path = "src/Ćwiczenia/Zażółć/Gęślą_jaźń_bardzo_długa_nazwa.php";
        let out = truncate_path(path, 20);
        assert!(out.width() <= 20, "{}", out);
        assert!(out.starts_with('…'));
    }

    #[test]
    fn test_truncate_path_keeps_ascii_behavior() {
        assert_eq!(truncate_path("src/main.rs", 20), "src/main.rs");
        assert_eq!(
            truncate_path("a/very/long/dir/path/file.rs", 16),
            "…r/path/file.rs"
        );
    }
}
