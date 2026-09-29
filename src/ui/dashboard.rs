//! Rendering logic for the TUI dashboard using ratatui widgets.
//!
//! This module contains all the rendering functions that draw the UI
//! components: header with progress, system stats (CPU sparkline, MEM gauge), check list,
//! file list, output panel, and footer with keyboard shortcuts.
//!
//! # Layout
//!
//! The UI is divided into four horizontal sections:
//! 1. **Header**: Progress gauge with branch info and elapsed time
//! 2. **System stats**: CPU sparkline with 0/50/100 scale, memory gauge
//!    (hidden with `--no-stats` / `m`; its rows go to the main content)
//! 3. **Main content**: Split into checks list, files list, and output panel
//! 4. **Footer**: Keyboard shortcuts and version info
//!
//! Below [`MIN_WIDTH`] x [`min_height`] only a "terminal too small" message
//! renders. With color off every cell's fg/bg is reset after drawing.

use super::app::{App, PreCommandState, PreCommandStatus, SelectableItem, StatusKind};
use crate::checks::CheckFiles;
use crate::runner::{CheckResult, CheckStatus};
use crate::utils::time;
use ansi_to_tui::IntoText;
use chrono::{DateTime, Local};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Sparkline, Wrap,
    },
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
/// Max files listed by name in the collapsed output header; more show a count
const FILES_INLINE_MAX: usize = 3;
/// Max stderr lines shown per failed fix in fix-all results
const FIX_ERROR_PREVIEW_LINES: usize = 5;
/// Style patched onto a failed check's error-pattern lines
const ERROR_LINE_STYLE: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
/// Columns of the CPU y-axis scale: widest label ("100") + 1 gap
const CPU_SCALE_COLS: u16 = 4;
/// Rows of the header (progress gauge)
const HEADER_ROWS: u16 = 3;
/// Rows of the CPU/MEM stats panel (taller for better graphs)
const STATS_ROWS: u16 = 6;
/// Minimum rows of the main content (checks + output)
const MAIN_MIN_ROWS: u16 = 10;
/// Rows of the footer (shortcuts / status message)
const FOOTER_ROWS: u16 = 2;
/// Narrowest terminal the layout renders in; below it only a "terminal too
/// small" message shows
const MIN_WIDTH: u16 = 60;

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

/// CPU y-axis labels, one right-aligned row per graph row: `100` on top,
/// `0` at bottom, `50` on the row where a half-height bar tops out
fn cpu_scale_lines(height: usize) -> Vec<Line<'static>> {
    let mut labels = vec![""; height];
    if height > 0 {
        labels[height / 2] = "50";
        labels[height - 1] = "0";
        labels[0] = "100";
    }
    labels
        .into_iter()
        .map(|l| {
            Line::from(format!(
                "{:>width$} ",
                l,
                width = CPU_SCALE_COLS as usize - 1
            ))
        })
        .collect()
}

/// MEM gauge ratio and label (`14.7/30.7 GiB (48%)`) from one usage percent.
/// Ratio is clamped to `0.0..=1.0` (`Gauge::ratio` panics outside it).
fn mem_gauge_parts(used_gib: f64, total_gib: f64, usage_pct: f32) -> (f64, String) {
    let ratio = (f64::from(usage_pct) / 100.0).clamp(0.0, 1.0);
    let label = format!("{:.1}/{:.1} GiB ({:.0}%)", used_gib, total_gib, usage_pct);
    (ratio, label)
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
        Some(CheckStatus::Queued) => ("◌", Style::default().fg(Color::Blue)),
        Some(CheckStatus::Skipped) => ("⊘", Style::default().fg(Color::DarkGray)),
        Some(CheckStatus::OnDemand) => ("◇", Style::default().fg(Color::Cyan)),
        None => ("?", Style::default().fg(Color::DarkGray)),
    }
}

/// " +N on-demand" header suffix (empty without on-demand checks)
fn on_demand_suffix(counts: &super::app::StatusCounts) -> String {
    if counts.on_demand > 0 {
        format!(" +{} on-demand", counts.on_demand)
    } else {
        String::new()
    }
}

/// Header label once all checks finished; also the end-of-run notification
/// and the summary printed at quit. Checks that never ran (a failed group
/// pre-command stopped the run) count as pending, not passed.
pub(crate) fn finished_status_text(counts: &super::app::StatusCounts, elapsed: &str) -> String {
    let (passed, failed, cancelled, pending) = (
        counts.passed,
        counts.failed,
        counts.cancelled,
        counts.pending,
    );
    let total = counts.auto_run();
    let on_demand = on_demand_suffix(counts);
    if failed == 0 && cancelled == 0 && pending == 0 {
        return format!("✓ All {total} checks passed in {elapsed}{on_demand}");
    }
    let mark = if failed > 0 { "✗ " } else { "" };
    let mut rest = String::new();
    if cancelled > 0 {
        rest += &format!(", {cancelled} cancelled");
    }
    if pending > 0 {
        rest += &format!(", {pending} pending");
    }
    format!("{mark}{passed}/{total} passed, {failed} failed{rest} in {elapsed}{on_demand}")
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

/// Rows of the stats panel: [`STATS_ROWS`] when shown, else none
fn stats_rows(stats_visible: bool) -> u16 {
    if stats_visible {
        STATS_ROWS
    } else {
        0
    }
}

/// Lowest terminal height the layout renders in
fn min_height(stats_visible: bool) -> u16 {
    HEADER_ROWS + stats_rows(stats_visible) + MAIN_MIN_ROWS + FOOTER_ROWS
}

pub fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();
    let min_height = min_height(app.view.stats_visible);
    if area.width < MIN_WIDTH || area.height < min_height {
        // Hidden layout: drop its areas so clicks / scroll can't hit it
        app.set_checks_list_layout(Rect::default());
        app.set_output_layout(Rect::default(), Rect::default());
        render_too_small(frame, min_height);
    } else {
        render_dashboard(app, frame);
    }
    // Color off: reset every cell's fg/bg; modifiers (bold, reversed
    // selection) stay so the UI remains usable
    if !app.color {
        frame
            .buffer_mut()
            .set_style(area, Style::new().fg(Color::Reset).bg(Color::Reset));
    }
}

/// Header, stats (if shown), main content, footer, and the help overlay
fn render_dashboard(app: &mut App, frame: &mut Frame) {
    let stats_rows = stats_rows(app.view.stats_visible);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_ROWS),
            Constraint::Length(stats_rows),
            Constraint::Min(MAIN_MIN_ROWS),
            Constraint::Length(FOOTER_ROWS),
        ])
        .split(frame.area());

    render_header(app, frame, chunks[0]);
    if app.view.stats_visible {
        render_system_stats(app, frame, chunks[1]);
    }
    render_main(app, frame, chunks[2]);
    render_footer(app, frame, chunks[3]);

    if app.view.help_visible {
        render_help_overlay(frame);
    }
}

/// Centered "terminal too small" notice with current and required size, in
/// place of a clipped layout
fn render_too_small(frame: &mut Frame, min_height: u16) {
    let area = frame.area();
    let lines = vec![
        Line::from(Span::styled(
            "Terminal too small",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "{}x{}, need {}x{}",
            area.width, area.height, MIN_WIDTH, min_height
        )),
    ];
    let height = (lines.len() as u16).min(area.height);
    let row = area.centered_vertically(Constraint::Length(height));
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: true }),
        row,
    );
}

/// Help overlay key bindings: (key, action), shown in two columns
const HELP_LEFT: [(&str, &str); 10] = [
    ("↑/k ↓/j", "prev/next row"),
    ("g / G", "first/last row"),
    ("n / N", "next/prev failed"),
    ("Space/Enter", "fold group"),
    ("PgUp/PgDn", "scroll page"),
    ("Home/End", "top/bottom"),
    ("J / K", "scroll line"),
    ("Ctrl-d/u", "half page"),
    ("/", "search output"),
    ("o / O", "$PAGER/$EDITOR"),
];
const HELP_RIGHT: [(&str, &str); 10] = [
    ("f / a", "failed/all"),
    ("r / R", "retry/retry all"),
    ("t", "run on-demand"),
    ("s", "cancel running"),
    ("x / X", "fix/fix all"),
    ("A", "run on all files"),
    ("c / e", "copy/expand cmd"),
    ("m", "CPU/MEM stats"),
    ("w", "save log file"),
    ("q / Ctrl-c", "quit"),
];
/// Columns of one help column: space, 11-col key, space, 16-col action
const HELP_COL_WIDTH: usize = 29;

