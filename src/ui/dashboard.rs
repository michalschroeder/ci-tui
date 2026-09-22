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

use super::app::App;
use crate::checks::CheckFiles;
use crate::runner::CheckStatus;
use crate::utils::time;
use ansi_to_tui::IntoText;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Sparkline, Wrap},
    Frame,
};
use std::collections::VecDeque;

const GIT_HASH: &str = env!("CI_TUI_GIT_HASH");
const BUILD_DATE: &str = env!("CI_TUI_BUILD_DATE");

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
        Some(CheckStatus::Running) => ("●", Style::default().fg(Color::Yellow)),
        Some(CheckStatus::Pending) => ("○", Style::default().fg(Color::DarkGray)),
        Some(CheckStatus::Skipped) => ("⊘", Style::default().fg(Color::DarkGray)),
        Some(CheckStatus::OnDemand) => ("◇", Style::default().fg(Color::Cyan)),
        None => ("?", Style::default().fg(Color::DarkGray)),
    }
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
    let (passed, failed, pending, on_demand) = app.count_by_status();
    let total = app.checks.len();
    let auto_run_total = total - on_demand;
    let completed = passed + failed;
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
        if failed == 0 {
            format!(
                "✓ All {} checks passed in {}{}",
                auto_run_total, elapsed_str, on_demand_text
            )
        } else {
            format!(
                "✗ {}/{} passed, {} failed in {}{}",
                passed, auto_run_total, failed, elapsed_str, on_demand_text
            )
        }
    } else if pending > 0 {
        format!(
            "Running... {}/{} ({} in progress){}",
            completed, auto_run_total, pending, on_demand_text
        )
    } else {
        format!(
            "Preparing... {}/{}{}",
            completed, auto_run_total, on_demand_text
        )
    };

    let color = if app.run.all_finished {
        if failed == 0 {
            Color::Green
        } else {
            Color::Red
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
        " MEM {:.1}/{:.1}GB ({:.0}%) ",
        app.mem_used_gb(),
        app.mem_total_gb(),
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
    // Add 2 for borders, minimum 8 lines, cap at 70% of available height
    let checks_height = ((total_check_items + 2) as u16)
        .max(8)
        .min((area.height as f32 * 0.7) as u16);

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
fn render_pre_command_item(app: &App, pre_cmd: &super::app::PreCommandState) -> ListItem<'static> {
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

    let is_selected = app
        .selected_pre_command()
        .map(|p| p.group == pre_cmd.group && p.name == pre_cmd.name)
        .unwrap_or(false);

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

    let is_selected = app
        .selected_check()
        .map(|c| c.id() == check.id())
        .unwrap_or(false);

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

    // Truncate name if needed - calculate based on available width
    let available_width = area.width.saturating_sub(15) as usize;
    let max_name_len = available_width.max(20);
    let name = if check.name().len() > max_name_len {
        format!("{}…", &check.name()[..max_name_len - 1])
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

fn render_checks_list(app: &App, frame: &mut Frame, area: Rect) {
    let groups = app.groups();
    let mut items: Vec<ListItem> = Vec::new();

    for group in groups {
        let visible_pre_commands: Vec<_> = app
            .pre_commands
            .iter()
            .filter(|p| p.group == group && app.should_show_pre_command(p))
            .collect();
        let visible_checks: Vec<_> = app
            .checks_in_group(group)
            .into_iter()
            .filter(|c| app.should_show_check(c))
            .collect();

        // Hide groups with nothing to show under the current filter
        if visible_pre_commands.is_empty() && visible_checks.is_empty() {
            continue;
        }

        let group_style = if Some(group.to_string()) == app.run.current_group {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        };

        let display_name = app.group_display_name(group);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("─ {} ", display_name.to_uppercase()), group_style),
            Span::styled("───────────", Style::default().fg(Color::DarkGray)),
        ])));

        for pre_cmd in visible_pre_commands {
            items.push(render_pre_command_item(app, pre_cmd));
        }

        for check in visible_checks {
            items.push(render_check_item(app, check, area));
        }
    }

    let filter_info = match app.view.status_filter {
        super::app::StatusFilter::All => "",
        super::app::StatusFilter::Failed => " [failed]",
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Checks{} ", filter_info)),
    );

    frame.render_widget(list, area);
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
    if path.len() <= max_width {
        return path.to_string();
    }

    // Find the last path separator to split filename from directory
    if let Some(last_sep) = path.rfind('/') {
        let filename = &path[last_sep + 1..];
        let dir_path = &path[..last_sep];

        // If filename alone is too long, truncate it
        if filename.len() >= max_width {
            return format!(
                "…{}",
                &filename[filename.len().saturating_sub(max_width - 1)..]
            );
        }

        // Calculate space for directory (max_width - filename - "/" - "…")
        let dir_space = max_width.saturating_sub(filename.len() + 2);

        if dir_space < 3 {
            // Not enough space for directory, just show filename
            return filename.to_string();
        }

        // Truncate directory from the left, keeping the rightmost part
        let truncated_dir = if dir_path.len() > dir_space {
            format!("…{}", &dir_path[dir_path.len() - dir_space + 1..])
        } else {
            dir_path.to_string()
        };

        format!("{}/{}", truncated_dir, filename)
    } else {
        // No separator, just truncate from the left
        format!("…{}", &path[path.len().saturating_sub(max_width - 1)..])
    }
}

