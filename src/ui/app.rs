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
use std::time::{Duration, Instant};

/// Maximum number of samples to keep in history for sparklines
const MAX_HISTORY_SAMPLES: usize = 60;

/// Bytes in one GiB
const BYTES_PER_GIB: f64 = 1_073_741_824.0;

/// Default assumed output-panel height before first render measures it
const DEFAULT_OUTPUT_VISIBLE_LINES: usize = 20;

/// How long a status message stays visible before auto-dismissing
pub const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(3);

/// Filter for which checks to display in the UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    /// Show all checks
    All,
    /// Show only failed checks
    Failed,
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

/// A fix command ready to execute
#[derive(Debug, Clone)]
pub struct FixJob {
    /// Check being fixed (provides id, container override and env)
    pub check: CheckToRun,
    /// Resolved fix command with files substituted
    pub command: String,
}

/// State for fix and fix-all operations
#[derive(Debug, Default)]
pub struct FixState {
    /// True while a fix command is running
    pub running: bool,
    /// Result of the last fix command
    pub result: Option<CheckResult>,
    /// True while fix-all is running
    pub all_running: bool,
    /// Results from fix-all operation
    pub all_results: Vec<CheckResult>,
    /// Total number of fixes in fix-all
    pub all_total: usize,
}

/// System monitoring history (updated by background stats worker)
#[derive(Debug)]
pub struct SysStats {
    /// CPU usage history for sparkline (percentage values)
    pub cpu_history: VecDeque<f32>,
    /// Memory usage history for sparkline (percentage values)
    pub mem_history: VecDeque<f32>,
    /// Current memory usage in bytes
    pub mem_used_bytes: u64,
    /// Total memory in bytes (0 until first stats sample arrives)
    pub mem_total_bytes: u64,
}

impl Default for SysStats {
    fn default() -> Self {
        Self {
            cpu_history: VecDeque::with_capacity(64),
            mem_history: VecDeque::with_capacity(64),
            mem_used_bytes: 0,
            mem_total_bytes: 0,
        }
    }
}

/// How a status message leaves the footer (any keypress also dismisses it)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    /// Auto-dismisses after `STATUS_MESSAGE_TTL`
    Info,
    /// Stays until a keypress so failures are not missed
    Error,
    /// Stays until the work it describes finishes
    Progress,
}

/// Footer status message
#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub kind: StatusKind,
    /// Auto-dismiss deadline (`Info` only)
    pub expires_at: Option<Instant>,
}

/// UI view state: selection, scrolling, filtering, toggles, status line
#[derive(Debug)]
pub struct ViewState {
    /// Index of currently selected item in the filtered list
    pub selected_check: usize,
    /// Scroll position in output panel
    pub output_scroll: usize,
    /// Number of visible lines in output area (updated during render)
    pub output_visible_lines: usize,
    /// Current filter for check list
    pub status_filter: StatusFilter,
    /// Whether to show full command in output panel
    pub show_full_command: bool,
    /// Status message shown in the footer
    pub status_message: Option<StatusMessage>,
    /// First visible row of the checks list (kept so the list scrolls only
    /// when the selection leaves the window)
    pub checks_list_offset: usize,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            selected_check: 0,
            output_scroll: 0,
            output_visible_lines: DEFAULT_OUTPUT_VISIBLE_LINES,
            status_filter: StatusFilter::All,
            show_full_command: false,
            status_message: None,
            checks_list_offset: 0,
        }
    }
}

/// Progress state for the current check run
#[derive(Debug)]
pub struct RunState {
    /// Currently executing group name
    pub current_group: Option<String>,
    /// True when all checks have completed
    pub all_finished: bool,
    /// When the run started
    pub started_at: Option<Instant>,
    /// When the run finished
    pub finished_at: Option<Instant>,
    /// Currently running pre-command index
    pub current_pre_command: Option<usize>,
    /// Retry-all git refresh in flight (blocks a second 'R')
    pub refresh_pending: bool,
}

impl RunState {
    /// Fresh run state with the clock started now
    fn started_now() -> Self {
        Self {
            current_group: None,
            all_finished: false,
            started_at: Some(Instant::now()),
            finished_at: None,
            current_pre_command: None,
            refresh_pending: false,
        }
    }
}