/// One padded help column cell: ` key         action`
fn help_cell((key, action): (&str, &str)) -> String {
    format!(
        "{:<width$}",
        format!(" {key:<11} {action}"),
        width = HELP_COL_WIDTH
    )
}

/// Centered popup listing all key bindings (`?` to toggle, `Esc` to close).
/// Sized to its content so it fits the minimum terminal size (60x15).
fn render_help_overlay(frame: &mut Frame) {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{:<HELP_COL_WIDTH$}", " Navigation / Output"), bold),
        Span::styled(" Actions", bold),
    ])];
    lines.extend(
        HELP_LEFT
            .into_iter()
            .zip(HELP_RIGHT)
            .map(|(l, r)| Line::from(help_cell(l) + &help_cell(r))),
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Press ? or Esc to close",
        Style::default().fg(Color::DarkGray),
    )));

    let width = (2 * HELP_COL_WIDTH as u16) + PANEL_BORDER_COLS;
    let height = lines.len() as u16 + PANEL_BORDER_ROWS as u16;
    let area = frame
        .area()
        .centered(Constraint::Max(width), Constraint::Max(height));
    frame.render_widget(Clear, area);
    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Help ")
            .style(Style::default()),
    );
    frame.render_widget(paragraph, area);
}

/// Header title: branch, changed files vs base, elapsed, and the active
/// `--only` / `--group` filter (persistent, unlike the dismissable footer).
fn header_title(app: &App, elapsed: &str) -> String {
    let filter = app
        .filter_notice
        .as_deref()
        .map(|notice| format!(" │ {notice}"))
        .unwrap_or_default();
    format!(
        " {} │ {} files vs {} │ {}{} ",
        app.current_branch,
        app.changed_files.len(),
        app.changed_files.base_ref,
        elapsed,
        filter
    )
}

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let counts = app.count_by_status();
    let auto_run_total = counts.auto_run();
    let completed = counts.completed();
    let ratio = if auto_run_total > 0 {
        completed as f64 / auto_run_total as f64
    } else {
        1.0 // All checks are on-demand, show as complete
    };

    // Format elapsed time
    let elapsed = app.elapsed_time();
    let elapsed_str = format_elapsed(elapsed);

    let branch_info = header_title(app, &elapsed_str);

    let on_demand_text = on_demand_suffix(&counts);
    let status_text = if app.run.all_finished {
        finished_status_text(&counts, &elapsed_str)
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
        } else if counts.cancelled > 0 || counts.pending > 0 {
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
    if !app.color {
        mark_gauge_fill(frame, Block::bordered().inner(area), ratio);
    }
}

/// Color off: a Gauge draws its fill as `█` except under the label, where
/// only swapped fg/bg mark it; reverse those cells so the fill stays visible
fn mark_gauge_fill(frame: &mut Frame, inner: Rect, ratio: f64) {
    let end = inner.left() + (f64::from(inner.width) * ratio).round() as u16;
    let buf = frame.buffer_mut();
    for pos in inner.positions().filter(|p| p.x < end) {
        let cell = &mut buf[pos];
        if cell.symbol() != symbols::block::FULL {
            cell.modifier.insert(Modifier::REVERSED);
        }
    }
}

fn render_system_stats(app: &App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // CPU: bordered block split into y-axis scale + sparkline
    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" CPU {:.0}% ", app.cpu_usage()));
    let cpu_inner = cpu_block.inner(chunks[0]);
    frame.render_widget(cpu_block, chunks[0]);
    let cpu_parts = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(CPU_SCALE_COLS), Constraint::Min(0)])
        .split(cpu_inner);

    let scale = Paragraph::new(cpu_scale_lines(cpu_parts[0].height as usize))
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(scale, cpu_parts[0]);

    let cpu_data = prepare_sparkline_data(&app.sys.cpu_history, cpu_parts[1].width as usize);
    let cpu_sparkline = Sparkline::default()
        .data(&cpu_data)
        .max(100)
        .style(Style::default().fg(get_usage_color(app.cpu_usage())))
        .bar_set(symbols::bar::NINE_LEVELS);
    frame.render_widget(cpu_sparkline, cpu_parts[1]);

    // MEM: gauge of current usage (history adds little for memory)
    let mem_pct = app.mem_usage();
    let (mem_ratio, mem_label) = mem_gauge_parts(app.mem_used_gib(), app.mem_total_gib(), mem_pct);
    let mem_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" MEM "))
        .gauge_style(
            Style::default()
                .fg(get_usage_color(mem_pct))
                .bg(Color::DarkGray),
        )
        .ratio(mem_ratio)
        .label(mem_label);
    frame.render_widget(mem_gauge, chunks[1]);
    if !app.color {
        mark_gauge_fill(frame, Block::bordered().inner(chunks[1]), mem_ratio);
    }
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
        PreCommandStatus::Skipped => ("⊘", Style::default().fg(Color::DarkGray)),
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

/// Duration span beside a check name: final `duration_ms` once finished,
/// " cached" for cached passes, live elapsed since `started_at` while
/// running, nothing before start
fn check_duration_span(result: &CheckResult, now: DateTime<Local>) -> Span<'static> {
    let done_style = Style::default().fg(Color::DarkGray);
    // Dimmed running-yellow: still ticking, not a final duration
    let live_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::DIM);
    match (&result.status, result.started_at) {
        _ if result.cached => Span::styled(" cached", done_style),
        (CheckStatus::Running, Some(started)) => {
            let elapsed_ms = (now - started).num_milliseconds().max(0) as u64;
            Span::styled(format!(" {}", time::format(elapsed_ms)), live_style)
        }
        _ if result.duration_ms > 0 => {
            Span::styled(format!(" {}", time::format(result.duration_ms)), done_style)
        }
        _ => Span::raw(""),
    }
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
        .map(|r| check_duration_span(r, Local::now()))
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
        duration,
    ]))
}

fn render_checks_list(app: &mut App, frame: &mut Frame, area: Rect) {
    let items = build_checks_list_items(app, area);
    let selected_row = (!items.is_empty()).then_some(app.view.selected_check);

    let filter_info = match app.view.status_filter {
        super::app::StatusFilter::All => "",
        super::app::StatusFilter::Failed => " [failed]",
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Checks{} ", filter_info));
    // Store the block's inner (border-excluded) rect so mouse click
    // hit-testing maps 1:1 onto the rows the list actually draws into.
    app.set_checks_list_layout(block.inner(area));

    // scroll_padding keeps the neighbor rows (e.g. group headers) in view
    let list = List::new(items).scroll_padding(1).block(block);

    // Persist the scroll offset so the list scrolls only when the selection
    // leaves the visible window
    let mut state = ListState::default()
        .with_offset(app.view.checks_list_offset)
        .with_selected(selected_row);
    frame.render_stateful_widget(list, area, &mut state);
    app.view.checks_list_offset = state.offset();
}

/// Build the checks list rows, one per [`App::selectable_items`] entry
/// (group headers included), so row index == item index and the list and
/// the selection can never disagree.
fn build_checks_list_items(app: &App, area: Rect) -> Vec<ListItem<'static>> {
    app.selectable_items()
        .enumerate()
        .map(|(idx, item)| {
            let is_selected = idx == app.view.selected_check;
            match item {
                SelectableItem::Group(group) => group_header_item(app, group, is_selected),
                SelectableItem::PreCommand(pc) => render_pre_command_item(pc, is_selected),
                SelectableItem::Check(check) => render_check_item(app, check, area, is_selected),
            }
        })
        .collect()
}

