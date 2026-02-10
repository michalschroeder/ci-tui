//! Application state management for the TUI.
//!
//! This module contains the [`App`] struct which holds all state needed to
//! render the UI and track check execution. It handles navigation, filtering,
//! fix operations, and system stats tracking.
//!
//! # Key Types
//!
//! - [`App`]: Main application state container
//! - [`StatusFilter`]: Filter for displaying checks by status
//! - [`PreCommandState`]: State tracking for pre-commands

use crate::checks::CheckToRun;
use crate::config::CiConfig;
use crate::git::ChangedFiles;
use crate::runner::{CheckResult, CheckStatus, RunnerEvent};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Maximum number of samples to keep in history for sparklines
const MAX_HISTORY_SAMPLES: usize = 60;

/// Filter for which checks to display in the UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    /// Show all checks
    All,
    /// Show only failed checks
    Failed,
}

/// Messages for state updates - all mutations flow through update()
#[derive(Debug)]
pub enum AppMessage {
    /// Navigation: next/previous check, scroll up/down
    NextCheck,
    PreviousCheck,
    ScrollUp(usize),
    ScrollDown(usize),
    /// Filtering
    ToggleFailedFilter,
    ShowAll,
    /// UI toggles
    ToggleFullCommand,
    /// Runner events from check execution
    RunnerEvent(crate::runner::RunnerEvent),
    /// System stats from background worker
    SystemStats {
        cpu_usage: f32,
        mem_used: u64,
        mem_total: u64,
    },
    /// Fix operations
    StartFix,
    FinishFix(crate::runner::CheckResult),
    StartFixAll(usize),
    AddFixAllResult(crate::runner::CheckResult),
    FinishFixAll,
    /// Retry/trigger operations
    TriggerOnDemand(String),
    ResetForRetry(String),
    /// Retry result from async task
    RetryResult(crate::runner::CheckResult),
    /// Status message
    SetStatusMessage(Option<String>),
    /// Clear status on any key
    ClearStatusMessage,
}

/// Status of a pre-command
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreCommandStatus {
    Pending,
    Running,
    Passed,
    Failed,
}

/// State of a pre-command for UI display
#[derive(Debug, Clone)]
pub struct PreCommandState {
    pub group: String,
    pub name: String,
    pub status: PreCommandStatus,
    pub output: String,
    pub duration_ms: u64,
}