/// Render fix-all results (completed)
fn render_fix_all_results(app: &App, frame: &mut Frame, area: Rect) {
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
                .take(5)
                .map(|line| format!("  \x1b[31m{}\x1b[0m\n", line))
                .collect();
            raw_output.push_str(&error_lines);
        }
    }

    // Calculate scroll indicator
    let total_lines = raw_output.lines().count();
    let visible_lines = app.view.output_visible_lines;
    let title = if total_lines > visible_lines && visible_lines > 0 {
        let current_line = app.view.output_scroll + 1;
        format!(" Fix All Results [{}/{}] ", current_line, total_lines)
    } else {
        " Fix All Results ".to_string()
    };

    let paragraph = Paragraph::new(raw_output.into_text().unwrap_or_default())
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false })
        .scroll((app.view.output_scroll as u16, 0));
    frame.render_widget(paragraph, area);
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

/// Render single fix result
fn render_fix_result(app: &App, frame: &mut Frame, area: Rect) {
    let Some(fix_result) = app.fix.result.as_ref() else {
        // Should not reach here (called only when fix_result is Some), but handle gracefully
        return;
    };
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

    // Calculate scroll indicator
    let total_lines = raw_output.lines().count();
    let visible_lines = app.view.output_visible_lines;
    let title = if total_lines > visible_lines && visible_lines > 0 {
        let current_line = app.view.output_scroll + 1;
        format!(" Fix Result [{}/{}] ", current_line, total_lines)
    } else {
        " Fix Result ".to_string()
    };

    let paragraph = Paragraph::new(raw_output.into_text().unwrap_or_default())
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false })
        .scroll((app.view.output_scroll as u16, 0));
    frame.render_widget(paragraph, area);
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
    if app.view.show_full_command || command.len() <= max_cmd_len {
        raw_output.push_str(command);
    } else {
        raw_output.push_str(&command[..max_cmd_len.saturating_sub(3)]);
        raw_output.push_str("...\x1b[0m \x1b[90m[e=expand]");
    }
    raw_output.push_str("\x1b[0m\n\n");

    // Status header
    let status_line = match result.status {
        CheckStatus::Passed => "\x1b[32m✓ PASSED\x1b[0m",
        CheckStatus::Failed => "\x1b[31m✗ FAILED\x1b[0m",
        CheckStatus::Running => "\x1b[33m◉ RUNNING...\x1b[0m",
        CheckStatus::Pending => "\x1b[90m○ PENDING\x1b[0m",
        CheckStatus::Skipped => "\x1b[90m⊘ SKIPPED\x1b[0m",
        CheckStatus::OnDemand => "\x1b[36m◇ ON-DEMAND\x1b[0m",
    };
    raw_output.push_str(status_line);

    // Show hints
    if result.status == CheckStatus::OnDemand {
        raw_output.push_str("  \x1b[33m← press 't' to run\x1b[0m");
    } else if result.status == CheckStatus::Failed && check.has_fix() {
        raw_output.push_str("  \x1b[33m← press 'x' to fix\x1b[0m");
    }
    raw_output.push_str("\n\n");

    // Files
    append_files_section(&mut raw_output, app, check);

    // Command output
    if !result.output.is_empty() {
        raw_output.push_str(&result.output);
    }

    // Stderr for failed checks
    if result.status == CheckStatus::Failed && !result.error_output.is_empty() {
        raw_output.push_str("\n\x1b[31m── stderr ──\x1b[0m\n");
        raw_output.push_str(result.error_output.trim_end());
    }

    if result.status == CheckStatus::Pending {
        raw_output.push_str("\x1b[90mWaiting to run...\x1b[0m");
    }

    raw_output
}