/// Group header row: fold marker (`▾` open, `▸ NAME (n)` folded with its
/// hidden row count); highlighted while the group runs
fn group_header_item(app: &App, group: &str, is_selected: bool) -> ListItem<'static> {
    let color = if app.run.current_group.as_deref() == Some(group) {
        Color::Yellow
    } else {
        Color::Cyan
    };
    let mut group_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    if is_selected {
        group_style = group_style.add_modifier(Modifier::REVERSED);
    }
    let display_name = app.group_display_name(group).to_uppercase();
    let label = if app.is_group_collapsed(group) {
        format!("▸ {} ({})", display_name, app.group_child_count(group))
    } else {
        format!("▾ {}", display_name)
    };
    ListItem::new(Line::from(vec![
        Span::styled(label, group_style),
        Span::styled(" ───────────", Style::default().fg(Color::DarkGray)),
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
            // Group header: no output of its own (placeholder text)
            Some(SelectableItem::Group(_)) | None => OutputView::Check(None),
        }
    }
}

/// Cheap fields that change whenever the output panel's rendered content
/// would change. Comparing these against the previous frame's key tells us
/// whether the cached [`Text`] is still valid, without needing to rebuild
/// the raw output string (the expensive part) just to check for staleness.
///
/// `output_len`/`error_len` catch content growth (a streamed append to a
/// rolling-capped buffer that keeps its length is not caught here:
/// `App::on_check_output` drops the cache instead); `status` catches
/// transitions (e.g. Running -> Passed)
/// that don't change length; `width` catches resize; `resolved_command`
/// catches a retried check's command line changing without status/output
/// changing yet.
#[derive(Debug, Clone, PartialEq)]
enum OutputCacheKey {
    Check {
        id: String,
        status: CheckStatus,
        output_len: usize,
        error_len: usize,
        show_full_command: bool,
        resolved_command: String,
    },
    PreCommand {
        name: String,
        status: PreCommandStatus,
        output_len: usize,
    },
    FixResult {
        status: CheckStatus,
        output_len: usize,
        error_len: usize,
    },
    FixAllResults {
        len: usize,
        passed: usize,
        failed: usize,
    },
}

/// Cached parsed ANSI [`Text`] and its wrapped line count for the output
/// panel, keyed on [`OutputCacheKey`] plus the width it was wrapped at.
/// Shared by [`output_line_count`] (scroll clamping) and [`render_scrollable`]
/// (drawing) so a frame where nothing changed parses/wraps the output at
/// most once instead of twice (see issue #126).
pub(crate) struct OutputCache {
    key: OutputCacheKey,
    width: u16,
    text: Text<'static>,
    line_count: usize,
    /// Text with the confirmed search query's matches styled, computed once
    /// by [`confirm_search`] rather than every frame — `None` when there is
    /// no confirmed search (or it had no matches).
    highlighted: Option<Text<'static>>,
    /// Scroll offset putting a failed check's first `error_pattern` line at
    /// the top (selection jump target); `None` when not failed / no match.
    /// Error lines themselves are styled in place in `text`.
    first_error_scroll: Option<usize>,
}

/// Count passed/failed among `fix.all_results` in a single pass.
fn fix_all_passed_failed(app: &App) -> (usize, usize) {
    app.fix
        .all_results
        .iter()
        .fold((0, 0), |(p, f), r| match r.status {
            CheckStatus::Passed => (p + 1, f),
            CheckStatus::Failed => (p, f + 1),
            _ => (p, f),
        })
}

/// Cheap key describing the current output view, or `None` for views that
/// don't scroll (running spinners) / have no result to show yet — mirrors
/// the `return 0` cases the old `output_line_count` had. Only called on a
/// cache miss; [`output_cache_key_matches`] checks validity on a hit without
/// allocating a fresh key.
fn output_cache_key(app: &App) -> Option<OutputCacheKey> {
    match output_view(app) {
        OutputView::FixAllResults => {
            let (passed, failed) = fix_all_passed_failed(app);
            Some(OutputCacheKey::FixAllResults {
                len: app.fix.all_results.len(),
                passed,
                failed,
            })
        }
        OutputView::FixResult(result) => Some(OutputCacheKey::FixResult {
            status: result.status.clone(),
            output_len: result.output.len(),
            error_len: result.error_output.len(),
        }),
        OutputView::PreCommand(pc) => Some(OutputCacheKey::PreCommand {
            name: pc.name.clone(),
            status: pc.status.clone(),
            output_len: pc.output.len(),
        }),
        OutputView::Check(Some(check)) => {
            let result = app.results.get(check.id())?;
            Some(OutputCacheKey::Check {
                id: check.id().to_string(),
                status: result.status.clone(),
                output_len: result.output.len(),
                error_len: result.error_output.len(),
                show_full_command: app.view.show_full_command,
                resolved_command: check.resolved_command.clone(),
            })
        }
        OutputView::FixAllRunning | OutputView::FixRunning | OutputView::Check(None) => None,
    }
}

/// Whether `cache.key` still matches the current output view, compared
/// field-by-field against borrowed data so a cache hit — the common case,
/// checked every frame — doesn't allocate a fresh [`OutputCacheKey`] just to
/// throw it away.
fn output_cache_key_matches(app: &App, cache: &OutputCache) -> bool {
    match (output_view(app), &cache.key) {
        (
            OutputView::FixAllResults,
            OutputCacheKey::FixAllResults {
                len,
                passed,
                failed,
            },
        ) => {
            let (p, f) = fix_all_passed_failed(app);
            *len == app.fix.all_results.len() && *passed == p && *failed == f
        }
        (
            OutputView::FixResult(result),
            OutputCacheKey::FixResult {
                status,
                output_len,
                error_len,
            },
        ) => {
            *status == result.status
                && *output_len == result.output.len()
                && *error_len == result.error_output.len()
        }
        (
            OutputView::PreCommand(pc),
            OutputCacheKey::PreCommand {
                name,
                status,
                output_len,
            },
        ) => *name == pc.name && *status == pc.status && *output_len == pc.output.len(),
        (
            OutputView::Check(Some(check)),
            OutputCacheKey::Check {
                id,
                status,
                output_len,
                error_len,
                show_full_command,
                resolved_command,
            },
        ) => {
            let Some(result) = app.results.get(check.id()) else {
                return false;
            };
            id == check.id()
                && *status == result.status
                && *output_len == result.output.len()
                && *error_len == result.error_output.len()
                && *show_full_command == app.view.show_full_command
                && *resolved_command == check.resolved_command
        }
        _ => false,
    }
}

/// Where to look for a failed check's error lines in its parsed output
struct ErrorScan {
    /// The check's `error_pattern` (or the built-in default)
    re: regex::Regex,
    /// First line of command output; lines before it are ci-tui's header
    start: usize,
    /// ci-tui's own `── stderr ──` separator line, never an error line
    separator: Option<usize>,
}

/// Build the raw output string for the current view. Only called on a cache
/// miss — this is the work the cache exists to avoid repeating every frame.
///
/// Also returns, for a failed check's view only, where to look for its error
/// lines.
fn build_raw_output(app: &App, width: u16) -> Option<(String, Option<ErrorScan>)> {
    match output_view(app) {
        OutputView::FixAllResults => Some((fix_all_results_text(app), None)),
        OutputView::FixResult(result) => Some((fix_result_text(result), None)),
        OutputView::PreCommand(pc) => Some((pre_command_output_text(pc), None)),
        OutputView::Check(Some(check)) => {
            let result = app.results.get(check.id())?;
            Some(build_check_output_text(app, check, result, width))
        }
        OutputView::FixAllRunning | OutputView::FixRunning | OutputView::Check(None) => None,
    }
}