/// Represents an item that can be selected in the checks list
#[derive(Debug, Clone)]
pub enum SelectableItem<'a> {
    PreCommand(&'a PreCommandState),
    Check(&'a CheckToRun),
}

/// Build initial CheckResult for a check based on its state
fn initial_result_for_check(check: &CheckToRun) -> CheckResult {
    if check.skipped_no_files {
        CheckResult::skipped(check.id())
    } else if check.on_demand {
        CheckResult::on_demand(check.id())
    } else {
        CheckResult::pending(check.id())
    }
}

/// Build pre-command state list from config for active groups
fn build_pre_commands(
    config: &CiConfig,
    active_groups: &std::collections::HashSet<&str>,
) -> Vec<PreCommandState> {
    let mut pre_commands = Vec::new();
    for (group_name, group_config) in config.groups() {
        if !active_groups.contains(group_name) {
            continue;
        }
        for pre_cmd in &group_config.pre_commands {
            pre_commands.push(PreCommandState {
                group: group_name.to_string(),
                name: pre_cmd.name.clone(),
                status: PreCommandStatus::Pending,
                output: String::new(),
                duration_ms: 0,
            });
        }
    }
    pre_commands
}

/// Application state for the TUI
///
/// Contains all data needed to render the UI and track check execution,
/// including check results, system stats, and UI state.
pub struct App {
    /// CI configuration loaded from YAML
    pub config: CiConfig,
    /// Files changed compared to base branch
    pub changed_files: ChangedFiles,
    /// Checks to be executed
    pub checks: Vec<CheckToRun>,
    /// Results indexed by check ID
    pub results: HashMap<String, CheckResult>,
    /// Current git branch name
    pub current_branch: String,

    // UI state
    /// Index of currently selected check in filtered list
    pub selected_check: usize,
    /// Scroll position in output panel
    pub output_scroll: usize,
    /// Number of visible lines in output area (updated during render)
    pub output_visible_lines: usize,
    /// Current filter for check list
    pub status_filter: StatusFilter,

    // Runner state
    /// Currently executing group name
    pub current_group: Option<String>,
    /// True when all checks have completed
    pub all_finished: bool,
    /// When the run started
    pub run_started_at: Option<Instant>,
    /// When the run finished
    pub run_finished_at: Option<Instant>,

    // Pre-command state
    /// Pre-commands and their status (group, name, status, output)
    pub pre_commands: Vec<PreCommandState>,
    /// Currently running pre-command index
    pub current_pre_command: Option<usize>,

    // Fix state
    /// True while a fix command is running
    pub fix_running: bool,
    /// Result of the last fix command
    pub fix_result: Option<CheckResult>,
    /// True while fix-all is running
    pub fix_all_running: bool,
    /// Results from fix-all operation
    pub fix_all_results: Vec<CheckResult>,
    /// Total number of fixes in fix-all
    pub fix_all_total: usize,

    // System monitoring (updated by background task)
    /// CPU usage history for sparkline (percentage values)
    pub cpu_history: VecDeque<f32>,
    /// Memory usage history for sparkline (percentage values)
    pub mem_history: VecDeque<f32>,
    /// Current memory usage in bytes
    pub mem_used_bytes: u64,
    /// Total memory in bytes
    pub mem_total_bytes: u64,

    /// Status message shown to user (clears on next keypress)
    pub status_message: Option<String>,

    /// Whether to show full command in output panel
    pub show_full_command: bool,

    /// Dirty flag - set when state changes, cleared after render
    /// Used to avoid unnecessary re-renders for better responsiveness
    pub needs_redraw: bool,
}

impl App {
    pub fn new(
        config: CiConfig,
        changed_files: ChangedFiles,
        checks: Vec<CheckToRun>,
        current_branch: String,
    ) -> Self {
        // Initialize results - skipped for checks with {files} and no matches,
        // on_demand for manual triggers, pending for auto-run
        let results: HashMap<String, CheckResult> = checks
            .iter()
            .map(|check| (check.id().to_string(), initial_result_for_check(check)))
            .collect();

        // Build pre-commands list from config (only for groups that have checks)
        let active_groups: std::collections::HashSet<&str> =
            checks.iter().map(|c| c.group()).collect();
        let pre_commands = build_pre_commands(&config, &active_groups);

        Self {
            config,
            changed_files,
            checks,
            results,
            current_branch,
            selected_check: 0,
            output_scroll: 0,
            output_visible_lines: 20,
            status_filter: StatusFilter::All,
            current_group: None,
            all_finished: false,
            run_started_at: Some(Instant::now()),
            run_finished_at: None,
            pre_commands,
            current_pre_command: None,
            fix_running: false,
            fix_result: None,
            fix_all_running: false,
            fix_all_results: Vec::new(),
            fix_all_total: 0,
            cpu_history: VecDeque::with_capacity(64),
            mem_history: VecDeque::with_capacity(64),
            mem_used_bytes: 0,
            mem_total_bytes: 1, // Avoid division by zero
            status_message: None,
            show_full_command: false,
            needs_redraw: true, // Initial render needed
        }
    }

    /// Reset for retry - update changed files and checks, reset results
    pub fn reset_for_retry(&mut self, changed_files: ChangedFiles, checks: Vec<CheckToRun>) {
        self.changed_files = changed_files;
        self.checks = checks.clone();

        // Reset results - skipped for checks with {files} and no matches,
        // on_demand for manual triggers, pending for auto-run
        self.results = checks
            .iter()
            .map(|check| (check.id().to_string(), initial_result_for_check(check)))
            .collect();

        // Reset pre-commands
        let active_groups: std::collections::HashSet<&str> =
            checks.iter().map(|c| c.group()).collect();
        self.pre_commands = build_pre_commands(&self.config, &active_groups);

        // Reset state
        self.selected_check = 0;
        self.output_scroll = 0;
        self.output_visible_lines = 20;
        self.current_group = None;
        self.all_finished = false;
        self.run_started_at = Some(Instant::now());
        self.run_finished_at = None;
        self.current_pre_command = None;
        self.fix_running = false;
        self.fix_result = None;
        self.fix_all_running = false;
        self.fix_all_results = Vec::new();
        self.fix_all_total = 0;
        self.status_message = None;
        self.show_full_command = false;
        self.needs_redraw = true;
    }

    /// Get total elapsed time
    pub fn elapsed_time(&self) -> std::time::Duration {
        let Some(started) = self.run_started_at else {
            return std::time::Duration::ZERO;
        };
        self.run_finished_at
            .map(|finished| finished.duration_since(started))
            .unwrap_or_else(|| started.elapsed())
    }

    /// Check if selected check can be fixed
    pub fn can_fix_selected(&self) -> bool {
        if self.fix_running {
            return false;
        }
        let Some(check) = self.selected_check() else {
            return false;
        };
        let Some(result) = self.results.get(check.id()) else {
            return false;
        };
        result.status == CheckStatus::Failed && check.has_fix()
    }

    /// Get fix command and service for selected check
    /// Returns (fix_command, service, container) for the selected check
    pub fn get_selected_fix_command(&self) -> Option<(String, String, Option<String>)> {
        let check = self.selected_check()?;
        let cmd = check.resolved_fix_command.as_ref()?;
        Some((
            cmd.clone(),
            check.service.clone(),
            check.definition.container.clone(),
        ))
    }

    /// Mark fix as started
    pub fn start_fix(&mut self) {
        self.fix_running = true;
        self.fix_result = None;
        self.needs_redraw = true;
    }

    /// Store fix result
    pub fn finish_fix(&mut self, result: CheckResult) {
        self.fix_running = false;
        self.fix_result = Some(result);
        self.needs_redraw = true;
    }

    fn is_fixable(&self, check: &CheckToRun) -> bool {
        self.results
            .get(check.id())
            .is_some_and(|r| r.status == CheckStatus::Failed && check.has_fix())
    }

    /// Get all failed checks that can be fixed
    pub fn get_fixable_checks(&self) -> Vec<&CheckToRun> {
        self.checks
            .iter()
            .filter(|check| self.is_fixable(check))
            .collect()
    }

    /// Check if there are any checks that can be fixed
    pub fn can_fix_all(&self) -> bool {
        if self.fix_running || self.fix_all_running {
            return false;
        }
        !self.get_fixable_checks().is_empty()
    }

    /// Get fix commands for all fixable checks (check_id, fix_command, service)
    /// Returns Vec of (check_id, fix_command, service, container) for all fixable checks
    pub fn get_all_fix_commands(&self) -> Vec<(String, String, String, Option<String>)> {
        self.get_fixable_checks()
            .iter()
            .filter_map(|check| {
                let cmd = check.resolved_fix_command.as_ref()?;
                Some((
                    check.id().to_string(),
                    cmd.clone(),
                    check.service.clone(),
                    check.definition.container.clone(),
                ))
            })
            .collect()
    }

    /// Start fix-all operation
    pub fn start_fix_all(&mut self, total: usize) {
        self.fix_all_running = true;
        self.fix_all_results = Vec::new();
        self.fix_all_total = total;
        self.fix_result = None;
        self.needs_redraw = true;
    }

    /// Add a result from fix-all
    pub fn add_fix_all_result(&mut self, result: CheckResult) {
        self.fix_all_results.push(result);
        self.needs_redraw = true;
    }

    /// Finish fix-all operation
    pub fn finish_fix_all(&mut self) {
        self.fix_all_running = false;
        self.needs_redraw = true;
    }

    /// Check if selected check can be retried
    pub fn can_retry_selected(&self) -> bool {
        if self.fix_running || self.fix_all_running {
            return false;
        }
        let Some(check) = self.selected_check() else {
            return false;
        };
        let Some(result) = self.results.get(check.id()) else {
            return false;
        };
        result.status == CheckStatus::Passed || result.status == CheckStatus::Failed
    }

    /// Check if selected check is on-demand and can be triggered
    pub fn can_trigger_selected(&self) -> bool {
        if self.fix_running || self.fix_all_running {
            return false;
        }
        let Some(check) = self.selected_check() else {
            return false;
        };
        let Some(result) = self.results.get(check.id()) else {
            return false;
        };
        result.status == CheckStatus::OnDemand
    }

    /// Check if selected check can be run with all files (no filtering)
    pub fn can_run_all_files(&self) -> bool {
        if self.fix_running || self.fix_all_running {
            return false;
        }
        let Some(check) = self.selected_check() else {
            return false;
        };
        let Some(result) = self.results.get(check.id()) else {
            return false;
        };
        result.status == CheckStatus::Passed
            || result.status == CheckStatus::Failed
            || result.status == CheckStatus::OnDemand
    }

    /// Mark an on-demand check as running (preparing to execute)
    pub fn trigger_on_demand_check(&mut self, check_id: &str) {
        if let Some(result) = self.results.get_mut(check_id) {
            result.status = CheckStatus::Running;
            result.output.clear();
            result.error_output.clear();
            result.duration_ms = 0;
            result.started_at = Some(chrono::Local::now());
            result.finished_at = None;
        }
        self.needs_redraw = true;
    }

    /// Reset a single check to pending for retry
    pub fn reset_check_for_retry(&mut self, check_id: &str) {
        if let Some(result) = self.results.get_mut(check_id) {
            result.status = CheckStatus::Running;
            result.output.clear();
            result.error_output.clear();
            result.duration_ms = 0;
            result.started_at = Some(chrono::Local::now());
            result.finished_at = None;
        }
        // Clear any fix results
        self.fix_result = None;
        self.fix_all_results.clear();
        self.needs_redraw = true;
    }

    /// Update system stats from background task data
    ///
    /// This is called when the stats background worker sends new data.
    /// The actual sysinfo queries happen in a separate task to avoid
    /// blocking the UI thread.
    pub fn update_stats(&mut self, cpu_usage: f32, mem_used: u64, mem_total: u64) {
        self.mem_used_bytes = mem_used;
        self.mem_total_bytes = mem_total;

        // Memory usage percentage
        let mem_usage = if mem_total > 0 {
            (mem_used as f32 / mem_total as f32) * 100.0
        } else {
            0.0
        };

        // Add to history (keep last 60 samples) - VecDeque for O(1) pop_front
        self.cpu_history.push_back(cpu_usage);
        if self.cpu_history.len() > MAX_HISTORY_SAMPLES {
            self.cpu_history.pop_front();
        }

        self.mem_history.push_back(mem_usage);
        if self.mem_history.len() > MAX_HISTORY_SAMPLES {
            self.mem_history.pop_front();
        }

        self.needs_redraw = true;
    }

    pub fn cpu_usage(&self) -> f32 {
        self.cpu_history.back().copied().unwrap_or(0.0)
    }

    pub fn mem_usage(&self) -> f32 {
        self.mem_history.back().copied().unwrap_or(0.0)
    }

    pub fn mem_used_gb(&self) -> f64 {
        self.mem_used_bytes as f64 / 1_073_741_824.0
    }

    pub fn mem_total_gb(&self) -> f64 {
        self.mem_total_bytes as f64 / 1_073_741_824.0
    }

    pub fn handle_runner_event(&mut self, event: RunnerEvent) {
        self.needs_redraw = true;
        match event {
            RunnerEvent::CheckStarted { check_id } => self.on_check_started(&check_id),
            RunnerEvent::CheckOutput { check_id, line } => self.on_check_output(&check_id, &line),
            RunnerEvent::CheckFinished { result } => {
                self.results.insert(result.check_id.clone(), result);
            }
            RunnerEvent::GroupStarted { group } => {
                self.current_group = Some(group);
            }
            RunnerEvent::GroupFinished { .. } => {}
            RunnerEvent::PreCommandStarted { group, name } => {
                self.on_pre_command_started(&group, &name);
            }
            RunnerEvent::PreCommandFinished {
                group,
                name,
                success,
                output,
                duration_ms,
            } => {
                self.on_pre_command_finished(&group, &name, success, output, duration_ms);
            }
            RunnerEvent::AllFinished => {
                self.all_finished = true;
                self.current_group = None;
                self.status_message = None;
                self.run_finished_at = Some(Instant::now());
            }
        }
    }

    fn on_check_started(&mut self, check_id: &str) {
        if let Some(result) = self.results.get_mut(check_id) {
            result.status = CheckStatus::Running;
            result.started_at = Some(chrono::Local::now());
        }
    }

    fn on_check_output(&mut self, check_id: &str, line: &str) {
        if let Some(result) = self.results.get_mut(check_id) {
            result.output.push_str(line);
            result.output.push('\n');
        }
    }

    fn on_pre_command_started(&mut self, group: &str, name: &str) {
        if let Some(idx) = self
            .pre_commands
            .iter()
            .position(|p| p.group == group && p.name == name)
        {
            self.pre_commands[idx].status = PreCommandStatus::Running;
            self.current_pre_command = Some(idx);
        }
    }

    fn on_pre_command_finished(
        &mut self,
        group: &str,
        name: &str,
        success: bool,
        output: String,
        duration_ms: u64,
    ) {
        self.current_pre_command = None;
        let Some(idx) = self
            .pre_commands
            .iter()
            .position(|p| p.group == group && p.name == name)
        else {
            return;
        };
        self.pre_commands[idx].status = if success {
            PreCommandStatus::Passed
        } else {
            PreCommandStatus::Failed
        };
        self.pre_commands[idx].output = output;
        self.pre_commands[idx].duration_ms = duration_ms;
    }

    pub fn selected_check(&self) -> Option<&CheckToRun> {
        match self.selected_item() {
            Some(SelectableItem::Check(check)) => {
                // Return reference from self, not from temporary
                self.checks.iter().find(|c| c.id() == check.id())
            }
            _ => None,
        }
    }

    /// Central state update function - all mutations flow through here
    ///
    /// This implements a TEA-lite pattern where state changes are dispatched
    /// through explicit message variants, enabling predictable state transitions
    /// and easier testing.
    pub fn update(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::NextCheck => self.next_check_internal(),
            AppMessage::PreviousCheck => self.previous_check_internal(),
            AppMessage::ScrollUp(n) => self.scroll_up_internal(n),
            AppMessage::ScrollDown(n) => self.scroll_down_internal(n),
            AppMessage::ToggleFailedFilter => self.toggle_failed_filter_internal(),
            AppMessage::ShowAll => self.show_all_internal(),
            AppMessage::ToggleFullCommand => self.toggle_full_command_internal(),
            AppMessage::RunnerEvent(event) => self.handle_runner_event(event),
            AppMessage::SystemStats {
                cpu_usage,
                mem_used,
                mem_total,
            } => {
                self.update_stats(cpu_usage, mem_used, mem_total);
            }
            AppMessage::StartFix => self.start_fix(),
            AppMessage::FinishFix(result) => self.finish_fix(result),
            AppMessage::StartFixAll(total) => self.start_fix_all(total),
            AppMessage::AddFixAllResult(result) => self.add_fix_all_result(result),
            AppMessage::FinishFixAll => self.finish_fix_all(),
            AppMessage::TriggerOnDemand(check_id) => self.trigger_on_demand_check(&check_id),
            AppMessage::ResetForRetry(check_id) => self.reset_check_for_retry(&check_id),
            AppMessage::RetryResult(result) => {
                self.results.insert(result.check_id.clone(), result);
                self.needs_redraw = true;
            }
            AppMessage::SetStatusMessage(msg) => {
                self.status_message = msg;
                self.needs_redraw = true;
            }
            AppMessage::ClearStatusMessage => {
                self.status_message = None;
            }
        }
        // Automatic dirty flag - every state change triggers redraw
        // (some handlers already set this, but redundant sets are harmless)
        self.needs_redraw = true;
    }

    // Navigation - all methods set needs_redraw for immediate visual feedback

    /// Internal navigation method - does not set needs_redraw (handled by update())
    fn next_check_internal(&mut self) {
        // Clear fix results and reset command view when navigating
        self.fix_result = None;
        self.fix_all_results.clear();
        self.output_scroll = 0;
        self.show_full_command = false;

        let max = self.get_selectable_items().len().saturating_sub(1);
        if self.selected_check < max {
            self.selected_check += 1;
        }
    }

    pub fn next_check(&mut self) {
        self.update(AppMessage::NextCheck);
    }

    /// Internal navigation method - does not set needs_redraw (handled by update())
    fn previous_check_internal(&mut self) {
        // Clear fix results and reset command view when navigating
        self.fix_result = None;
        self.fix_all_results.clear();
        self.output_scroll = 0;
        self.show_full_command = false;

        if self.selected_check > 0 {
            self.selected_check -= 1;
        }
    }

    pub fn previous_check(&mut self) {
        self.update(AppMessage::PreviousCheck);
    }

    /// Internal scroll method - does not set needs_redraw (handled by update())
    fn scroll_up_internal(&mut self, n: usize) {
        self.output_scroll = self.output_scroll.saturating_sub(n);
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.update(AppMessage::ScrollUp(n));
    }

    /// Internal scroll method - does not set needs_redraw (handled by update())
    fn scroll_down_internal(&mut self, n: usize) {
        let max_scroll = self.compute_max_scroll();
        self.output_scroll = (self.output_scroll + n).min(max_scroll);
    }

    /// Compute maximum scroll offset for the currently selected item's output
    fn compute_max_scroll(&self) -> usize {
        let visible = self.output_visible_lines;
        match self.selected_item() {
            Some(SelectableItem::Check(check)) => {
                self.check_total_lines(check.id()).saturating_sub(visible)
            }
            Some(SelectableItem::PreCommand(pc)) => {
                pc.output.lines().count().saturating_sub(visible)
            }
            None => 0,
        }
    }

    /// Count total output lines for a check (stdout + stderr + header)
    fn check_total_lines(&self, check_id: &str) -> usize {
        let Some(result) = self.results.get(check_id) else {
            return 0;
        };
        let stdout_lines = result.output.lines().count();
        let stderr_lines = result.error_output.lines().count();
        if stderr_lines > 0 {
            stdout_lines + stderr_lines + 2
        } else {
            stdout_lines
        }
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.update(AppMessage::ScrollDown(n));
    }

    /// Set the number of visible lines in output area (called during render)
    pub fn set_output_visible_lines(&mut self, lines: usize) {
        self.output_visible_lines = lines;
    }

    /// Internal filter method - does not set needs_redraw (handled by update())
    fn toggle_failed_filter_internal(&mut self) {
        self.status_filter = match self.status_filter {
            StatusFilter::All => StatusFilter::Failed,
            StatusFilter::Failed => StatusFilter::All,
        };
        self.selected_check = 0;
    }

    pub fn toggle_failed_filter(&mut self) {
        self.update(AppMessage::ToggleFailedFilter);
    }

    /// Internal filter method - does not set needs_redraw (handled by update())
    fn show_all_internal(&mut self) {
        self.status_filter = StatusFilter::All;
        self.selected_check = 0;
    }

    pub fn show_all(&mut self) {
        self.update(AppMessage::ShowAll);
    }

    /// Internal toggle method - does not set needs_redraw (handled by update())
    fn toggle_full_command_internal(&mut self) {
        self.show_full_command = !self.show_full_command;
    }

    pub fn toggle_full_command(&mut self) {
        self.update(AppMessage::ToggleFullCommand);
    }

    // Stats
    pub fn count_by_status(&self) -> (usize, usize, usize, usize) {
        self.results
            .values()
            .fold((0, 0, 0, 0), |(p, f, pe, o), r| match r.status {
                CheckStatus::Passed => (p + 1, f, pe, o),
                CheckStatus::Failed => (p, f + 1, pe, o),
                CheckStatus::Pending | CheckStatus::Running => (p, f, pe + 1, o),
                CheckStatus::OnDemand => (p, f, pe, o + 1),
                CheckStatus::Skipped => (p, f, pe, o),
            })
    }

    pub fn groups(&self) -> Vec<&str> {
        // Get groups that have checks, in config order (IndexMap preserves YAML order)
        let active_groups: std::collections::HashSet<&str> =
            self.checks.iter().map(|c| c.group()).collect();

        // Return groups in config order, filtered to only those with checks
        self.config
            .groups()
            .map(|(key, _)| key)
            .filter(|g| active_groups.contains(g))
            .collect()
    }

    pub fn checks_in_group(&self, group: &str) -> Vec<&CheckToRun> {
        self.checks.iter().filter(|c| c.group() == group).collect()
    }

    /// Get the display name for a group (uses custom name if set, otherwise the key)
    pub fn group_display_name<'a>(&'a self, group_key: &'a str) -> &'a str {
        self.config
            .get_group(group_key)
            .map(|g| g.display_name(group_key))
            .unwrap_or(group_key)
    }

    /// Get all selectable items in display order (pre-commands + checks, grouped)
    pub fn get_selectable_items(&self) -> Vec<SelectableItem<'_>> {
        let mut items = Vec::new();

        for group in self.groups() {
            items.extend(
                self.pre_commands
                    .iter()
                    .filter(|p| p.group == group && self.should_show_pre_command(p))
                    .map(SelectableItem::PreCommand),
            );
            items.extend(
                self.checks_in_group(group)
                    .into_iter()
                    .filter(|c| self.should_show_check(c))
                    .map(SelectableItem::Check),
            );
        }

        items
    }

    fn should_show_pre_command(&self, pre_cmd: &PreCommandState) -> bool {
        match self.status_filter {
            StatusFilter::All => true,
            StatusFilter::Failed => pre_cmd.status == PreCommandStatus::Failed,
        }
    }

    fn should_show_check(&self, check: &CheckToRun) -> bool {
        match self.status_filter {
            StatusFilter::All => true,
            StatusFilter::Failed => self
                .results
                .get(check.id())
                .map(|r| r.status == CheckStatus::Failed)
                .unwrap_or(false),
        }
    }

    /// Get the currently selected item (pre-command or check)
    pub fn selected_item(&self) -> Option<SelectableItem<'_>> {
        self.get_selectable_items()
            .into_iter()
            .nth(self.selected_check)
    }

    /// Get the selected pre-command, if one is selected
    pub fn selected_pre_command(&self) -> Option<&PreCommandState> {
        match self.selected_item() {
            Some(SelectableItem::PreCommand(pc)) => {
                // Need to return reference from self, not from the temporary
                self.pre_commands
                    .iter()
                    .find(|p| p.group == pc.group && p.name == pc.name)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckToRun;
    use crate::config::{CheckDefinition, CiConfig};
    use crate::git::ChangedFiles;
    use crate::runner::{CheckResult, CheckStatus};

    fn minimal_config_yaml() -> &'static str {
        r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php
  shell: bash

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  php:
    pattern: '\.php$'

checks:
  fast:
    name: Fast Checks
    parallel: true
    checks:
      php-lint:
        name: PHP syntax check
        command: php-lint {files}
        triggers:
          file_pattern: php

  tests:
    checks:
      phpunit:
        name: PHPUnit Tests
        command: phpunit {files}
        fix_command: phpunit --fix {files}
        triggers:
          file_pattern: php
"#
    }

    fn parse_config() -> CiConfig {
        serde_yaml::from_str(minimal_config_yaml()).expect("Failed to parse config")
    }

    fn make_check(id: &str, group: &str, name: &str, has_fix: bool, on_demand: bool) -> CheckToRun {
        CheckToRun {
            id: id.to_string(),
            group: group.to_string(),
            definition: CheckDefinition {
                name: name.to_string(),
                command: format!("{} {{files}}", id),
                service: None,
                container: None,
                fix_command: if has_fix {
                    Some(format!("{} --fix {{files}}", id))
                } else {
                    None
                },
                triggers: None,
                on_demand: false,
                env: std::collections::HashMap::new(),
            },
            service: "php".to_string(),
            files: vec!["test.php".to_string()],
            resolved_command: format!("{} test.php", id),
            resolved_fix_command: if has_fix {
                Some(format!("{} --fix test.php", id))
            } else {
                None
            },
            on_demand,
            skipped_no_files: false,
        }
    }

    fn make_app() -> App {
        let config = parse_config();
        let changed_files = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: "development".to_string(),
        };
        let checks = vec![
            make_check("php-lint", "fast", "PHP Lint", false, false),
            make_check("phpunit", "tests", "PHPUnit", true, false),
            make_check("behat", "tests", "Behat", false, true),
        ];
        App::new(config, changed_files, checks, "main".to_string())
    }

    #[test]
    fn test_new_initializes_results() {
        let app = make_app();

        // All checks should have results
        assert_eq!(app.results.len(), 3);

        // Non-on-demand checks should be pending
        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            CheckStatus::Pending
        );
        assert_eq!(
            app.results.get("phpunit").unwrap().status,
            CheckStatus::Pending
        );

        // On-demand checks should be on_demand
        assert_eq!(
            app.results.get("behat").unwrap().status,
            CheckStatus::OnDemand
        );
    }

    #[test]
    fn test_new_initial_state() {
        let app = make_app();

        assert_eq!(app.selected_check, 0);
        assert_eq!(app.output_scroll, 0);
        assert_eq!(app.status_filter, StatusFilter::All);
        assert!(!app.all_finished);
        assert!(!app.fix_running);
        assert!(app.needs_redraw);
    }

    #[test]
    fn test_navigation_next() {
        let mut app = make_app();

        assert_eq!(app.selected_check, 0);
        app.next_check();
        assert_eq!(app.selected_check, 1);
        app.next_check();
        assert_eq!(app.selected_check, 2);

        // Should not go past last item
        app.next_check();
        assert_eq!(app.selected_check, 2);
    }

    #[test]
    fn test_navigation_previous() {
        let mut app = make_app();
        app.selected_check = 2;

        app.previous_check();
        assert_eq!(app.selected_check, 1);
        app.previous_check();
        assert_eq!(app.selected_check, 0);

        // Should not go below 0
        app.previous_check();
        assert_eq!(app.selected_check, 0);
    }

    #[test]
    fn test_navigation_clears_fix_result() {
        let mut app = make_app();
        app.fix_result = Some(CheckResult::pending("test"));

        app.next_check();

        assert!(app.fix_result.is_none());
    }

    #[test]
    fn test_toggle_failed_filter() {
        let mut app = make_app();

        assert_eq!(app.status_filter, StatusFilter::All);
        app.toggle_failed_filter();
        assert_eq!(app.status_filter, StatusFilter::Failed);
        app.toggle_failed_filter();
        assert_eq!(app.status_filter, StatusFilter::All);
    }

    #[test]
    fn test_show_all_resets_filter() {
        let mut app = make_app();
        app.status_filter = StatusFilter::Failed;
        app.selected_check = 5;

        app.show_all();

        assert_eq!(app.status_filter, StatusFilter::All);
        assert_eq!(app.selected_check, 0);
    }

    #[test]
    fn test_count_by_status() {
        let mut app = make_app();

        // Initial state: 2 pending, 1 on-demand
        let (passed, failed, pending, on_demand) = app.count_by_status();
        assert_eq!(passed, 0);
        assert_eq!(failed, 0);
        assert_eq!(pending, 2);
        assert_eq!(on_demand, 1);

        // Mark one as passed
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Passed;

        let (passed, failed, pending, on_demand) = app.count_by_status();
        assert_eq!(passed, 1);
        assert_eq!(failed, 0);
        assert_eq!(pending, 1);
        assert_eq!(on_demand, 1);

        // Mark one as failed
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        let (passed, failed, pending, on_demand) = app.count_by_status();
        assert_eq!(passed, 1);
        assert_eq!(failed, 1);
        assert_eq!(pending, 0);
        assert_eq!(on_demand, 1);
    }

    #[test]
    fn test_can_fix_selected_requires_failed_and_fix_command() {
        let mut app = make_app();

        // Check phpunit which has fix command
        app.selected_check = 1; // phpunit

        // Not failed yet - can't fix
        assert!(!app.can_fix_selected());

        // Mark as failed
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        // Now can fix
        assert!(app.can_fix_selected());
    }

    #[test]
    fn test_can_fix_selected_no_fix_command() {
        let mut app = make_app();

        // Select php-lint which has no fix command
        app.selected_check = 0; // php-lint
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;

        // Can't fix because no fix command
        assert!(!app.can_fix_selected());
    }

    #[test]
    fn test_can_fix_selected_disabled_during_fix() {
        let mut app = make_app();
        app.selected_check = 1; // phpunit
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        app.fix_running = true;

        assert!(!app.can_fix_selected());
    }

    #[test]
    fn test_can_trigger_selected() {
        let mut app = make_app();

        // Select behat which is on-demand
        app.selected_check = 2;

        assert!(app.can_trigger_selected());

        // Select php-lint which is pending
        app.selected_check = 0;
        assert!(!app.can_trigger_selected());
    }

    #[test]
    fn test_can_retry_selected() {
        let mut app = make_app();
        app.selected_check = 0; // php-lint

        // Can't retry pending
        assert!(!app.can_retry_selected());

        // Can retry passed
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Passed;
        assert!(app.can_retry_selected());

        // Can retry failed
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;
        assert!(app.can_retry_selected());
    }

    #[test]
    fn test_start_and_finish_fix() {
        let mut app = make_app();

        app.start_fix();
        assert!(app.fix_running);
        assert!(app.fix_result.is_none());

        let result = CheckResult {
            check_id: "phpunit".to_string(),
            status: CheckStatus::Passed,
            output: "Fixed!".to_string(),
            error_output: String::new(),
            duration_ms: 100,
            started_at: None,
            finished_at: None,
        };

        app.finish_fix(result);
        assert!(!app.fix_running);
        assert!(app.fix_result.is_some());
    }

    #[test]
    fn test_get_fixable_checks() {
        let mut app = make_app();

        // No failed checks - empty
        assert!(app.get_fixable_checks().is_empty());

        // phpunit failed with fix command
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        let fixable = app.get_fixable_checks();
        assert_eq!(fixable.len(), 1);
        assert_eq!(fixable[0].id(), "phpunit");

        // php-lint failed but no fix command
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;

        let fixable = app.get_fixable_checks();
        assert_eq!(fixable.len(), 1); // Still just phpunit
    }

    #[test]
    fn test_update_stats() {
        let mut app = make_app();

        app.update_stats(50.0, 8_000_000_000, 16_000_000_000);

        assert_eq!(app.cpu_usage(), 50.0);
        assert_eq!(app.mem_usage(), 50.0); // 8/16 = 50%
        assert_eq!(app.mem_used_bytes, 8_000_000_000);
        assert_eq!(app.mem_total_bytes, 16_000_000_000);
    }

    #[test]
    fn test_update_stats_history_limit() {
        let mut app = make_app();

        // Add more samples than MAX_HISTORY_SAMPLES
        for i in 0..70 {
            app.update_stats(i as f32, 1000, 2000);
        }

        // Should be capped at MAX_HISTORY_SAMPLES
        assert_eq!(app.cpu_history.len(), MAX_HISTORY_SAMPLES);
        assert_eq!(app.mem_history.len(), MAX_HISTORY_SAMPLES);

        // Most recent should be 69
        assert_eq!(app.cpu_usage(), 69.0);
    }

    #[test]
    fn test_handle_runner_event_check_started() {
        let mut app = make_app();

        app.handle_runner_event(RunnerEvent::CheckStarted {
            check_id: "php-lint".to_string(),
        });

        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            CheckStatus::Running
        );
    }

    #[test]
    fn test_handle_runner_event_check_output() {
        let mut app = make_app();

        app.handle_runner_event(RunnerEvent::CheckOutput {
            check_id: "php-lint".to_string(),
            line: "Checking file...".to_string(),
        });

        assert!(app
            .results
            .get("php-lint")
            .unwrap()
            .output
            .contains("Checking file..."));
    }

    #[test]
    fn test_handle_runner_event_check_finished() {
        let mut app = make_app();

        let result = CheckResult {
            check_id: "php-lint".to_string(),
            status: CheckStatus::Passed,
            output: "OK".to_string(),
            error_output: String::new(),
            duration_ms: 500,
            started_at: None,
            finished_at: None,
        };

        app.handle_runner_event(RunnerEvent::CheckFinished { result });

        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            CheckStatus::Passed
        );
        assert_eq!(app.results.get("php-lint").unwrap().duration_ms, 500);
    }

    #[test]
    fn test_handle_runner_event_all_finished() {
        let mut app = make_app();
        assert!(!app.all_finished);

        app.handle_runner_event(RunnerEvent::AllFinished);

        assert!(app.all_finished);
        assert!(app.run_finished_at.is_some());
    }

    #[test]
    fn test_groups_returns_config_order() {
        let app = make_app();
        let groups = app.groups();

        // Should be in config order: fast, tests
        assert_eq!(groups, vec!["fast", "tests"]);
    }

    #[test]
    fn test_checks_in_group() {
        let app = make_app();

        let fast_checks = app.checks_in_group("fast");
        assert_eq!(fast_checks.len(), 1);
        assert_eq!(fast_checks[0].id(), "php-lint");

        let test_checks = app.checks_in_group("tests");
        assert_eq!(test_checks.len(), 2);
    }

    #[test]
    fn test_trigger_on_demand_check() {
        let mut app = make_app();

        // behat is on-demand
        assert_eq!(
            app.results.get("behat").unwrap().status,
            CheckStatus::OnDemand
        );

        app.trigger_on_demand_check("behat");

        assert_eq!(
            app.results.get("behat").unwrap().status,
            CheckStatus::Running
        );
    }

    #[test]
    fn test_reset_check_for_retry() {
        let mut app = make_app();

        // Mark as passed with output
        {
            let result = app.results.get_mut("php-lint").unwrap();
            result.status = CheckStatus::Passed;
            result.output = "Previous output".to_string();
            result.duration_ms = 500;
        }

        app.reset_check_for_retry("php-lint");

        let result = app.results.get("php-lint").unwrap();
        assert_eq!(result.status, CheckStatus::Running);
        assert!(result.output.is_empty());
        assert_eq!(result.duration_ms, 0);
    }

    #[test]
    fn test_toggle_full_command() {
        let mut app = make_app();

        assert!(!app.show_full_command);
        app.toggle_full_command();
        assert!(app.show_full_command);
        app.toggle_full_command();
        assert!(!app.show_full_command);
    }

    #[test]
    fn test_scroll_down_and_up() {
        let mut app = make_app();

        // Add some output so we can scroll
        app.results.get_mut("php-lint").unwrap().output =
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10".to_string();

        // Set visible lines smaller than output to allow scrolling (10 lines, 5 visible)
        app.set_output_visible_lines(5);

        assert_eq!(app.output_scroll, 0);

        app.scroll_down(3);
        assert_eq!(app.output_scroll, 3);

        app.scroll_up(2);
        assert_eq!(app.output_scroll, 1);

        // Can't scroll below 0
        app.scroll_up(10);
        assert_eq!(app.output_scroll, 0);
    }

    #[test]
    fn test_failed_filter_excludes_passed_pre_commands() {
        let mut app = make_app();

        // Add a pre-command that passed
        app.pre_commands.push(PreCommandState {
            group: "fast".to_string(),
            name: "init-db".to_string(),
            status: PreCommandStatus::Passed,
            output: String::new(),
            duration_ms: 100,
        });

        // Add a pre-command that failed
        app.pre_commands.push(PreCommandState {
            group: "fast".to_string(),
            name: "setup-env".to_string(),
            status: PreCommandStatus::Failed,
            output: "Error".to_string(),
            duration_ms: 50,
        });

        // Mark a check as failed so we have something to filter
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;

        // With All filter, both pre-commands should be visible
        app.status_filter = StatusFilter::All;
        let items = app.get_selectable_items();
        let pre_cmd_count = items
            .iter()
            .filter(|i| matches!(i, SelectableItem::PreCommand(_)))
            .count();
        assert_eq!(pre_cmd_count, 2, "All filter should show all pre-commands");

        // With Failed filter, only the failed pre-command should be visible
        app.status_filter = StatusFilter::Failed;
        let items = app.get_selectable_items();
        let pre_cmds: Vec<_> = items
            .iter()
            .filter_map(|i| {
                if let SelectableItem::PreCommand(pc) = i {
                    Some(pc)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            pre_cmds.len(),
            1,
            "Failed filter should only show failed pre-commands"
        );
        assert_eq!(
            pre_cmds[0].name, "setup-env",
            "Should only show the failed pre-command"
        );
    }

    #[test]
    fn test_skipped_no_files_initializes_as_skipped() {
        let config = parse_config();
        let changed_files = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: "development".to_string(),
        };

        // Create a check with skipped_no_files=true
        let mut check = make_check("php-lint", "fast", "PHP Lint", false, true);
        check.skipped_no_files = true;

        let checks = vec![check];

        let app = App::new(config, changed_files, checks, "main".to_string());

        // Check should be initialized as Skipped status
        let result = app.results.get("php-lint").unwrap();
        assert_eq!(result.status, CheckStatus::Skipped);
        assert_eq!(result.output, "No changes detected");
    }

    mod update_tests {
        use super::*;
        use pretty_assertions::assert_eq;

        #[test]
        fn test_update_next_check() {
            let mut app = make_app();
            assert_eq!(app.selected_check, 0);

            app.update(AppMessage::NextCheck);

            assert_eq!(app.selected_check, 1);
            assert!(app.needs_redraw);
        }

        #[test]
        fn test_update_previous_check() {
            let mut app = make_app();
            app.selected_check = 2;

            app.update(AppMessage::PreviousCheck);

            assert_eq!(app.selected_check, 1);
            assert!(app.needs_redraw);
        }

        #[test]
        fn test_update_toggle_filter() {
            let mut app = make_app();
            assert_eq!(app.status_filter, StatusFilter::All);

            app.update(AppMessage::ToggleFailedFilter);

            assert_eq!(app.status_filter, StatusFilter::Failed);
            assert!(app.needs_redraw);
        }

        #[test]
        fn test_update_system_stats() {
            let mut app = make_app();

            app.update(AppMessage::SystemStats {
                cpu_usage: 75.0,
                mem_used: 8_000_000_000,
                mem_total: 16_000_000_000,
            });

            assert_eq!(app.cpu_usage(), 75.0);
            assert_eq!(app.mem_usage(), 50.0);
            assert!(app.needs_redraw);
        }

        #[test]
        fn test_update_fix_lifecycle() {
            let mut app = make_app();

            // Start fix
            app.update(AppMessage::StartFix);
            assert!(app.fix_running);
            assert!(app.fix_result.is_none());

            // Finish fix
            let result = CheckResult {
                check_id: "test".to_string(),
                status: CheckStatus::Passed,
                output: "Fixed!".to_string(),
                error_output: String::new(),
                duration_ms: 100,
                started_at: None,
                finished_at: None,
            };
            app.update(AppMessage::FinishFix(result));

            assert!(!app.fix_running);
            assert!(app.fix_result.is_some());
            assert!(app.needs_redraw);
        }

        #[test]
        fn test_update_status_message() {
            let mut app = make_app();

            app.update(AppMessage::SetStatusMessage(Some(
                "Test message".to_string(),
            )));
            assert_eq!(app.status_message, Some("Test message".to_string()));

            app.update(AppMessage::ClearStatusMessage);
            assert!(app.status_message.is_none());
            assert!(app.needs_redraw);
        }

        #[test]
        fn test_update_clears_fix_result_on_navigation() {
            let mut app = make_app();
            app.fix_result = Some(CheckResult::pending("test"));

            app.update(AppMessage::NextCheck);

            assert!(app.fix_result.is_none());
        }

        #[test]
        fn test_update_scroll() {
            let mut app = make_app();
            app.results.get_mut("php-lint").unwrap().output = (0..50)
                .map(|i| format!("line {}", i))
                .collect::<Vec<_>>()
                .join("\n");
            app.set_output_visible_lines(10);

            app.update(AppMessage::ScrollDown(5));
            assert_eq!(app.output_scroll, 5);

            app.update(AppMessage::ScrollUp(3));
            assert_eq!(app.output_scroll, 2);
        }
    }

    mod needs_redraw_tests {
        use super::*;

        #[test]
        fn test_needs_redraw_true_on_new_app() {
            let app = make_app();
            assert!(app.needs_redraw, "New app should need initial render");
        }

        #[test]
        fn test_needs_redraw_can_be_cleared() {
            let mut app = make_app();
            app.needs_redraw = false;
            assert!(!app.needs_redraw, "needs_redraw should be clearable");
        }

        #[test]
        fn test_needs_redraw_set_after_update() {
            let mut app = make_app();
            app.needs_redraw = false; // Clear first
            app.update(AppMessage::NextCheck);
            assert!(app.needs_redraw, "update() should set needs_redraw");
        }

        #[test]
        fn test_needs_redraw_set_after_various_updates() {
            let messages = [
                AppMessage::PreviousCheck,
                AppMessage::ScrollUp(1),
                AppMessage::ScrollDown(1),
                AppMessage::ToggleFailedFilter,
                AppMessage::ShowAll,
                AppMessage::ToggleFullCommand,
            ];
            for msg in messages {
                let mut app = make_app();
                app.needs_redraw = false;
                app.update(msg);
                assert!(
                    app.needs_redraw,
                    "update() should set needs_redraw for all message types"
                );
            }
        }
    }
}