/// Count the lines of the fully rendered output text for a check.
///
/// Used by [`App`] scroll clamping so the max scroll offset matches what
/// [`render_check_output`] actually draws (command line, status header,
/// files section, output, stderr).
pub fn check_output_line_count(
    app: &App,
    check: &crate::checks::CheckToRun,
    result: &crate::runner::CheckResult,
    width: u16,
) -> usize {
    build_check_output_text(app, check, result, width)
        .lines()
        .count()
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
        let display_files = if files.len() <= 3 {
            files.join(", ")
        } else {
            files.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        };
        raw_output.push_str(&format!("\x1b[90mFiles: {}\x1b[0m\n\n", display_files));
    }
}

/// Render check output details
fn render_check_output(app: &App, frame: &mut Frame, area: Rect) {
    let Some(check) = app.selected_check() else {
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

    // Calculate scroll indicator
    let total_lines = raw_output.lines().count();
    let visible_lines = app.view.output_visible_lines;
    let title = if total_lines > visible_lines && visible_lines > 0 {
        let current_line = app.view.output_scroll + 1;
        format!(" {} [{}/{}] ", check.name(), current_line, total_lines)
    } else {
        format!(" {} ", check.name())
    };

    let block = Block::default().borders(Borders::ALL).title(title);
    let paragraph = Paragraph::new(raw_output.into_text().unwrap_or_default())
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.view.output_scroll as u16, 0));
    frame.render_widget(paragraph, area);
}

/// Render pre-command output details
fn render_pre_command_output(
    app: &App,
    pre_cmd: &super::app::PreCommandState,
    frame: &mut Frame,
    area: Rect,
) {
    let base_title = &pre_cmd.name;
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

    // Calculate scroll indicator
    let total_lines = raw_output.lines().count();
    let visible_lines = app.view.output_visible_lines;
    let title = if total_lines > visible_lines && visible_lines > 0 {
        let current_line = app.view.output_scroll + 1;
        format!(" {} [{}/{}] ", base_title, current_line, total_lines)
    } else {
        format!(" {} ", base_title)
    };

    let paragraph = Paragraph::new(raw_output.into_text().unwrap_or_default())
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false })
        .scroll((app.view.output_scroll as u16, 0));
    frame.render_widget(paragraph, area);
}

fn render_output(app: &mut App, frame: &mut Frame, area: Rect) {
    // Update the visible lines for scroll calculations (subtract 2 for borders)
    app.set_output_visible_lines(area.height.saturating_sub(2) as usize);
    app.output_area_width = area.width;

    // Dispatch to appropriate sub-renderer based on state
    if !app.fix.all_results.is_empty() && !app.fix.all_running {
        render_fix_all_results(app, frame, area);
    } else if app.fix.all_running {
        render_fix_all_running(app, frame, area);
    } else if app.fix.result.is_some() {
        render_fix_result(app, frame, area);
    } else if app.fix.running {
        render_fix_running(frame, area);
    } else if let Some(pre_cmd) = app.selected_pre_command() {
        render_pre_command_output(app, pre_cmd, frame, area);
    } else {
        render_check_output(app, frame, area);
    }
}

/// Build keyboard shortcut spans for the footer
fn build_footer_shortcuts(app: &App) -> Vec<Span<'static>> {
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

    if app.can_trigger_selected() {
        spans.push(Span::styled(
            "t",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" run test", Style::default().fg(Color::Cyan)));
        spans.push(Span::raw("  "));
    }

    if app.can_retry_selected() {
        spans.push(Span::styled("r", Style::default().fg(Color::Cyan)));
        spans.push(Span::raw(" retry  "));
    }

    if app.can_run_all_files() {
        spans.push(Span::styled(
            "A",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" all files  "));
    }

    spans.push(Span::styled(
        "R",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" RETRY ALL  "));

    if app.can_fix_selected() {
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
        let status_line = Line::from(vec![
            Span::styled(
                " ℹ ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg.clone(), Style::default().fg(Color::White)),
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