#[cfg(test)]
static PARSE_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn parse_calls() -> usize {
    PARSE_CALLS.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn reset_parse_calls() {
    PARSE_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
}

/// Parse ANSI output into a [`Text`] and compute its wrapped line count at
/// `width`. This is the expensive step (`ansi_to_tui` parsing + paragraph
/// line-wrapping) that [`ensure_output_cache`] avoids repeating every frame.
fn parse_and_wrap(raw_output: &str, width: u16) -> (Text<'static>, usize) {
    #[cfg(test)]
    PARSE_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let text = raw_output.into_text().unwrap_or_default();
    let line_count = wrapped_line_count(&text.lines, width);
    (text, line_count)
}

/// Number of wrapped display rows `lines` occupy at `width` — the wrapping
/// idiom shared by [`parse_and_wrap`] (whole output) and [`line_start_scroll`]
/// (a prefix of it). Spans borrow their content, so no line text is copied.
fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> usize {
    let borrowed: Vec<Line<'_>> = lines
        .iter()
        .map(|line| Line {
            style: line.style,
            alignment: line.alignment,
            spans: line
                .spans
                .iter()
                .map(|span| Span::styled(span.content.as_ref(), span.style))
                .collect(),
        })
        .collect();
    Paragraph::new(borrowed)
        .wrap(Wrap { trim: false })
        .line_count(width.saturating_sub(PANEL_BORDER_COLS))
}

/// Both keys show the same check (content may differ: width, `e` toggle)
fn same_check(a: &OutputCacheKey, b: &OutputCacheKey) -> bool {
    matches!(
        (a, b),
        (OutputCacheKey::Check { id: x, .. }, OutputCacheKey::Check { id: y, .. }) if x == y
    )
}

/// Make sure `app.output_cache` holds the current output panel's parsed
/// text and wrapped line count, rebuilding only when [`OutputCacheKey`]
/// changed since the last call. Returns `false` for views with nothing to
/// cache (running spinners / no result yet), in which case any stale cache
/// entry is cleared.
fn ensure_output_cache(app: &mut App, width: u16) -> bool {
    if app
        .output_cache
        .as_ref()
        .is_some_and(|c| c.width == width && output_cache_key_matches(app, c))
    {
        return true;
    }
    let Some(key) = output_cache_key(app) else {
        app.output_cache = None;
        return false;
    };
    let Some((raw_output, error_scan)) = build_raw_output(app, width) else {
        app.output_cache = None;
        return false;
    };
    let visible = app.view.output_visible_lines;
    // Still sitting on this check's first error line (not scrolled away):
    // a rebuild that moves it (`e` expand, resize) carries the view along
    let parked_on_error = app.output_cache.as_ref().is_some_and(|old| {
        !app.view.follow_output
            && same_check(&old.key, &key)
            && old.first_error_scroll.is_some_and(|row| {
                row.min(old.line_count.saturating_sub(visible)) == app.view.output_scroll
            })
    });
    let (mut text, line_count) = parse_and_wrap(&raw_output, width);
    let error_lines = error_scan
        .map(|scan| error_line_indices(&text, &scan))
        .unwrap_or_default();
    style_error_lines(&mut text, &error_lines);
    let first_error_scroll = error_lines
        .first()
        .map(|&line| line_start_scroll(&text, line, width));
    if let Some(row) = first_error_scroll.filter(|_| parked_on_error) {
        app.view.output_scroll = row.min(line_count.saturating_sub(visible));
    }
    app.output_cache = Some(OutputCache {
        key,
        width,
        text,
        line_count,
        highlighted: None,
        first_error_scroll,
    });
    refresh_confirmed_search(app);
    true
}

/// Count the screen rows the output panel currently draws (after wrapping
/// at the panel's inner width).
///
/// Used by [`App`] scroll clamping. Views without text (fix spinners, no
/// selection) count 0.
pub fn output_line_count(app: &mut App, width: u16) -> usize {
    if !ensure_output_cache(app, width) {
        return 0;
    }
    app.output_cache.as_ref().map_or(0, |c| c.line_count)
}

/// Indices of command-output lines (from `scan.start`, minus ci-tui's
/// separator) whose plain text matches `scan.re`
fn error_line_indices(text: &Text<'static>, scan: &ErrorScan) -> Vec<usize> {
    text.lines
        .iter()
        .enumerate()
        .skip(scan.start)
        .filter(|&(i, line)| Some(i) != scan.separator && scan.re.is_match(&line_plain_text(line)))
        .map(|(i, _)| i)
        .collect()
}

/// Patch [`ERROR_LINE_STYLE`] onto every span of the `lines` indices (other
/// styling, e.g. background, kept). Style-only: wrapping and search, which
/// read plain text, are unaffected.
fn style_error_lines(text: &mut Text<'static>, lines: &[usize]) {
    for &i in lines {
        for span in &mut text.lines[i].spans {
            span.style = span.style.patch(ERROR_LINE_STYLE);
        }
    }
}

/// Cached scroll offset of the current view's first error line (see
/// [`OutputCache::first_error_scroll`]); read on the render path, after
/// [`ensure_output_cache`] ran for this frame
pub(crate) fn first_error_scroll(app: &App) -> Option<usize> {
    app.output_cache.as_ref()?.first_error_scroll
}

/// Text the output panel draws: confirmed search highlight, else the parsed
/// output (error lines already styled)
fn output_display_text(app: &App, cache: &OutputCache) -> Text<'static> {
    let confirmed = app.view.search.as_ref().is_some_and(|s| !s.typing);
    confirmed
        .then_some(cache.highlighted.as_ref())
        .flatten()
        .unwrap_or(&cache.text)
        .clone()
}

/// Render scrollable ANSI output from the cache populated by
/// [`ensure_output_cache`] (already primed this frame by
/// [`App::clamp_output_scroll`], called from [`render_output`] with the
/// same `area.width` before dispatch). The title gets `[row/total]` when
/// the wrapped text overflows the panel.
fn render_scrollable(app: &App, frame: &mut Frame, area: Rect, title: &str) {
    let Some(cache) = app.output_cache.as_ref() else {
        return;
    };
    let total_lines = cache.line_count;
    let visible_lines = app.view.output_visible_lines;
    let title = if total_lines > visible_lines && visible_lines > 0 {
        let current_line = app.view.output_scroll + 1;
        format!(" {} [{}/{}] ", title, current_line, total_lines)
    } else {
        format!(" {} ", title)
    };
    // ratatui scrolls by u16; larger offsets saturate instead of wrapping
    let scroll = u16::try_from(app.view.output_scroll).unwrap_or(u16::MAX);
    let text = output_display_text(app, cache);
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

/// Plain-text content of a line (concatenated span contents, styles dropped)
fn line_plain_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Number of wrapped display rows the lines *before* `target_line` occupy at
/// `width` — i.e. the scroll offset needed to bring `target_line` to the top
/// of the output panel. Reuses [`Paragraph::line_count`], the same wrapping
/// logic [`parse_and_wrap`] uses for the total count.
fn line_start_scroll(text: &Text<'static>, target_line: usize, width: u16) -> usize {
    wrapped_line_count(&text.lines[..target_line], width)
}

/// Split `plain` into spans, styling every case-insensitive occurrence of
/// `needle` (ASCII-lowercased `query`) as a highlight; `query` gives the
/// original case/length of the match text to slice out of `plain`.
///
/// Case-folding is ASCII-only (not [`str::to_lowercase`]) so `lower` is
/// guaranteed to have the same byte length and boundaries as `plain` — full
/// Unicode lowercasing can change a character's byte length (e.g. `İ`
/// U+0130 is 2 bytes but lowercases to a 3-byte sequence), which would
/// desync the byte offsets found in `lower` from `plain` and panic on slice.
fn highlight_line_spans(plain: &str, lower: &str, needle: &str, query: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut idx = 0;
    while idx < plain.len() {
        let Some(pos) = lower[idx..].find(needle) else {
            spans.push(Span::raw(plain[idx..].to_string()));
            break;
        };
        let start = idx + pos;
        if start > idx {
            spans.push(Span::raw(plain[idx..start].to_string()));
        }
        let end = start + query.len();
        spans.push(Span::styled(
            plain[start..end].to_string(),
            // Underline keeps matches visible with color off (NO_COLOR)
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::UNDERLINED),
        ));
        idx = end;
    }
    spans
}