/// Represents an item that can be selected in the checks list
#[derive(Debug, Clone)]
pub enum SelectableItem<'a> {
    PreCommand(&'a PreCommandState),
    Check(&'a CheckToRun),
}

/// Build initial CheckResult for a check based on its state
fn initial_result_for_check(check: &CheckToRun) -> CheckResult {
    if check.is_skipped_no_files() {
        CheckResult::skipped(check.id())
    } else if check.is_on_demand() {
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
    pub(crate) config: CiConfig,
    /// Files changed compared to base branch
    pub(crate) changed_files: ChangedFiles,
    /// Checks to be executed
    pub checks: Vec<CheckToRun>,
    /// Results indexed by check ID
    pub results: HashMap<String, CheckResult>,
    /// Current git branch name
    pub(crate) current_branch: String,

    /// Width of the output panel area (updated during render, used for
    /// command-line truncation when counting rendered lines)
    pub(crate) output_area_width: u16,

    /// UI view state: selection, scrolling, filtering, toggles, status line
    pub(crate) view: ViewState,

    /// Progress state for the current check run
    pub run: RunState,

    // Pre-command state
    /// Pre-commands and their status (group, name, status, output)
    pub(crate) pre_commands: Vec<PreCommandState>,

    /// State for fix and fix-all operations
    pub(crate) fix: FixState,

    /// System monitoring history (updated by background stats worker)
    pub(crate) sys: SysStats,

    /// Dirty flag - set when state changes, cleared after render
    /// Used to avoid unnecessary re-renders for better responsiveness
    pub(crate) needs_redraw: bool,
}

/// Number of checks per status bucket
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub passed: usize,
    pub failed: usize,
    /// Pending or running
    pub pending: usize,
    pub on_demand: usize,
}

/// What the currently selected check can do — computed once, read many times per frame
#[derive(Debug, Clone, Copy)]
pub struct SelectedCaps {
    pub can_fix: bool,
    pub can_retry: bool,
    pub can_trigger: bool,
    pub can_run_all_files: bool,
}

impl App {
    /// Create app state; results start pending, skipped or on-demand per check
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
            output_area_width: 80,
            view: ViewState::default(),
            run: RunState::started_now(),
            pre_commands,
            fix: FixState::default(),
            sys: SysStats::default(),
            needs_redraw: true, // Initial render needed
        }
    }

    /// Reset for retry - update changed files and checks, reset results
    pub fn reset_for_retry(&mut self, changed_files: ChangedFiles, checks: Vec<CheckToRun>) {
        self.changed_files = changed_files;

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
        self.checks = checks;

        // Reset state
        self.view = ViewState {
            status_filter: self.view.status_filter,
            output_visible_lines: self.view.output_visible_lines,
            ..ViewState::default()
        };
        self.run = RunState::started_now();
        self.fix = FixState::default();
        // sys stats deliberately survive retries
        self.needs_redraw = true;
    }

    /// Store a git-refreshed file list and check (single-check retry)
    pub fn replace_check(&mut self, changed_files: ChangedFiles, check: CheckToRun) {
        self.changed_files = changed_files;
        if let Some(slot) = self.checks.iter_mut().find(|c| c.id() == check.id()) {
            *slot = check;
        }
        self.needs_redraw = true;
    }

    /// Get total elapsed time
    pub fn elapsed_time(&self) -> std::time::Duration {
        let Some(started) = self.run.started_at else {
            return std::time::Duration::ZERO;
        };
        self.run
            .finished_at
            .map(|finished| finished.duration_since(started))
            .unwrap_or_else(|| started.elapsed())
    }

    /// Compute all selected-check capabilities with a single selection lookup
    pub fn selected_capabilities(&self) -> SelectedCaps {
        let busy = self.fix.running || self.fix.all_running;
        let selected = self.selected_check();
        let status = selected
            .and_then(|c| self.results.get(c.id()))
            .map(|r| r.status.clone());
        let has_fix = selected.map(|c| c.has_fix()).unwrap_or(false);
        SelectedCaps {
            can_fix: !busy && status == Some(CheckStatus::Failed) && has_fix,
            can_retry: !busy && matches!(status, Some(CheckStatus::Passed | CheckStatus::Failed)),
            can_trigger: !busy && status == Some(CheckStatus::OnDemand),
            can_run_all_files: !busy
                && matches!(
                    status,
                    Some(CheckStatus::Passed | CheckStatus::Failed | CheckStatus::OnDemand)
                ),
        }
    }

    /// Get the fix job for the selected check
    pub fn get_selected_fix_command(&self) -> Option<FixJob> {
        Self::fix_job(self.selected_check()?)
    }

    /// Mark fix as started
    pub fn start_fix(&mut self) {
        self.fix.running = true;
        self.fix.result = None;
        // Fix-all results take precedence in the output panel; clear them so
        // this fix's result is shown
        self.fix.all_results.clear();
        self.needs_redraw = true;
    }

    /// Store fix result
    pub fn finish_fix(&mut self, result: CheckResult) {
        self.fix.running = false;
        self.fix.result = Some(result);
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
        if self.fix.running || self.fix.all_running {
            return false;
        }
        !self.get_fixable_checks().is_empty()
    }

    /// Get fix jobs for all fixable checks
    pub fn get_all_fix_commands(&self) -> Vec<FixJob> {
        self.get_fixable_checks()
            .into_iter()
            .filter_map(Self::fix_job)
            .collect()
    }

    fn fix_job(check: &CheckToRun) -> Option<FixJob> {
        Some(FixJob {
            command: check.resolved_fix_command.clone()?,
            check: check.clone(),
        })
    }

    /// Start fix-all operation
    pub fn start_fix_all(&mut self, total: usize) {
        self.fix.all_running = true;
        self.fix.all_results = Vec::new();
        self.fix.all_total = total;
        self.fix.result = None;
        self.needs_redraw = true;
    }

    /// Add a result from fix-all
    pub fn add_fix_all_result(&mut self, result: CheckResult) {
        self.fix.all_results.push(result);
        self.needs_redraw = true;
    }

    /// Finish fix-all operation
    pub fn finish_fix_all(&mut self) {
        self.fix.all_running = false;
        self.needs_redraw = true;
    }

    /// Check if retry-all can start (blocked while fixes edit files)
    pub fn can_retry_all(&self) -> bool {
        !self.fix.running && !self.fix.all_running && !self.run.refresh_pending
    }

    /// Reset a check's result to Running and clear previous output/timing
    fn reset_result_to_running(&mut self, check_id: &str) {
        if let Some(result) = self.results.get_mut(check_id) {
            result.status = CheckStatus::Running;
            result.output.clear();
            result.error_output.clear();
            result.duration_ms = 0;
            result.started_at = Some(chrono::Local::now());
            result.finished_at = None;
        }
        self.clamp_selection();
    }

    /// Mark an on-demand check as running (preparing to execute)
    pub fn trigger_on_demand_check(&mut self, check_id: &str) {
        self.reset_result_to_running(check_id);
        self.needs_redraw = true;
    }

    /// Reset a single check to running for retry
    pub fn reset_check_for_retry(&mut self, check_id: &str) {
        self.reset_result_to_running(check_id);
        // Clear any fix results
        self.fix.result = None;
        self.fix.all_results.clear();
        self.needs_redraw = true;
    }

    /// Store a finished check result (runner, retry or run-all-files)
    pub fn set_retry_result(&mut self, result: CheckResult) {
        self.results.insert(result.check_id.clone(), result);
        self.clamp_selection();
        self.needs_redraw = true;
    }

    /// Show a status message in the footer
    pub fn set_status_message(&mut self, kind: StatusKind, text: impl Into<String>) {
        let expires_at = (kind == StatusKind::Info).then(|| Instant::now() + STATUS_MESSAGE_TTL);
        self.view.status_message = Some(StatusMessage {
            text: text.into(),
            kind,
            expires_at,
        });
        self.needs_redraw = true;
    }

    /// Clear the status message (any keypress dismisses it)
    pub fn clear_status_message(&mut self) {
        self.view.status_message = None;
        self.needs_redraw = true;
    }

    /// When the current status message auto-dismisses, if ever
    pub fn status_message_deadline(&self) -> Option<Instant> {
        self.view.status_message.as_ref()?.expires_at
    }

    /// Clear the status message once its deadline has passed
    pub fn expire_status_message(&mut self, now: Instant) {
        if self.status_message_deadline().is_some_and(|d| now >= d) {
            self.clear_status_message();
        }
    }

    /// Clear a progress message once its work is done
    pub fn finish_progress(&mut self) {
        if self
            .view
            .status_message
            .as_ref()
            .is_some_and(|m| m.kind == StatusKind::Progress)
        {
            self.clear_status_message();
        }
    }

    /// Update system stats from background task data
    ///
    /// This is called when the stats background worker sends new data.
    /// The actual sysinfo queries happen in a separate task to avoid
    /// blocking the UI thread.
    pub fn update_stats(&mut self, cpu_usage: f32, mem_used: u64, mem_total: u64) {
        self.sys.mem_used_bytes = mem_used;
        self.sys.mem_total_bytes = mem_total;

        // Memory usage percentage
        let mem_usage = if mem_total > 0 {
            (mem_used as f32 / mem_total as f32) * 100.0
        } else {
            0.0
        };

        // Add to history (keep last 60 samples) - VecDeque for O(1) pop_front
        self.sys.cpu_history.push_back(cpu_usage);
        if self.sys.cpu_history.len() > MAX_HISTORY_SAMPLES {
            self.sys.cpu_history.pop_front();
        }

        self.sys.mem_history.push_back(mem_usage);
        if self.sys.mem_history.len() > MAX_HISTORY_SAMPLES {
            self.sys.mem_history.pop_front();
        }

        self.needs_redraw = true;
    }

    /// Latest CPU usage (percent)
    pub fn cpu_usage(&self) -> f32 {
        self.sys.cpu_history.back().copied().unwrap_or(0.0)
    }

    /// Latest memory usage (percent)
    pub fn mem_usage(&self) -> f32 {
        self.sys.mem_history.back().copied().unwrap_or(0.0)
    }

    /// Used memory in GiB (2^30 bytes)
    pub fn mem_used_gib(&self) -> f64 {
        self.sys.mem_used_bytes as f64 / BYTES_PER_GIB
    }

    /// Total memory in GiB (2^30 bytes)
    pub fn mem_total_gib(&self) -> f64 {
        self.sys.mem_total_bytes as f64 / BYTES_PER_GIB
    }

    /// Keep selection valid: the filtered item list can shrink when a Failed
    /// check changes status while the Failed filter is active. An
    /// out-of-range index made selected_item() return None and blanked the
    /// output panel. Called by every mutator that changes statuses or items.
    fn clamp_selection(&mut self) {
        let max = self.selectable_items().count().saturating_sub(1);
        if self.view.selected_check > max {
            self.view.selected_check = max;
        }
    }

    /// Apply a lifecycle event from the check runner
    pub fn handle_runner_event(&mut self, event: RunnerEvent) {
        self.needs_redraw = true;
        match event {
            RunnerEvent::CheckStarted { check_id } => self.on_check_started(&check_id),
            RunnerEvent::CheckFinished { result } => {
                self.results.insert(result.check_id.clone(), result);
            }
            RunnerEvent::GroupStarted { group } => {
                self.run.current_group = Some(group);
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
                self.run.all_finished = true;
                self.run.current_group = None;
                self.run.finished_at = Some(Instant::now());
            }
        }
        self.clamp_selection();
    }

    fn on_check_started(&mut self, check_id: &str) {
        if let Some(result) = self.results.get_mut(check_id) {
            result.status = CheckStatus::Running;
            result.started_at = Some(chrono::Local::now());
        }
    }

    fn on_pre_command_started(&mut self, group: &str, name: &str) {
        if let Some(idx) = self
            .pre_commands
            .iter()
            .position(|p| p.group == group && p.name == name)
        {
            self.pre_commands[idx].status = PreCommandStatus::Running;
            self.run.current_pre_command = Some(idx);
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
        self.run.current_pre_command = None;
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

    /// Get the selected check, if a check (not a pre-command) is selected
    pub fn selected_check(&self) -> Option<&CheckToRun> {
        match self.selected_item() {
            Some(SelectableItem::Check(check)) => Some(check),
            _ => None,
        }
    }

    // Navigation - all methods set needs_redraw for immediate visual feedback

    /// Select the next item (stops at the last one)
    pub fn next_check(&mut self) {
        // Clear fix results and reset command view when navigating
        self.fix.result = None;
        self.fix.all_results.clear();
        self.view.output_scroll = 0;
        self.view.show_full_command = false;

        let max = self.selectable_items().count().saturating_sub(1);
        if self.view.selected_check < max {
            self.view.selected_check += 1;
        }
        self.needs_redraw = true;
    }

    /// Select the previous item (stops at the first one)
    pub fn previous_check(&mut self) {
        // Clear fix results and reset command view when navigating
        self.fix.result = None;
        self.fix.all_results.clear();
        self.view.output_scroll = 0;
        self.view.show_full_command = false;

        if self.view.selected_check > 0 {
            self.view.selected_check -= 1;
        }
        self.needs_redraw = true;
    }

    /// Scroll the output panel up by `n` rows
    pub fn scroll_up(&mut self, n: usize) {
        self.view.output_scroll = self.view.output_scroll.saturating_sub(n);
        self.needs_redraw = true;
    }

    /// Scroll the output panel down by `n` rows (clamped to the content)
    pub fn scroll_down(&mut self, n: usize) {
        let max_scroll = self.compute_max_scroll();
        self.view.output_scroll = (self.view.output_scroll + n).min(max_scroll);
        self.needs_redraw = true;
    }

    /// Compute maximum scroll offset for whatever the output panel shows.
    /// Capped at `u16::MAX`, the largest offset ratatui can scroll to.
    fn compute_max_scroll(&self) -> usize {
        super::dashboard::output_line_count(self, self.output_area_width)
            .saturating_sub(self.view.output_visible_lines)
            .min(u16::MAX as usize)
    }

    /// Pull the scroll offset back when the output shrank (retry cleared
    /// it, command collapsed, panel resized), so the panel never goes blank
    pub(crate) fn clamp_output_scroll(&mut self) {
        if self.view.output_scroll > 0 {
            self.view.output_scroll = self.view.output_scroll.min(self.compute_max_scroll());
        }
    }

    /// Set the number of visible lines in output area (called during render)
    pub fn set_output_visible_lines(&mut self, lines: usize) {
        self.view.output_visible_lines = lines;
    }

    /// Toggle between showing all checks and only failed ones
    pub fn toggle_failed_filter(&mut self) {
        self.view.status_filter = match self.view.status_filter {
            StatusFilter::All => StatusFilter::Failed,
            StatusFilter::Failed => StatusFilter::All,
        };
        self.view.selected_check = 0;
        self.needs_redraw = true;
    }

    /// Show all checks (clear the failed filter)
    pub fn show_all(&mut self) {
        self.view.status_filter = StatusFilter::All;
        self.view.selected_check = 0;
        self.needs_redraw = true;
    }

    /// Toggle full vs truncated command (and file list) in the output panel
    pub fn toggle_full_command(&mut self) {
        self.view.show_full_command = !self.view.show_full_command;
        self.needs_redraw = true;
    }

    /// Count check results per status bucket (skipped checks are not counted)
    pub fn count_by_status(&self) -> StatusCounts {
        let mut counts = StatusCounts::default();
        for result in self.results.values() {
            match result.status {
                CheckStatus::Passed => counts.passed += 1,
                CheckStatus::Failed => counts.failed += 1,
                CheckStatus::Pending | CheckStatus::Running => counts.pending += 1,
                CheckStatus::OnDemand => counts.on_demand += 1,
                CheckStatus::Skipped => (),
            }
        }
        counts
    }

    /// Process exit code at quit: 1 if any check failed, else 0 (matches simple mode)
    pub fn exit_code(&self) -> i32 {
        i32::from(self.count_by_status().failed > 0)
    }

    /// Groups that have checks, in config (YAML) order
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

    /// Checks belonging to `group`, in check order
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
    #[cfg(test)]
    fn get_selectable_items(&self) -> Vec<SelectableItem<'_>> {
        self.selectable_items().collect()
    }

    /// Lazy iterator over selectable items in display order. The checks
    /// list is built from this, so list rows and selection always agree.
    pub(crate) fn selectable_items(&self) -> impl Iterator<Item = SelectableItem<'_>> {
        self.groups().into_iter().flat_map(move |group| {
            let pre_commands = self
                .pre_commands
                .iter()
                .filter(move |p| p.group == group && self.should_show_pre_command(p))
                .map(SelectableItem::PreCommand);
            let checks = self
                .checks
                .iter()
                .filter(move |c| c.group() == group && self.should_show_check(c))
                .map(SelectableItem::Check);
            pre_commands.chain(checks)
        })
    }

    fn should_show_pre_command(&self, pre_cmd: &PreCommandState) -> bool {
        match self.view.status_filter {
            StatusFilter::All => true,
            StatusFilter::Failed => pre_cmd.status == PreCommandStatus::Failed,
        }
    }

    fn should_show_check(&self, check: &CheckToRun) -> bool {
        match self.view.status_filter {
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
        self.selectable_items().nth(self.view.selected_check)
    }

    /// Get the selected pre-command, if one is selected
    pub fn selected_pre_command(&self) -> Option<&PreCommandState> {
        match self.selected_item() {
            Some(SelectableItem::PreCommand(pc)) => Some(pc),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{CheckFiles, CheckToRun};
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
            service: Some("php".to_string()),
            files: if on_demand {
                CheckFiles::OnDemand
            } else {
                CheckFiles::Files(vec!["test.php".to_string()])
            },
            resolved_command: format!("{} test.php", id),
            resolved_fix_command: if has_fix {
                Some(format!("{} --fix test.php", id))
            } else {
                None
            },
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

        assert_eq!(app.view.selected_check, 0);
        assert_eq!(app.view.output_scroll, 0);
        assert_eq!(app.view.status_filter, StatusFilter::All);
        assert!(!app.run.all_finished);
        assert!(!app.fix.running);
        assert!(app.needs_redraw);
    }

    #[test]
    fn test_navigation_next() {
        let mut app = make_app();

        assert_eq!(app.view.selected_check, 0);
        app.next_check();
        assert_eq!(app.view.selected_check, 1);
        app.next_check();
        assert_eq!(app.view.selected_check, 2);

        // Should not go past last item
        app.next_check();
        assert_eq!(app.view.selected_check, 2);
    }

    #[test]
    fn test_navigation_previous() {
        let mut app = make_app();
        app.view.selected_check = 2;

        app.previous_check();
        assert_eq!(app.view.selected_check, 1);
        app.previous_check();
        assert_eq!(app.view.selected_check, 0);

        // Should not go below 0
        app.previous_check();
        assert_eq!(app.view.selected_check, 0);
    }

    #[test]
    fn test_navigation_clears_fix_result() {
        let mut app = make_app();
        app.fix.result = Some(CheckResult::pending("test"));

        app.next_check();

        assert!(app.fix.result.is_none());
    }

    #[test]
    fn test_toggle_failed_filter() {
        let mut app = make_app();

        assert_eq!(app.view.status_filter, StatusFilter::All);
        app.toggle_failed_filter();
        assert_eq!(app.view.status_filter, StatusFilter::Failed);
        app.toggle_failed_filter();
        assert_eq!(app.view.status_filter, StatusFilter::All);
    }

    #[test]
    fn test_show_all_resets_filter() {
        let mut app = make_app();
        app.view.status_filter = StatusFilter::Failed;
        app.view.selected_check = 5;

        app.show_all();

        assert_eq!(app.view.status_filter, StatusFilter::All);
        assert_eq!(app.view.selected_check, 0);
    }

    #[test]
    fn test_count_by_status() {
        let mut app = make_app();

        // Initial state: 2 pending, 1 on-demand
        let counts = app.count_by_status();
        assert_eq!(
            counts,
            StatusCounts {
                passed: 0,
                failed: 0,
                pending: 2,
                on_demand: 1
            }
        );

        // Mark one as passed
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Passed;

        let counts = app.count_by_status();
        assert_eq!(
            counts,
            StatusCounts {
                passed: 1,
                failed: 0,
                pending: 1,
                on_demand: 1
            }
        );

        // Mark one as failed
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        let counts = app.count_by_status();
        assert_eq!(
            counts,
            StatusCounts {
                passed: 1,
                failed: 1,
                pending: 0,
                on_demand: 1
            }
        );
    }

    #[test]
    fn test_exit_code_zero_when_all_passed() {
        let mut app = make_app();
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Passed;
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Passed;

        assert_eq!(app.exit_code(), 0);
    }

    #[test]
    fn test_exit_code_one_when_any_failed() {
        let mut app = make_app();
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Passed;
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        assert_eq!(app.exit_code(), 1);
    }

    #[test]
    fn test_can_fix_selected_requires_failed_and_fix_command() {
        let mut app = make_app();

        // Check phpunit which has fix command
        app.view.selected_check = 1; // phpunit

        // Not failed yet - can't fix
        assert!(!app.selected_capabilities().can_fix);

        // Mark as failed
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        // Now can fix
        assert!(app.selected_capabilities().can_fix);
    }

    #[test]
    fn test_can_fix_selected_no_fix_command() {
        let mut app = make_app();

        // Select php-lint which has no fix command
        app.view.selected_check = 0; // php-lint
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;

        // Can't fix because no fix command
        assert!(!app.selected_capabilities().can_fix);
    }

    #[test]
    fn test_can_fix_selected_disabled_during_fix() {
        let mut app = make_app();
        app.view.selected_check = 1; // phpunit
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        app.fix.running = true;

        assert!(!app.selected_capabilities().can_fix);
    }

    #[test]
    fn test_can_trigger_selected() {
        let mut app = make_app();

        // Select behat which is on-demand
        app.view.selected_check = 2;

        assert!(app.selected_capabilities().can_trigger);

        // Select php-lint which is pending
        app.view.selected_check = 0;
        assert!(!app.selected_capabilities().can_trigger);
    }

    #[test]
    fn test_can_retry_selected() {
        let mut app = make_app();
        app.view.selected_check = 0; // php-lint

        // Can't retry pending
        assert!(!app.selected_capabilities().can_retry);

        // Can retry passed
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Passed;
        assert!(app.selected_capabilities().can_retry);

        // Can retry failed
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;
        assert!(app.selected_capabilities().can_retry);
    }

    #[test]
    fn test_start_and_finish_fix() {
        let mut app = make_app();

        app.start_fix();
        assert!(app.fix.running);
        assert!(app.fix.result.is_none());

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
        assert!(!app.fix.running);
        assert!(app.fix.result.is_some());
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
        assert_eq!(app.sys.mem_used_bytes, 8_000_000_000);
        assert_eq!(app.sys.mem_total_bytes, 16_000_000_000);
    }

    #[test]
    fn test_update_stats_history_limit() {
        let mut app = make_app();

        // Add more samples than MAX_HISTORY_SAMPLES
        for i in 0..70 {
            app.update_stats(i as f32, 1000, 2000);
        }

        // Should be capped at MAX_HISTORY_SAMPLES
        assert_eq!(app.sys.cpu_history.len(), MAX_HISTORY_SAMPLES);
        assert_eq!(app.sys.mem_history.len(), MAX_HISTORY_SAMPLES);

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
        assert!(!app.run.all_finished);

        app.handle_runner_event(RunnerEvent::AllFinished);

        assert!(app.run.all_finished);
        assert!(app.run.finished_at.is_some());
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

        assert!(!app.view.show_full_command);
        app.toggle_full_command();
        assert!(app.view.show_full_command);
        app.toggle_full_command();
        assert!(!app.view.show_full_command);
    }

    #[test]
    fn test_scroll_down_and_up() {
        let mut app = make_app();

        // Add some output so we can scroll
        app.results.get_mut("php-lint").unwrap().output =
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10".to_string();

        // Set visible lines smaller than output to allow scrolling (10 lines, 5 visible)
        app.set_output_visible_lines(5);

        assert_eq!(app.view.output_scroll, 0);

        app.scroll_down(3);
        assert_eq!(app.view.output_scroll, 3);

        app.scroll_up(2);
        assert_eq!(app.view.output_scroll, 1);

        // Can't scroll below 0
        app.scroll_up(10);
        assert_eq!(app.view.output_scroll, 0);
    }

    #[test]
    fn test_scroll_max_includes_rendered_header_lines() {
        let mut app = make_app();
        {
            let r = app.results.get_mut("php-lint").unwrap();
            r.status = CheckStatus::Passed;
            r.output = (1..=20)
                .map(|i| format!("line {}", i))
                .collect::<Vec<_>>()
                .join("\n");
        }
        app.set_output_visible_lines(5);
        app.output_area_width = 80;

        app.scroll_down(1000);

        // Rendered text prepends 6 lines before the 20 output lines:
        // "$ php-lint test.php", blank, "PASSED", blank, "Files: test.php", blank.
        // max scroll = 26 total - 5 visible = 21. The old clamp
        // (stdout-only count) allowed only 20 - 5 = 15.
        assert_eq!(app.view.output_scroll, 21);
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
        app.view.status_filter = StatusFilter::All;
        let items = app.get_selectable_items();
        let pre_cmd_count = items
            .iter()
            .filter(|i| matches!(i, SelectableItem::PreCommand(_)))
            .count();
        assert_eq!(pre_cmd_count, 2, "All filter should show all pre-commands");

        // With Failed filter, only the failed pre-command should be visible
        app.view.status_filter = StatusFilter::Failed;
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
    fn test_selection_clamped_when_filtered_list_shrinks() {
        let mut app = make_app();
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;
        app.view.status_filter = StatusFilter::Failed;
        app.view.selected_check = 1; // phpunit, second item in the filtered list

        // phpunit passes on retry - the filtered list shrinks to 1 item
        let result = CheckResult {
            check_id: "phpunit".to_string(),
            status: CheckStatus::Passed,
            output: String::new(),
            error_output: String::new(),
            duration_ms: 10,
            started_at: None,
            finished_at: None,
        };
        app.handle_runner_event(RunnerEvent::CheckFinished { result });

        assert_eq!(
            app.view.selected_check, 0,
            "selection must be clamped to list length"
        );
        assert!(
            app.selected_item().is_some(),
            "output panel must not go blank after list shrinks"
        );
    }

    #[test]
    fn test_selection_clamped_when_set_retry_result_shrinks_filtered_list() {
        let mut app = make_app();
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;
        app.view.status_filter = StatusFilter::Failed;
        app.view.selected_check = 1; // phpunit, second item in the filtered list

        // phpunit passes via set_retry_result - the filtered list shrinks to 1 item
        let result = CheckResult {
            check_id: "phpunit".to_string(),
            status: CheckStatus::Passed,
            output: String::new(),
            error_output: String::new(),
            duration_ms: 10,
            started_at: None,
            finished_at: None,
        };
        app.set_retry_result(result);

        assert_eq!(
            app.view.selected_check, 0,
            "selection must be clamped to list length"
        );
        assert!(
            app.selected_item().is_some(),
            "output panel must not go blank after list shrinks"
        );
    }

    #[test]
    fn test_skipped_no_files_initializes_as_skipped() {
        let config = parse_config();
        let changed_files = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: "development".to_string(),
        };

        // SkippedNoMatch + `{files}` command => skipped_no_files
        let mut check = make_check("php-lint", "fast", "PHP Lint", false, true);
        check.files = CheckFiles::SkippedNoMatch;

        let checks = vec![check];

        let app = App::new(config, changed_files, checks, "main".to_string());

        // Check should be initialized as Skipped status
        let result = app.results.get("php-lint").unwrap();
        assert_eq!(result.status, CheckStatus::Skipped);
        assert_eq!(result.output, "No changes detected");
    }

    #[test]
    fn test_reset_for_retry_keeps_status_filter() {
        let mut app = make_app();
        app.view.status_filter = StatusFilter::Failed;
        let checks = app.checks.clone();
        let changed_files = app.changed_files.clone();

        app.reset_for_retry(changed_files, checks);

        assert_eq!(app.view.status_filter, StatusFilter::Failed);
        assert_eq!(app.view.selected_check, 0);
    }

    #[test]
    fn test_can_fix_selected_disabled_during_fix_all() {
        let mut app = make_app();
        app.view.selected_check = 1; // phpunit
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        app.start_fix_all(1);

        assert!(!app.selected_capabilities().can_fix);
    }

    #[test]
    fn test_can_retry_all_disabled_during_fixes() {
        let mut app = make_app();
        assert!(app.can_retry_all());
        app.start_fix();
        assert!(!app.can_retry_all());
        app.fix.running = false;
        app.start_fix_all(1);
        assert!(!app.can_retry_all());
    }

    #[test]
    fn test_can_retry_all_disabled_during_refresh() {
        let mut app = make_app();
        app.run.refresh_pending = true;
        assert!(!app.can_retry_all());
        app.reset_for_retry(app.changed_files.clone(), app.checks.clone());
        assert!(app.can_retry_all(), "reset clears pending refresh");
    }

    #[test]
    fn test_fix_jobs_carry_check() {
        let mut app = make_app();
        app.results.get_mut("phpunit").unwrap().status = CheckStatus::Failed;

        let jobs = app.get_all_fix_commands();

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].check.id(), "phpunit");
        assert_eq!(jobs[0].command, "phpunit --fix test.php");
    }

    #[test]
    fn test_scroll_down_covers_fix_all_results() {
        let mut app = make_app();
        app.set_output_visible_lines(5);
        for i in 0..20 {
            app.fix
                .all_results
                .push(CheckResult::pending(&format!("c{}", i)));
        }

        app.scroll_down(1000);

        // Header line + blank + 20 result lines = 22; 22 - 5 visible = 17
        assert_eq!(app.view.output_scroll, 17);
    }

    #[test]
    fn test_start_fix_clears_fix_all_results() {
        let mut app = make_app();
        app.fix.all_results.push(CheckResult::pending("phpunit"));

        app.start_fix();

        assert!(
            app.fix.all_results.is_empty(),
            "single fix must not be hidden behind old fix-all results"
        );
    }

    #[test]
    fn test_clamp_output_scroll_after_output_shrinks() {
        let mut app = make_app();
        app.set_output_visible_lines(10);
        app.results.get_mut("php-lint").unwrap().output = "line\n".repeat(50);
        app.scroll_down(1000);
        assert!(app.view.output_scroll > 0);

        // Retry clears the output; the header alone fits in 10 rows
        app.reset_check_for_retry("php-lint");
        app.clamp_output_scroll();

        assert_eq!(app.view.output_scroll, 0);
    }

    #[test]
    fn test_scroll_max_counts_wrapped_rows() {
        let mut app = make_app();
        app.set_output_visible_lines(5);
        app.output_area_width = 42; // 40 inner columns
        {
            let r = app.results.get_mut("php-lint").unwrap();
            r.status = CheckStatus::Passed;
            r.output = "x".repeat(200);
        }

        app.scroll_down(1000);

        // 6 header rows + 200 chars wrapped at 40 cols (5 rows) = 11 rows;
        // 11 - 5 visible = 6. Counting text lines gave 7 - 5 = 2.
        assert_eq!(app.view.output_scroll, 6);
    }

    #[test]
    fn test_replace_check_updates_check_and_files() {
        let mut app = make_app();
        let mut check = app.checks[0].clone();
        check.resolved_command = "php-lint new.php".to_string();
        let files = ChangedFiles {
            files: vec!["new.php".to_string()],
            base_ref: "development".to_string(),
        };

        app.replace_check(files, check);

        assert_eq!(app.checks[0].resolved_command, "php-lint new.php");
        assert_eq!(app.changed_files.files, vec!["new.php".to_string()]);
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
        fn test_needs_redraw_set_after_mutations() {
            let cases: Vec<(&str, fn(&mut App))> = vec![
                ("next_check", |a| a.next_check()),
                ("previous_check", |a| a.previous_check()),
                ("scroll_up", |a| a.scroll_up(1)),
                ("scroll_down", |a| a.scroll_down(1)),
                ("toggle_failed_filter", |a| a.toggle_failed_filter()),
                ("show_all", |a| a.show_all()),
                ("toggle_full_command", |a| a.toggle_full_command()),
                ("clear_status_message", |a| a.clear_status_message()),
                ("start_fix", |a| a.start_fix()),
                ("finish_fix", |a| {
                    a.finish_fix(CheckResult {
                        check_id: "phpunit".to_string(),
                        status: CheckStatus::Passed,
                        output: String::new(),
                        error_output: String::new(),
                        duration_ms: 10,
                        started_at: None,
                        finished_at: None,
                    })
                }),
                ("trigger_on_demand_check", |a| {
                    a.trigger_on_demand_check("behat")
                }),
                ("reset_check_for_retry", |a| {
                    a.reset_check_for_retry("phpunit")
                }),
                ("set_retry_result", |a| {
                    a.set_retry_result(CheckResult {
                        check_id: "phpunit".to_string(),
                        status: CheckStatus::Passed,
                        output: String::new(),
                        error_output: String::new(),
                        duration_ms: 10,
                        started_at: None,
                        finished_at: None,
                    })
                }),
            ];
            for (name, mutate) in cases {
                let mut app = make_app();
                app.needs_redraw = false;
                mutate(&mut app);
                assert!(app.needs_redraw, "{} should set needs_redraw", name);
            }
        }

        #[test]
        fn test_set_and_clear_status_message() {
            let mut app = make_app();
            app.set_status_message(StatusKind::Info, "Test message");
            assert_eq!(
                app.view.status_message.as_ref().map(|m| m.text.as_str()),
                Some("Test message")
            );
            app.clear_status_message();
            assert!(app.view.status_message.is_none());
        }

        #[test]
        fn test_status_message_expires_after_ttl() {
            let mut app = make_app();
            app.set_status_message(StatusKind::Info, "Test message");
            let deadline = app.status_message_deadline().expect("info expires");

            app.expire_status_message(deadline - Duration::from_millis(1));
            assert!(app.view.status_message.is_some(), "not yet expired");

            app.expire_status_message(deadline);
            assert!(app.view.status_message.is_none(), "expired at deadline");
        }

        #[test]
        fn test_error_and_progress_messages_do_not_expire() {
            let mut app = make_app();
            for kind in [StatusKind::Error, StatusKind::Progress] {
                app.set_status_message(kind, "sticky");
                assert!(app.status_message_deadline().is_none());
                app.expire_status_message(Instant::now() + STATUS_MESSAGE_TTL * 10);
                assert!(app.view.status_message.is_some(), "{:?} must stay", kind);
            }
        }

        #[test]
        fn test_finish_progress_clears_only_progress() {
            let mut app = make_app();
            app.set_status_message(StatusKind::Error, "failed");
            app.finish_progress();
            assert!(app.view.status_message.is_some(), "error survives");

            app.set_status_message(StatusKind::Progress, "working...");
            app.finish_progress();
            assert!(app.view.status_message.is_none(), "progress cleared");
        }
    }
}