/// Rebuild `text` with every occurrence of `query` on the given `matches`
/// line indices styled as a highlight (indices already known to contain a
/// case-insensitive match, e.g. from [`compute_search_matches`] — this does
/// not rescan every line to find them). Lines outside `matches` are returned
/// unchanged (styling preserved); matching lines lose their original ANSI
/// styling in favor of a plain highlight — an acceptable simplification for
/// search results.
fn highlight_matches(text: &Text<'static>, matches: &[usize], query: &str) -> Text<'static> {
    if matches.is_empty() || query.is_empty() {
        return text.clone();
    }
    let needle = query.to_ascii_lowercase();
    let match_lines: std::collections::HashSet<usize> = matches.iter().copied().collect();
    let lines = text
        .lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if !match_lines.contains(&i) {
                return line.clone();
            }
            let plain = line_plain_text(line);
            let lower = plain.to_ascii_lowercase();
            Line::from(highlight_line_spans(&plain, &lower, &needle, query))
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

/// Line indices in `cache.text` that case-insensitively contain `query`
/// (ASCII-only fold, matching [`highlight_matches`]/[`highlight_line_spans`]
/// so a line that's counted as a match is always one that highlights
/// correctly), plus the highlighted text if there was at least one match.
fn compute_search_matches(cache: &OutputCache, query: &str) -> (Vec<usize>, Option<Text<'static>>) {
    if query.is_empty() {
        return (Vec::new(), None);
    }
    let needle = query.to_ascii_lowercase();
    let matches: Vec<usize> = cache
        .text
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line_plain_text(line).to_ascii_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect();
    if matches.is_empty() {
        return (matches, None);
    }
    let highlighted = highlight_matches(&cache.text, &matches, query);
    (matches, Some(highlighted))
}

/// Recompute a confirmed search's matches/highlight against the current
/// output cache. Called after [`ensure_output_cache`] rebuilds the cache
/// (retry, resize, ...) so a stale `search.matches` (and the footer's match
/// count) don't linger against output that no longer matches them. No-op
/// while still typing (nothing confirmed yet) or with no search open.
fn refresh_confirmed_search(app: &mut App) {
    let Some(query) = app
        .view
        .search
        .as_ref()
        .filter(|s| !s.typing)
        .map(|s| s.query.clone())
    else {
        return;
    };
    let Some(cache) = app.output_cache.as_ref() else {
        return;
    };
    let (matches, highlighted) = compute_search_matches(cache, &query);
    if let Some(search) = app.view.search.as_mut() {
        search.matches = matches;
    }
    if let Some(cache) = app.output_cache.as_mut() {
        cache.highlighted = highlighted;
    }
}

/// Confirm the in-progress output search (`Enter` while typing): computes
/// matches against the currently selected check's output and jumps the
/// scroll offset to the first one. No-op if not currently typing a search.
pub(crate) fn confirm_search(app: &mut App) {
    let width = app.output_area_width;
    ensure_output_cache(app, width);

    let Some(query) = app
        .view
        .search
        .as_ref()
        .filter(|s| s.typing)
        .map(|s| s.query.clone())
    else {
        return;
    };

    let (matches, first_scroll, highlighted) = match &app.output_cache {
        Some(cache) => {
            let (matches, highlighted) = compute_search_matches(cache, &query);
            let first_scroll = matches
                .first()
                .map(|&idx| line_start_scroll(&cache.text, idx, width));
            (matches, first_scroll, highlighted)
        }
        None => (Vec::new(), None, None),
    };

    if let Some(search) = app.view.search.as_mut() {
        search.typing = false;
        search.matches = matches;
    }
    if let Some(cache) = app.output_cache.as_mut() {
        cache.highlighted = highlighted;
    }
    if let Some(scroll) = first_scroll {
        // Stay at the match rather than following streamed output
        app.scroll_to_row(scroll);
    }
    app.needs_redraw = true;
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
    render_scrollable(app, frame, area, "Fix All Results");
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
fn render_fix_result(app: &App, frame: &mut Frame, area: Rect) {
    render_scrollable(app, frame, area, "Fix Result");
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

/// Build the output string for a check with results: header (command,
/// status, files), then command output (stderr above stdout while running,
/// below once failed). Also returns, for a failed check, where its error
/// lines may be (command output only), so ci-tui's own `✗ FAILED` header and
/// `── stderr ──` separator are never taken for error lines.
fn build_check_output_text(
    app: &App,
    check: &crate::checks::CheckToRun,
    result: &crate::runner::CheckResult,
    width: u16,
) -> (String, Option<ErrorScan>) {
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
        CheckStatus::Queued => "\x1b[34m◌ QUEUED\x1b[0m",
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
    } else if matches!(result.status, CheckStatus::Queued | CheckStatus::Running) {
        raw_output.push_str("  \x1b[33m← press 's' to cancel\x1b[0m");
    }
    raw_output.push_str("\n\n");

    // Files
    append_files_section(&mut raw_output, app, check);
    let output_start = raw_output.matches('\n').count();

    // While running, streamed stderr goes above stdout: follow pins the panel
    // to the bottom, where long stderr (compose/cargo progress) would
    // otherwise push the latest stdout off-screen
    if result.status == CheckStatus::Running && !result.error_output.is_empty() {
        raw_output.push_str("\x1b[31m── stderr ──\x1b[0m\n");
        raw_output.push_str(result.error_output.trim_end());
        raw_output.push_str("\n\n");
    }

    // Command output
    if !result.output.is_empty() {
        raw_output.push_str(&result.output);
    }

    // Stderr for failed checks
    let mut separator = None;
    if result.status.is_failure() && !result.error_output.is_empty() {
        // The leading newline ends the output's last line; the separator is
        // on the line after it
        separator = Some(raw_output.matches('\n').count() + 1);
        raw_output.push_str("\n\x1b[31m── stderr ──\x1b[0m\n");
        raw_output.push_str(result.error_output.trim_end());
    }

    if result.status == CheckStatus::Pending {
        raw_output.push_str("\x1b[90mWaiting to run...\x1b[0m");
    } else if result.status == CheckStatus::Queued {
        raw_output.push_str("\x1b[90mWaiting for a free slot (max_parallel / --jobs)...\x1b[0m");
    }

    let error_scan = result.status.is_failure().then(|| ErrorScan {
        re: check.definition.error_regex().clone(),
        start: output_start,
        separator,
    });
    (raw_output, error_scan)
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
    } else if files.len() <= FILES_INLINE_MAX {
        raw_output.push_str(&format!("\x1b[90mFiles: {}\x1b[0m\n\n", files.join(", ")));
    } else {
        // Count only: a partial name preview hid the rest without saying so (#145)
        raw_output.push_str(&format!(
            "\x1b[90mFiles: {} [e=expand]\x1b[0m\n\n",
            files.len()
        ));
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

    if !app.results.contains_key(check.id()) {
        let title = format!(" {} ", check.name());
        let block = Block::default().borders(Borders::ALL).title(title);
        let paragraph = Paragraph::new("Select a check to view details")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    render_scrollable(app, frame, area, check.name());
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
        super::app::PreCommandStatus::Skipped => "\x1b[90m⊘ SKIPPED\x1b[0m",
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
    } else if pre_cmd.status == super::app::PreCommandStatus::Skipped {
        raw_output.push_str(
            "\x1b[90mNot run: no check of this group runs. Re-running one of them runs it first.\x1b[0m",
        );
    }

    raw_output
}

/// Render pre-command output details
fn render_pre_command_output(app: &App, pre_cmd: &PreCommandState, frame: &mut Frame, area: Rect) {
    render_scrollable(app, frame, area, &pre_cmd.name);
}

fn render_output(app: &mut App, frame: &mut Frame, area: Rect) {
    // All sub-renderers below draw a Borders::ALL block into `area`; derive
    // the same inner rect here so mouse hit-testing matches what's drawn.
    let inner_area = Block::default().borders(Borders::ALL).inner(area);
    app.set_output_layout(area, inner_area);
    // Prime the cache once up front: `clamp_output_scroll` below only
    // consults it when there's a nonzero scroll offset to clamp, but
    // `render_scrollable` always needs it, so it can't rely on that path
    // alone to populate it (e.g. the very first frame, scroll == 0).
    ensure_output_cache(app, area.width);
    app.clamp_output_scroll();
    app.apply_pending_error_jump();
    app.follow_output_tail();

    // Dispatch to appropriate sub-renderer based on state
    match output_view(app) {
        OutputView::FixAllResults => render_fix_all_results(app, frame, area),
        OutputView::FixAllRunning => render_fix_all_running(app, frame, area),
        OutputView::FixResult(_) => render_fix_result(app, frame, area),
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
        Span::styled("?", Style::default().fg(Color::Yellow)),
        Span::raw(" help  "),
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

/// "no matches" / "1 match" / "N matches" summary for the search footer
fn match_count_summary(count: usize) -> String {
    if count == 0 {
        return "no matches".to_string();
    }
    let suffix = if count == 1 { "" } else { "es" };
    format!("{} match{}", count, suffix)
}

/// Footer line while an output search is open (typing or confirmed)
fn search_footer_line(search: &super::app::SearchState) -> Line<'static> {
    let hint = if search.typing {
        Span::styled(
            "  [Enter confirm, Esc cancel]",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        Span::raw(format!(
            "  ({})  ",
            match_count_summary(search.matches.len())
        ))
    };
    Line::from(vec![
        Span::styled("Search: ", Style::default().fg(Color::Yellow)),
        Span::raw(format!("/{}", search.query)),
        hint,
    ])
}

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    // status_message takes priority: it's transient (cleared on the next
    // keypress) and would otherwise be set but never shown while a search
    // is open, e.g. pressing 'c' to copy while a confirmed search/highlight
    // is displayed.
    if let Some(ref search) = app.view.search {
        if app.view.status_message.is_none() {
            frame.render_widget(Paragraph::new(search_footer_line(search)), area);
            return;
        }
    }
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
    fn test_prepare_sparkline_data_trims_to_width_keeping_newest() {
        let history: VecDeque<f32> = (0..600).map(|i| (i % 100) as f32).collect();
        let data = prepare_sparkline_data(&history, 200);
        assert_eq!(data.len(), 200);
        // newest sample (599 % 100) is rightmost
        assert_eq!(data.last(), Some(&99));
    }

    #[test]
    fn test_prepare_sparkline_data_pads_left_when_short() {
        let history: VecDeque<f32> = VecDeque::from(vec![40.0, 60.0]);
        assert_eq!(prepare_sparkline_data(&history, 5), vec![1, 1, 1, 40, 60]);
    }

    #[test]
    fn test_cpu_scale_lines_place_100_50_0() {
        assert_eq!(
            cpu_scale_lines(4),
            ["100 ", "    ", " 50 ", "  0 "].map(Line::from)
        );
        assert_eq!(cpu_scale_lines(3), ["100 ", " 50 ", "  0 "].map(Line::from));
        assert_eq!(cpu_scale_lines(1), ["100 "].map(Line::from));
        assert!(cpu_scale_lines(0).is_empty());
    }

    #[test]
    fn test_mem_gauge_parts_ratio_and_label() {
        let (ratio, label) = mem_gauge_parts(14.7, 30.7, 48.0);
        assert!((ratio - 0.48).abs() < 1e-6);
        assert_eq!(label, "14.7/30.7 GiB (48%)");
    }

    #[test]
    fn test_mem_gauge_parts_clamps_ratio() {
        assert_eq!(mem_gauge_parts(2.0, 1.0, 200.0).0, 1.0);
        assert_eq!(mem_gauge_parts(0.0, 1.0, -5.0).0, 0.0);
        let (ratio, label) = mem_gauge_parts(0.0, 0.0, 0.0);
        assert_eq!(ratio, 0.0);
        assert_eq!(label, "0.0/0.0 GiB (0%)");
    }

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

    use super::super::app::tests::{make_check, minimal_config_yaml, output_event};
    use crate::git::ChangedFiles;
    use crate::runner::CheckStatus;
    use ratatui::{backend::TestBackend, Terminal};

    /// App with one check whose output is large enough to make re-parsing
    /// wasteful (mirrors #126's "MBs of test output" scenario at a test-
    /// friendly size).
    fn make_test_app() -> App {
        let config = serde_yaml::from_str(minimal_config_yaml()).expect("failed to parse config");
        let changed_files = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: "development".to_string(),
        };
        let check = make_check("php-lint", "fast", "PHP Lint", false, false);
        let mut app = App::new(config, changed_files, vec![check], "main".to_string());
        let result = app.results.get_mut("php-lint").unwrap();
        result.status = CheckStatus::Passed;
        result.output = "\x1b[32mok\x1b[0m line\n".repeat(2000);
        app
    }

    /// Result with `status`, started `ago` before a fixed `now`
    fn timed_result(status: CheckStatus, ago_ms: i64) -> (CheckResult, DateTime<Local>) {
        let now = Local::now();
        let mut result = CheckResult::pending("php-lint");
        result.status = status;
        result.started_at = Some(now - chrono::Duration::milliseconds(ago_ms));
        (result, now)
    }

    /// #141: running row shows live elapsed, same format as final durations
    #[test]
    fn test_check_duration_running_shows_live_elapsed() {
        let (result, now) = timed_result(CheckStatus::Running, 5_300);

        let span = check_duration_span(&result, now);

        assert_eq!(span.content, format!(" {}", time::format(5_300)));
        assert_eq!(span.content, " 5.3s");
    }

    /// #141: live elapsed styled apart from final durations
    #[test]
    fn test_check_duration_live_style_differs_from_final() {
        let (running, now) = timed_result(CheckStatus::Running, 1_500);
        let mut passed = running.clone();
        passed.status = CheckStatus::Passed;
        passed.duration_ms = 1_500;

        let live = check_duration_span(&running, now);
        let done = check_duration_span(&passed, now);

        assert_eq!(live.content, done.content);
        assert_ne!(live.style, done.style);
        assert!(live.style.add_modifier.contains(Modifier::DIM));
    }

    /// Finished rows keep `duration_ms`, not elapsed since start
    #[rstest::rstest]
    #[case::passed(CheckStatus::Passed)]
    #[case::failed(CheckStatus::Failed)]
    #[case::timed_out(CheckStatus::TimedOut)]
    #[case::cancelled(CheckStatus::Cancelled)]
    fn test_check_duration_finished_shows_duration_ms(#[case] status: CheckStatus) {
        let (mut result, now) = timed_result(status, 60_000);
        result.duration_ms = 2_000;

        let span = check_duration_span(&result, now);

        assert_eq!(span.content, " 2.0s");
        assert_eq!(span.style, Style::default().fg(Color::DarkGray));
    }

    #[test]
    fn test_check_duration_cached_pass() {
        let (mut result, now) = timed_result(CheckStatus::Passed, 0);
        result.cached = true;

        assert_eq!(check_duration_span(&result, now).content, " cached");
    }

    /// Not-started rows show no time (no `started_at`, or queued)
    #[rstest::rstest]
    #[case::pending(CheckStatus::Pending)]
    #[case::queued(CheckStatus::Queued)]
    #[case::on_demand(CheckStatus::OnDemand)]
    #[case::running_without_start(CheckStatus::Running)]
    fn test_check_duration_not_started_is_empty(#[case] status: CheckStatus) {
        let mut result = CheckResult::pending("php-lint");
        result.status = status;

        assert_eq!(check_duration_span(&result, Local::now()).content, "");
    }

    /// App with one check matching `files`; `expanded` toggles `e` view
    fn files_section_for(files: CheckFiles, expanded: bool) -> String {
        let mut app = make_test_app();
        app.checks[0].files = files;
        app.view.show_full_command = expanded;
        let check = app.checks[0].clone();
        let mut out = String::new();
        append_files_section(&mut out, &app, &check);
        out
    }

    fn five_files() -> CheckFiles {
        CheckFiles::Files((1..=5).map(|i| format!("src/f{}.php", i)).collect())
    }

    /// #145: collapsed view shows a count once names no longer fit inline
    #[test]
    fn test_files_section_collapsed_shows_count_past_inline_max() {
        assert_eq!(
            files_section_for(five_files(), false),
            "\x1b[90mFiles: 5 [e=expand]\x1b[0m\n\n"
        );
    }

    #[test]
    fn test_files_section_collapsed_lists_names_up_to_inline_max() {
        let files = vec!["a.php".into(), "b.php".into(), "c.php".into()];
        assert_eq!(
            files_section_for(CheckFiles::Files(files), false),
            "\x1b[90mFiles: a.php, b.php, c.php\x1b[0m\n\n"
        );
    }

    #[test]
    fn test_files_section_expanded_lists_all_files() {
        assert_eq!(
            files_section_for(five_files(), true),
            "\x1b[90mFiles:\x1b[0m\n\
             \x1b[90m  - src/f1.php\x1b[0m\n\
             \x1b[90m  - src/f2.php\x1b[0m\n\
             \x1b[90m  - src/f3.php\x1b[0m\n\
             \x1b[90m  - src/f4.php\x1b[0m\n\
             \x1b[90m  - src/f5.php\x1b[0m\n\
             \n"
        );
    }

    #[test]
    fn test_files_section_empty_or_non_files_renders_nothing() {
        for expanded in [false, true] {
            for files in [
                CheckFiles::Files(vec![]),
                CheckFiles::RunAll,
                CheckFiles::OnDemand,
                CheckFiles::SkippedNoMatch,
            ] {
                assert_eq!(files_section_for(files, expanded), "");
            }
        }
    }

    /// #126: rendering a frame calls into the cache from two places
    /// (`App::clamp_output_scroll` -> `output_line_count`, then
    /// `render_scrollable`); an unchanged second frame must not reparse.
    #[test]
    fn test_render_output_shares_cache_across_call_sites_and_frames() {
        reset_parse_calls();
        let mut app = make_test_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let area = Rect::new(0, 0, 80, 20);

        terminal
            .draw(|f| render_output(&mut app, f, area))
            .expect("draw");
        assert_eq!(
            parse_calls(),
            1,
            "first frame should parse/wrap exactly once, shared by clamp + render"
        );

        terminal
            .draw(|f| render_output(&mut app, f, area))
            .expect("draw");
        assert_eq!(
            parse_calls(),
            1,
            "second frame with unchanged content/width must hit the cache, not reparse"
        );
    }

    /// Too-small frame hides the layout: clicks at the old checks-list
    /// position must not select hidden rows, scrolling must not panic
    #[test]
    fn test_too_small_clears_stale_layout_areas() {
        let mut app = make_test_app();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|f| render(&mut app, f)).expect("draw");
        let old = app.view.checks_list_area;
        assert!(old.height > 0);
        let selected = app.view.selected_check;

        terminal.backend_mut().resize(40, 10);
        terminal.autoresize().expect("resize");
        terminal.draw(|f| render(&mut app, f)).expect("draw");
        for row in old.y..old.bottom() {
            app.select_check_at_position(old.x + 1, row);
        }
        assert_eq!(app.view.selected_check, selected);
        assert_eq!(app.view.checks_list_area, Rect::default());
        assert_eq!(app.view.output_area, Rect::default());
        assert_eq!(app.view.output_visible_lines, 0);
        assert_eq!(app.output_area_width, 0);
        app.scroll_down(5);
        app.scroll_to_bottom();
    }

    /// Help overlay lists every binding, even at the minimum terminal size
    #[test]
    fn test_help_overlay_lists_output_keys_at_min_size() {
        let mut app = make_test_app();
        app.toggle_help();
        let height = min_height(app.view.stats_visible);
        let mut terminal = Terminal::new(TestBackend::new(MIN_WIDTH, height)).expect("terminal");
        terminal.draw(|f| render(&mut app, f)).expect("draw");
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..height)
            .map(|y| (0..MIN_WIDTH).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let screen = rows.join("\n");
        for text in [
            "o / O",
            "$PAGER/$EDITOR",
            "save log file",
            "quit",
            "Esc to close",
        ] {
            assert!(screen.contains(text), "missing {text:?}:\n{screen}");
        }
    }

    #[test]
    fn test_header_title_shows_filter_notice() {
        let mut app = make_test_app();
        assert!(!header_title(&app, "1s").contains("Filtered"));
        app.filter_notice = Some("Filtered: --only php-lint".to_string());
        assert!(header_title(&app, "1s").ends_with(" │ 1s │ Filtered: --only php-lint "));
    }

    #[test]
    fn test_confirm_search_finds_matches_and_jumps_scroll() {
        let mut app = make_test_app();
        app.results.get_mut("php-lint").unwrap().output = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.set_output_visible_lines(5);
        app.output_area_width = 80;
        app.open_search();
        app.search_push('l');
        app.search_push('i');
        app.search_push('n');
        app.search_push('e');
        app.search_push(' ');
        app.search_push('1');
        app.search_push('5');

        confirm_search(&mut app);

        let search = app.view.search.as_ref().expect("search stays open");
        assert!(!search.typing);
        assert_eq!(search.matches.len(), 1, "only 'line 15' matches");
        assert!(
            app.view.output_scroll > 0,
            "scrolled to bring match into view"
        );
    }

    #[test]
    fn test_confirm_search_jump_turns_follow_off() {
        let mut app = make_test_app();
        let result = app.results.get_mut("php-lint").unwrap();
        result.status = CheckStatus::Running;
        result.output = (1..=20).map(|i| format!("line {i}\n")).collect();
        app.set_output_visible_lines(5);
        app.output_area_width = 80;
        app.open_search();
        for c in "line 3".chars() {
            app.search_push(c);
        }

        confirm_search(&mut app);
        app.follow_output_tail();

        assert!(!app.view.follow_output, "search jump turns follow off");
        let search = app.view.search.as_ref().expect("search stays open");
        assert_eq!(search.matches.len(), 1);
        assert!(app.view.output_scroll < 10, "stays at the match");
    }

    /// Streamed output content changes without changing length once the
    /// rolling cap is reached; the cache must still rebuild.
    #[test]
    fn test_output_cache_rebuilds_on_same_length_streamed_chunk() {
        let mut app = make_test_app();
        app.config.max_output_lines = 2;
        let result = app.results.get_mut("php-lint").unwrap();
        result.status = CheckStatus::Running;
        result.output.clear();
        app.handle_runner_event(output_event("php-lint", "a1\na2\na3\n", ""));
        ensure_output_cache(&mut app, 80);
        app.handle_runner_event(output_event("php-lint", "b1\nb2\n", ""));
        ensure_output_cache(&mut app, 80);

        let text = app.output_cache.as_ref().unwrap().text.clone();
        let plain: String = text.lines.iter().map(line_plain_text).collect();
        assert!(plain.contains("b2"), "got: {plain}");
    }

    /// Stderr above stdout while running, so a following panel's bottom
    /// shows the latest stdout even when stderr is long
    #[test]
    fn test_running_output_puts_stderr_above_stdout() {
        let mut app = make_test_app();
        let result = app.results.get_mut("php-lint").unwrap();
        result.status = CheckStatus::Running;
        result.output = "out1\nout2\n".to_string();
        result.error_output = "Compiling a\nCompiling b\n".to_string();
        let check = app.checks[0].clone();
        let result = app.results["php-lint"].clone();

        let (text, _) = build_check_output_text(&app, &check, &result, 80);

        let stderr_at = text.find("── stderr ──").expect("stderr section");
        assert!(stderr_at < text.find("out1").unwrap(), "got: {text}");
        assert!(text.find("Compiling b").unwrap() < text.find("out1").unwrap());
        assert!(text.trim_end().ends_with("out2"), "got: {text}");
    }

    #[test]
    fn test_failed_output_keeps_stderr_below_stdout() {
        let mut app = make_test_app();
        let result = app.results.get_mut("php-lint").unwrap();
        result.status = CheckStatus::Failed;
        result.output = "out1\n".to_string();
        result.error_output = "err1\n".to_string();
        let check = app.checks[0].clone();
        let result = app.results["php-lint"].clone();

        let (text, _) = build_check_output_text(&app, &check, &result, 80);

        assert!(text.find("out1").unwrap() < text.find("── stderr ──").unwrap());
    }

    #[test]
    fn test_confirm_search_no_match_leaves_scroll_and_empty_matches() {
        let mut app = make_test_app();
        app.set_output_visible_lines(5);
        app.output_area_width = 80;
        app.open_search();
        for c in "nope-not-there".chars() {
            app.search_push(c);
        }

        confirm_search(&mut app);

        let search = app.view.search.as_ref().expect("search stays open");
        assert!(!search.typing);
        assert!(search.matches.is_empty());
        assert_eq!(app.view.output_scroll, 0);
    }

    /// make_test_app with php-lint in `status` with stdout `output` and
    /// stderr `stderr`, cache built
    fn app_with_output(status: CheckStatus, output: &str, stderr: &str) -> App {
        let mut app = make_test_app();
        let result = app.results.get_mut("php-lint").unwrap();
        result.status = status;
        result.output = output.to_string();
        result.error_output = stderr.to_string();
        app.output_area_width = 80;
        ensure_output_cache(&mut app, 80);
        app
    }

    /// Displayed line containing `needle` (panics if none)
    fn shown_line(app: &App, needle: &str) -> Line<'static> {
        output_display_text(app, app.output_cache.as_ref().unwrap())
            .lines
            .into_iter()
            .find(|l| line_plain_text(l).contains(needle))
            .unwrap_or_else(|| panic!("no line containing {needle}"))
    }

    fn is_error_styled(line: &Line<'_>) -> bool {
        line.spans
            .iter()
            .all(|s| s.style == s.style.patch(ERROR_LINE_STYLE))
    }

    #[test]
    fn test_failed_check_highlights_error_lines_only() {
        let app = app_with_output(CheckStatus::Failed, "ok\nerror: boom\nstill ok\n", "");

        assert!(is_error_styled(&shown_line(&app, "error: boom")));
        assert!(!is_error_styled(&shown_line(&app, "still ok")));
        // Header status line "✗ FAILED" is ours, not command output
        assert!(!is_error_styled(&shown_line(&app, "FAILED")));
    }

    #[test]
    fn test_passed_check_has_no_error_highlight() {
        let app = app_with_output(CheckStatus::Passed, "ok\nerror: boom\n", "");
        assert!(!is_error_styled(&shown_line(&app, "error: boom")));
    }

    #[test]
    fn test_failed_check_error_highlight_includes_stderr() {
        let app = app_with_output(
            CheckStatus::Failed,
            "ok\n",
            "Traceback (most recent call last):\n",
        );

        assert!(is_error_styled(&shown_line(&app, "Traceback")));
    }

    #[test]
    fn test_check_error_pattern_overrides_default_highlight() {
        let mut app = app_with_output(CheckStatus::Failed, "E0308 bad\nerror: not matched\n", "");
        app.checks[0].definition.error_pattern = Some(regex::Regex::new("^E\\d+").unwrap());
        app.output_cache = None; // pattern is not part of the cache key
        ensure_output_cache(&mut app, 80);

        assert!(is_error_styled(&shown_line(&app, "E0308")));
        assert!(!is_error_styled(&shown_line(&app, "not matched")));
    }

    #[test]
    fn test_stderr_separator_is_never_an_error_line() {
        // Edge case: a pattern that also matches ci-tui's own separator
        let mut app = app_with_output(CheckStatus::Failed, "ok\n", "err1\n");
        app.checks[0].definition.error_pattern = Some(regex::Regex::new("err").unwrap());
        app.output_cache = None; // pattern is not part of the cache key
        ensure_output_cache(&mut app, 80);

        assert!(!is_error_styled(&shown_line(&app, "── stderr ──")));
        assert!(is_error_styled(&shown_line(&app, "err1")));
    }

    /// App parked on its first error line; `e` then makes the long command
    /// wrap onto more lines, moving the error line down
    fn app_parked_on_error() -> App {
        let output = format!("{}error: boom\n{}", "ok\n".repeat(10), "tail\n".repeat(60));
        let mut app = app_with_output(CheckStatus::Failed, &output, "");
        app.checks[0].resolved_command = "x".repeat(300);
        app.output_cache = None;
        ensure_output_cache(&mut app, 80);
        let row = first_error_scroll(&app).expect("error line");
        app.scroll_to_row(row);
        app
    }

    #[test]
    fn test_error_jump_follows_rebuild_while_parked() {
        let mut app = app_parked_on_error();
        let before = app.view.output_scroll;

        app.view.show_full_command = true;
        ensure_output_cache(&mut app, 80);

        let after = first_error_scroll(&app).unwrap();
        assert!(after > before, "expanded command pushes the error down");
        assert_eq!(app.view.output_scroll, after);
    }

    #[test]
    fn test_error_jump_rebuild_leaves_scrolled_away_view() {
        let mut app = app_parked_on_error();
        app.scroll_up(1);
        let scrolled = app.view.output_scroll;

        app.view.show_full_command = true;
        ensure_output_cache(&mut app, 80);

        assert_eq!(app.view.output_scroll, scrolled);
    }

    #[test]
    fn test_search_highlight_wins_over_error_highlight() {
        let mut app = app_with_output(CheckStatus::Failed, "ok\nerror: boom\nerror: other\n", "");
        app.open_search();
        for c in "boom".chars() {
            app.search_push(c);
        }
        confirm_search(&mut app);

        let searched = shown_line(&app, "boom");
        assert!(searched
            .spans
            .iter()
            .any(|s| s.content.as_ref() == "boom" && s.style.bg == Some(Color::Yellow)));
        // Unsearched error line keeps its error highlight
        assert!(is_error_styled(&shown_line(&app, "error: other")));
    }

    #[test]
    fn test_highlight_matches_splits_matching_line_into_styled_spans() {
        let text = Text::from(vec![
            Line::from("no match here"),
            Line::from("has FOO in it"),
        ]);

        let highlighted = highlight_matches(&text, &[1], "foo");

        assert_eq!(highlighted.lines.len(), 2);
        // Unrelated line is untouched
        assert_eq!(line_plain_text(&highlighted.lines[0]), "no match here");
        // Matching line is rebuilt with the match as a separate styled span
        let matched_line = &highlighted.lines[1];
        assert_eq!(line_plain_text(matched_line), "has FOO in it");
        assert!(
            matched_line
                .spans
                .iter()
                .any(|s| s.content.as_ref() == "FOO" && s.style.bg == Some(Color::Yellow)),
            "matched text gets a highlight background"
        );
    }

    #[test]
    fn test_highlight_matches_empty_query_returns_text_unchanged() {
        let text = Text::from(vec![Line::from("some text")]);
        let highlighted = highlight_matches(&text, &[0], "");
        assert_eq!(highlighted.lines.len(), 1);
        assert_eq!(line_plain_text(&highlighted.lines[0]), "some text");
    }

    #[test]
    fn test_highlight_matches_does_not_panic_on_multibyte_case_folding() {
        // U+0130 (İ) is 2 bytes but full-Unicode-lowercases to a 3-byte
        // sequence ("i̇"); ASCII-only folding must be used so `lower` and
        // `plain` stay byte-aligned, or slicing panics on a char boundary.
        let text = Text::from(vec![Line::from("İ says foo")]);
        let highlighted = highlight_matches(&text, &[0], "foo");
        assert_eq!(line_plain_text(&highlighted.lines[0]), "İ says foo");
    }
}
