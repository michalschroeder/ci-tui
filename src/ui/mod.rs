//! Terminal UI using ratatui for interactive check execution.
//!
//! This module implements the main TUI event loop and coordinates between
//! user input, check execution, and rendering. It provides lifecycle-event
//! feedback during check execution with keyboard-driven navigation (output
//! is captured and shown on completion, not streamed live).
//!
//! # Architecture
//!
//! - **Keyboard input**: Handled on dedicated OS thread for responsiveness
//! - **Check execution**: Runs in Tokio tasks with event streaming
//! - **System stats**: Collected in background task to avoid UI blocking
//! - **Event loop**: Uses `tokio::select!` with `biased;` for keyboard priority
//!
//! # Submodules
//!
//! - [`app`]: Application state management
//! - [`dashboard`]: Rendering logic using ratatui widgets

pub mod app;
pub mod dashboard;

use crate::checks::{determine_checks, CheckToRun};
use crate::config::CiConfig;
use crate::git::{current_branch, get_changed_files, ChangedFiles};
use crate::runner::{run_check_with_command, CheckResult, CheckRunner, RunnerEvent};
use anyhow::Result;
use app::{App, StatusKind};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io::{self, stdout};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use sysinfo::System;
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};

// Constants for timing and performance tuning
/// Interval between system stats updates (CPU, memory)
const STATS_UPDATE_INTERVAL: Duration = Duration::from_millis(500);
/// Keyboard poll timeout - responsive enough for shutdown, not too CPU intensive
const KEYBOARD_POLL_TIMEOUT: Duration = Duration::from_millis(50);
/// Channel capacity for system stats
const STATS_CHANNEL_CAPACITY: usize = 4;
/// Channel capacity for runner events
const RUNNER_CHANNEL_CAPACITY: usize = 100;
/// Channel capacity for background task events
const TASK_CHANNEL_CAPACITY: usize = 16;
/// Lines to scroll per PageUp/PageDown press
const PAGE_SCROLL_LINES: usize = 10;

/// System stats data sent from background task
#[derive(Debug, Clone)]
pub struct SystemStats {
    pub cpu_usage: f32,
    pub mem_used: u64,
    pub mem_total: u64,
}

/// Messages for the event loop - all state transitions go through explicit Message variants
#[derive(Debug)]
enum Message {
    KeyPress(KeyEvent),
    /// Terminal resized - redraw with the new layout
    Resize,
    RunnerEvent(RunnerEvent),
    SystemStats(SystemStats),
    Task(TaskEvent),
    /// Status message auto-dismiss deadline reached
    StatusExpired,
}

/// Events from spawned background tasks (fix, fix-all, retry, refresh)
#[derive(Debug)]
enum TaskEvent {
    /// Single fix command finished
    FixResult(CheckResult),
    /// One fix-all command finished
    FixAllResult(CheckResult),
    /// The whole fix-all loop finished (sent unconditionally after the loop)
    FixAllDone,
    /// Git refresh for a single retry found the updated check
    CheckRefreshed {
        changed_files: ChangedFiles,
        check: Box<CheckToRun>,
    },
    /// Check dropped out after git refresh; carries its previous result
    CheckNotApplicable(CheckResult),
    /// Git refresh failed; the retry runs with the previous file list
    GitRefreshFailed,
    /// Retry / on-demand / run-all-files result
    RetryResult(CheckResult),
    /// Git refresh for retry-all finished; restart the runner
    RetryAllReady {
        changed_files: ChangedFiles,
        checks: Vec<CheckToRun>,
    },
}

/// Read keyboard and resize events and send them through a channel
///
/// This runs on a dedicated OS thread for responsiveness under high CPU load.
fn keyboard_loop(shutdown: Arc<AtomicBool>, tx: mpsc::UnboundedSender<Event>) {
    while !shutdown.load(Ordering::Relaxed) {
        if !event::poll(KEYBOARD_POLL_TIMEOUT).unwrap_or(false) {
            continue;
        }
        let event = match event::read() {
            // Filter for Press events only (Windows sends Press+Release)
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => Event::Key(key),
            Ok(event @ Event::Resize(..)) => event,
            _ => continue,
        };
        // If send fails, receiver is dropped - exit thread
        if tx.send(event).is_err() {
            break;
        }
    }
}

/// Spawn a dedicated OS thread for keyboard input handling
///
/// CRITICAL: This uses std::thread (NOT tokio::spawn) to ensure keyboard events
/// are processed by the OS scheduler independently of Tokio's task scheduling.
/// Under high CPU load from Docker containers, Tokio tasks may be starved,
/// but OS threads will still be scheduled by the kernel.
///
/// The thread polls for keyboard events and sends them through an unbounded
/// channel to the main event loop. Using unbounded ensures we never drop
/// keyboard events due to backpressure.
fn spawn_keyboard_thread(
    shutdown: Arc<AtomicBool>,
) -> (mpsc::UnboundedReceiver<Event>, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel();

    let handle = thread::Builder::new()
        .name("keyboard-input".to_string())
        .spawn(move || keyboard_loop(shutdown, tx))
        .expect("Failed to spawn keyboard thread");

    (rx, handle)
}

/// Spawn a background task that collects system stats without blocking the UI
///
/// This runs sysinfo queries in a separate task and sends results via channel.
/// The UI thread never blocks on syscalls for memory/CPU stats.
fn spawn_stats_worker(tx: mpsc::Sender<SystemStats>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut system = System::new();
        let mut interval = tokio::time::interval(STATS_UPDATE_INTERVAL);

        loop {
            interval.tick().await;

            // These sysinfo calls can be slow under high load, but they run
            // in this background task rather than blocking the UI thread
            system.refresh_cpu_usage();
            system.refresh_memory();

            let cpu_usage: f32 = system.cpus().iter().map(|cpu| cpu.cpu_usage()).sum::<f32>()
                / system.cpus().len().max(1) as f32;

            let stats = SystemStats {
                cpu_usage,
                mem_used: system.used_memory(),
                mem_total: system.total_memory(),
            };

            // Non-blocking send - skip if channel is full
            let _ = tx.try_send(stats);
        }
    })
}

/// Action result from handle_message
enum Action {
    Continue,
    Quit,
    RestartRunner {
        new_changed_files: ChangedFiles,
        new_checks: Vec<CheckToRun>,
    },
}

/// Shared handles for spawned async tasks (Arcs for cheap cloning)
#[derive(Clone)]
struct TaskCtx {
    /// Git change detection root (cwd)
    project_root: Arc<PathBuf>,
    /// Command execution + test discovery root (repo root in local mode, cwd in docker mode)
    exec_root: Arc<PathBuf>,
    /// Config; `config.runner` is where commands execute (docker or local host)
    config: Arc<CiConfig>,
}

impl TaskCtx {
    /// Run `command` as `check` (check's container override and env apply)
    async fn run(&self, check: &CheckToRun, command: &str) -> CheckResult {
        run_check_with_command(check, command, &self.exec_root, &self.config.runner).await
    }

    /// Re-detect changed files and matching checks. Blocking (git + file
    /// system): call from `spawn_blocking`. `--files` lists are kept as-is
    /// (no git base to diff against); only the checks are re-determined.
    fn refresh(&self, previous: ChangedFiles) -> Result<(ChangedFiles, Vec<CheckToRun>)> {
        let changed_files = if previous.is_cli_files() {
            previous
        } else {
            let mut changed = get_changed_files(&self.project_root, &previous.base_ref)?;
            changed.apply_ignore_patterns(self.config.compiled_ignore_patterns());
            changed
        };
        let checks = determine_checks(&self.config, &changed_files, &self.exec_root);
        Ok((changed_files, checks))
    }

    /// [`Self::refresh`] on the blocking pool
    async fn refresh_async(
        &self,
        previous: ChangedFiles,
    ) -> Result<(ChangedFiles, Vec<CheckToRun>)> {
        let ctx = self.clone();
        tokio::task::spawn_blocking(move || ctx.refresh(previous)).await?
    }
}

/// Spawns background tasks and owns them, so retry-all can abort them all
struct Tasks {
    tx: mpsc::Sender<TaskEvent>,
    set: JoinSet<()>,
    ctx: TaskCtx,
}

impl Tasks {
    /// Create a task spawner and the receiver for its events. Replacing it
    /// on retry-all drops late results from tasks started before the reset.
    fn new(ctx: TaskCtx) -> (Self, mpsc::Receiver<TaskEvent>) {
        let (tx, rx) = mpsc::channel(TASK_CHANNEL_CAPACITY);
        let tasks = Self {
            tx,
            set: JoinSet::new(),
            ctx,
        };
        (tasks, rx)
    }

    /// Spawn `f(ctx, tx)` as a tracked task
    fn spawn<F, Fut>(&mut self, f: F)
    where
        F: FnOnce(TaskCtx, mpsc::Sender<TaskEvent>) -> Fut,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // Reap finished tasks so the set does not grow without bound
        while self.set.try_join_next().is_some() {}
        self.set.spawn(f(self.ctx.clone(), self.tx.clone()));
    }

    /// Spawn a task running `command` as `check`; `wrap` builds the event
    fn spawn_check(
        &mut self,
        check: CheckToRun,
        command: String,
        wrap: fn(CheckResult) -> TaskEvent,
    ) {
        self.spawn(|ctx, tx| async move {
            let result = ctx.run(&check, &command).await;
            let _ = tx.send(wrap(result)).await;
        });
    }
}

/// Retry `check` after a git refresh. `previous` is restored if the check
/// no longer applies to the refreshed file list.
async fn retry_with_refresh(
    ctx: TaskCtx,
    tx: mpsc::Sender<TaskEvent>,
    check: CheckToRun,
    previous: CheckResult,
    changed_files: ChangedFiles,
) {
    let check = match ctx.refresh_async(changed_files).await {
        Ok((changed_files, checks)) => {
            let Some(new_check) = checks.into_iter().find(|c| c.id() == check.id()) else {
                let _ = tx.send(TaskEvent::CheckNotApplicable(previous)).await;
                return;
            };
            let _ = tx
                .send(TaskEvent::CheckRefreshed {
                    changed_files,
                    check: Box::new(new_check.clone()),
                })
                .await;
            new_check
        }
        // Git refresh failed - run with the existing check
        Err(_) => {
            let _ = tx.send(TaskEvent::GitRefreshFailed).await;
            check
        }
    };
    let result = ctx.run(&check, &check.resolved_command).await;
    let _ = tx.send(TaskEvent::RetryResult(result)).await;
}

/// Restore terminal to normal state (called on exit and panic)
///
/// Errors are intentionally ignored because this is called during cleanup
/// and panic handling - we must attempt restoration regardless of errors.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

/// Copy text to clipboard using OSC 52 escape sequence
/// Works in most modern terminals: iTerm2, kitty, alacritty, Windows Terminal, etc.
fn copy_to_clipboard(text: &str) {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let encoded = STANDARD.encode(text);
    // OSC 52 sequence: \x1b]52;c;{base64}\x07
    print!("\x1b]52;c;{}\x07", encoded);
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// Install panic hook to restore terminal on panic
fn install_panic_hook() {
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        original_hook(panic_info);
    }));
}

/// Start a new check runner and return its handle and event receiver
fn start_runner(
    config: &CiConfig,
    project_root: &Path,
    checks: Vec<CheckToRun>,
) -> (JoinHandle<Result<()>>, mpsc::Receiver<RunnerEvent>) {
    let (event_tx, event_rx) = mpsc::channel::<RunnerEvent>(RUNNER_CHANNEL_CAPACITY);
    let runner = CheckRunner::new(config.clone(), project_root);

    let handle = tokio::spawn(async move { runner.run_checks(checks, event_tx).await });

    (handle, event_rx)
}

#[cfg(debug_assertions)]
fn warn_slow_keyboard(start: std::time::Instant) {
    let elapsed = start.elapsed();
    if elapsed > std::time::Duration::from_millis(1) {
        eprintln!("WARN: Keyboard response took {:?}", elapsed);
    }
}

/// Handle 'r' key: retry selected check with git refresh
fn handle_retry_selected(app: &mut App, tasks: &mut Tasks) -> Action {
    if !app.selected_capabilities().can_retry {
        return Action::Continue;
    }
    let Some(check) = app.selected_check() else {
        return Action::Continue;
    };
    let check = check.clone();
    let previous = app
        .results
        .get(check.id())
        .cloned()
        .unwrap_or_else(|| CheckResult::pending(check.id()));
    let changed_files = app.changed_files.clone();

    // Mark running now (also blocks a second 'r'); git refresh runs off the
    // event loop so large repos do not freeze the UI
    app.reset_check_for_retry(check.id());
    tasks.spawn(|ctx, tx| retry_with_refresh(ctx, tx, check, previous, changed_files));
    Action::Continue
}

/// Handle 't' key: trigger on-demand test
fn handle_trigger_on_demand(app: &mut App, tasks: &mut Tasks) -> Action {
    if !app.selected_capabilities().can_trigger {
        return Action::Continue;
    }
    let Some(check) = app.selected_check() else {
        return Action::Continue;
    };
    let check = check.clone();
    app.trigger_on_demand_check(check.id());
    let command = check.resolved_command.clone();
    tasks.spawn_check(check, command, TaskEvent::RetryResult);
    Action::Continue
}

/// Handle 'A' key: run selected check for all files
fn handle_run_all_files(app: &mut App, tasks: &mut Tasks) -> Action {
    if !app.selected_capabilities().can_run_all_files {
        return Action::Continue;
    }
    let Some(check) = app.selected_check() else {
        return Action::Continue;
    };
    let check = check.clone();
    let all_files_cmd = check.get_command_for_all_files();
    app.reset_check_for_retry(check.id());
    app.set_status_message(StatusKind::Progress, "Running for all files...");
    tasks.spawn_check(check, all_files_cmd, TaskEvent::RetryResult);
    Action::Continue
}

/// Handle 'R' key: retry all checks after a git refresh (runs off the event loop)
fn handle_retry_all(app: &mut App, tasks: &mut Tasks) -> Action {
    if !app.can_retry_all() {
        return Action::Continue;
    }
    let previous = app.changed_files.clone();
    let base_ref = previous.base_ref.clone();
    app.start_refresh();
    tasks.spawn(|ctx, tx| async move {
        let (changed_files, checks) = match ctx.refresh_async(previous).await {
            Ok(refreshed) => refreshed,
            // Git refresh failed - restart with no changed files
            Err(_) => {
                let changed_files = ChangedFiles {
                    files: vec![],
                    base_ref,
                };
                let checks = determine_checks(&ctx.config, &changed_files, &ctx.exec_root);
                (changed_files, checks)
            }
        };
        let _ = tx
            .send(TaskEvent::RetryAllReady {
                changed_files,
                checks,
            })
            .await;
    });
    Action::Continue
}

/// Handle 'x' key: run fix for selected check
fn handle_fix_selected(app: &mut App, tasks: &mut Tasks) -> Action {
    if !app.selected_capabilities().can_fix {
        return Action::Continue;
    }
    let Some(job) = app.get_selected_fix_command() else {
        return Action::Continue;
    };
    app.start_fix();
    tasks.spawn_check(job.check, job.command, TaskEvent::FixResult);
    Action::Continue
}

/// Handle 'X' key: run fix for all failed checks
fn handle_fix_all(app: &mut App, tasks: &mut Tasks) -> Action {
    if !app.can_fix_all() {
        return Action::Continue;
    }
    let fix_commands = app.get_all_fix_commands();
    if fix_commands.is_empty() {
        return Action::Continue;
    }
    app.start_fix_all(fix_commands.len());
    tasks.spawn(|ctx, tx| async move {
        for job in fix_commands {
            let result = ctx.run(&job.check, &job.command).await;
            let _ = tx.send(TaskEvent::FixAllResult(result)).await;
        }
        // Always signal completion, decoupled from per-result send success.
        // Previously completion rode on an is_last flag on the final result;
        // a failed send left fix_all_running=true and disabled r/t/x/X forever.
        let _ = tx.send(TaskEvent::FixAllDone).await;
    });
    Action::Continue
}

/// Handle a key event and return the action for the main loop
fn handle_key_event(app: &mut App, key: KeyEvent, tasks: &mut Tasks) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Quit,
        (KeyCode::Up | KeyCode::Char('k'), _) => {
            app.previous_check();
            Action::Continue
        }
        (KeyCode::Down | KeyCode::Char('j'), _) => {
            app.next_check();
            Action::Continue
        }
        (KeyCode::PageUp, _) => {
            app.scroll_up(PAGE_SCROLL_LINES);
            Action::Continue
        }
        (KeyCode::PageDown, _) => {
            app.scroll_down(PAGE_SCROLL_LINES);
            Action::Continue
        }
        (KeyCode::Char('f'), _) => {
            app.toggle_failed_filter();
            Action::Continue
        }
        (KeyCode::Char('a'), _) => {
            app.show_all();
            Action::Continue
        }
        (KeyCode::Char('r'), KeyModifiers::NONE) => handle_retry_selected(app, tasks),
        (KeyCode::Char('t'), KeyModifiers::NONE) => handle_trigger_on_demand(app, tasks),
        (KeyCode::Char('A'), KeyModifiers::SHIFT) => handle_run_all_files(app, tasks),
        (KeyCode::Char('c'), KeyModifiers::NONE) => {
            if let Some(check) = app.selected_check() {
                copy_to_clipboard(&check.resolved_command);
                app.set_status_message(StatusKind::info(), "Command copied to clipboard");
            }
            Action::Continue
        }
        (KeyCode::Char('e'), KeyModifiers::NONE) => {
            app.toggle_full_command();
            Action::Continue
        }
        (KeyCode::Char('R'), KeyModifiers::SHIFT) => handle_retry_all(app, tasks),
        (KeyCode::Char('x'), KeyModifiers::NONE) => handle_fix_selected(app, tasks),
        (KeyCode::Char('X'), KeyModifiers::SHIFT) => handle_fix_all(app, tasks),
        _ => Action::Continue,
    }
}

/// Apply an event from a background task
fn handle_task_event(app: &mut App, event: TaskEvent) -> Action {
    match event {
        TaskEvent::FixResult(result) => app.finish_fix(result),
        TaskEvent::FixAllResult(result) => app.add_fix_all_result(result),
        TaskEvent::FixAllDone => app.finish_fix_all(),
        TaskEvent::CheckRefreshed {
            changed_files,
            check,
        } => app.replace_check(changed_files, *check),
        TaskEvent::CheckNotApplicable(previous) => {
            app.set_retry_result(previous);
            app.set_status_message(
                StatusKind::Error,
                "Check no longer applicable after refresh",
            );
        }
        TaskEvent::GitRefreshFailed => app.set_status_message(
            StatusKind::Error,
            "Git refresh failed - retrying with previous file list",
        ),
        TaskEvent::RetryResult(result) => {
            app.set_retry_result(result);
            app.finish_progress();
        }
        TaskEvent::RetryAllReady {
            changed_files,
            checks,
        } => {
            return Action::RestartRunner {
                new_changed_files: changed_files,
                new_checks: checks,
            }
        }
    }
    Action::Continue
}

/// Handle a message from the event loop and update app state
/// All state changes go through this function via &mut App
fn handle_message(app: &mut App, msg: Message, tasks: &mut Tasks) -> Action {
    match msg {
        Message::KeyPress(key) => {
            // Keyboard response timing instrumentation
            #[cfg(debug_assertions)]
            let start = std::time::Instant::now();

            // Any key dismisses the status message, then still acts
            // (cleared first so the handler may set a fresh message)
            if app.view.status_message.is_some() {
                app.clear_status_message();
            }

            // Dispatch to key handler
            let action = handle_key_event(app, key, tasks);

            #[cfg(debug_assertions)]
            warn_slow_keyboard(start);

            action
        }
        Message::Resize => {
            app.needs_redraw = true;
            Action::Continue
        }
        Message::RunnerEvent(event) => {
            app.handle_runner_event(event);
            Action::Continue
        }
        Message::SystemStats(stats) => {
            app.update_stats(stats.cpu_usage, stats.mem_used, stats.mem_total);
            Action::Continue
        }
        Message::Task(event) => handle_task_event(app, event),
        Message::StatusExpired => {
            app.expire_status_message(std::time::Instant::now());
            Action::Continue
        }
    }
}

pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
    exec_root: PathBuf,
) -> Result<()> {
    // Install panic hook to restore terminal on panic
    install_panic_hook();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Get current branch name
    let branch_name = current_branch(&project_root).unwrap_or_else(|_| "unknown".to_string());

    // Create app state
    let mut app = App::new(config.clone(), changed_files, checks.clone(), branch_name);

    // Start the runner in background
    let (mut runner_handle, mut event_rx) = start_runner(&config, &exec_root, checks);

    // Spawner for fix/retry/refresh tasks and the receiver for their events
    let project_root = Arc::new(project_root);
    let (mut tasks, mut task_rx) = Tasks::new(TaskCtx {
        project_root: Arc::clone(&project_root),
        exec_root: Arc::new(exec_root.clone()),
        config: Arc::new(config.clone()),
    });

    // Start background stats worker - runs sysinfo queries without blocking UI
    let (stats_tx, mut stats_rx) = mpsc::channel::<SystemStats>(STATS_CHANNEL_CAPACITY);
    let stats_handle = spawn_stats_worker(stats_tx);

    // Start dedicated OS thread for keyboard input
    // CRITICAL: Using std::thread ensures keyboard events are processed by the OS
    // scheduler even when Tokio is starved of CPU time by Docker containers.
    let keyboard_shutdown = Arc::new(AtomicBool::new(false));
    let (mut input_rx, keyboard_thread) = spawn_keyboard_thread(Arc::clone(&keyboard_shutdown));

    // Main event loop - uses tokio::select! for event-driven responsiveness
    //
    // Architecture for responsive keyboard handling under high CPU load:
    // 1. biased; ensures keyboard events are checked first (highest priority)
    // 2. Each channel becomes a select! branch - wakes on any event
    // 3. All state transitions go through handle_message with explicit Message enum
    // 4. Render only if state changed (dirty flag)
    loop {
        // Status message auto-dismiss deadline (disabled branch when None)
        let status_deadline = app.status_message_deadline();

        // Wait for the next event from any channel
        // biased; ensures keyboard is checked first for immediate responsiveness
        let msg = tokio::select! {
            biased;

            // Keyboard (and resize) events have highest priority
            Some(event) = input_rx.recv() => match event {
                Event::Key(key) => Message::KeyPress(key),
                _ => Message::Resize,
            },

            // Status message TTL (ahead of busy channels so it cannot starve)
            _ = tokio::time::sleep_until(
                status_deadline.unwrap_or_else(std::time::Instant::now).into()
            ), if status_deadline.is_some() => Message::StatusExpired,

            // Runner lifecycle/status events
            Some(event) = event_rx.recv() => Message::RunnerEvent(event),

            // System stats from background worker
            Some(stats) = stats_rx.recv() => Message::SystemStats(stats),

            // Fix / retry / refresh task events
            Some(event) = task_rx.recv() => Message::Task(event),

            // All channels closed - exit
            else => break,
        };

        // Handle the message and get the action
        match handle_message(&mut app, msg, &mut tasks) {
            Action::Quit => break,
            Action::RestartRunner {
                new_changed_files,
                new_checks,
            } => {
                // Abort old runner and background tasks (kill_on_drop stops
                // their docker processes), then start fresh. The new channel
                // drops late results from tasks started before the reset.
                runner_handle.abort();
                tasks.set.abort_all();
                (tasks, task_rx) = Tasks::new(tasks.ctx.clone());
                app.reset_for_retry(new_changed_files, new_checks.clone());
                let (new_handle, new_rx) = start_runner(&config, &exec_root, new_checks);
                runner_handle = new_handle;
                event_rx = new_rx;
            }
            Action::Continue => {}
        }

        // Render if state changed
        if app.needs_redraw {
            terminal.draw(|f| dashboard::render(&mut app, f))?;
            app.needs_redraw = false;
        }
    }

    // Cleanup - signal keyboard thread to shutdown and wait for it
    keyboard_shutdown.store(true, Ordering::Relaxed);
    runner_handle.abort();
    tasks.set.abort_all();
    stats_handle.abort();

    // Wait for keyboard thread to finish (with timeout to avoid hanging)
    let _ = keyboard_thread.join();

    // Restore terminal: use the backend's own stdout handle to ensure
    // LeaveAlternateScreen goes through the same IO path as all TUI writes.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    // Drop terminal before printing to ensure all backend IO is flushed
    drop(terminal);

    // Print summary
    print_summary(&app);

    // Non-zero exit on failure so `ci-tui && git push` is safe (terminal already restored)
    let code = app.exit_code();
    if code != 0 {
        std::process::exit(code);
    }

    Ok(())
}

fn print_summary(app: &App) {
    let counts = app.count_by_status();
    let (passed, failed) = (counts.passed, counts.failed);
    let total = passed + failed;

    // Format elapsed time
    let elapsed = app.elapsed_time();
    let elapsed_str = dashboard::format_elapsed(elapsed);

    if failed == 0 {
        println!(
            "\n\x1b[32m✓ All {} checks passed in {}\x1b[0m",
            total, elapsed_str
        );
        return;
    }

    println!(
        "\n\x1b[31m✗ {} of {} checks failed in {}\x1b[0m",
        failed, total, elapsed_str
    );

    // Show failed checks
    for (id, result) in &app.results {
        if result.status == crate::runner::CheckStatus::Failed {
            println!("  - {}", id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> CiConfig {
        serde_yaml::from_str(
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
    checks:
      php-lint:
        name: PHP syntax check
        command: php-lint {files}
        triggers:
          file_pattern: php
"#,
        )
        .expect("Failed to parse test config")
    }

    /// Build a task spawner for handle_message tests. project_root points
    /// at a nonexistent path so git operations fail deterministically.
    fn make_test_tasks(config: &CiConfig) -> (Tasks, mpsc::Receiver<TaskEvent>) {
        Tasks::new(TaskCtx {
            project_root: Arc::new(PathBuf::from("/nonexistent-ci-tui-test-path")),
            exec_root: Arc::new(PathBuf::from("/nonexistent-ci-tui-test-path")),
            config: Arc::new(config.clone()),
        })
    }

    fn make_test_app(config: &CiConfig) -> App {
        make_test_app_with_checks(config, vec![])
    }

    fn make_test_app_with_checks(config: &CiConfig, checks: Vec<CheckToRun>) -> App {
        let changed_files = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: "development".to_string(),
        };
        App::new(config.clone(), changed_files, checks, "main".to_string())
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        }
    }

    fn make_test_check(id: &str, group: &str) -> crate::checks::CheckToRun {
        crate::checks::CheckToRun {
            id: id.to_string(),
            group: group.to_string(),
            definition: crate::config::CheckDefinition {
                name: id.to_string(),
                command: format!("{} {{files}}", id),
                service: None,
                container: None,
                fix_command: None,
                triggers: None,
                on_demand: false,
                env: std::collections::HashMap::new(),
            },
            service: Some("php".to_string()),
            files: crate::checks::CheckFiles::Files(vec!["src/Foo.php".to_string()]),
            resolved_command: format!("{} src/Foo.php", id),
            resolved_fix_command: None,
        }
    }

    #[tokio::test]
    async fn test_retry_selected_git_failure_sets_status_message() {
        let config = test_config();
        let mut app = make_test_app_with_checks(&config, vec![make_test_check("php-lint", "fast")]);
        app.results.get_mut("php-lint").unwrap().status = crate::runner::CheckStatus::Passed;
        let (mut tasks, mut rx) = make_test_tasks(&config); // project_root does not exist -> git fails

        let action = handle_retry_selected(&mut app, &mut tasks);

        assert!(matches!(action, Action::Continue));
        // Marked running right away; git refresh happens in the background
        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            crate::runner::CheckStatus::Running
        );
        let event = rx.recv().await.expect("refresh task must report");
        assert!(matches!(event, TaskEvent::GitRefreshFailed));
        handle_task_event(&mut app, event);
        assert!(
            app.view.status_message.is_some(),
            "git refresh failure must surface a status message"
        );
    }

    #[tokio::test]
    async fn test_retry_all_refreshes_in_background() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, mut rx) = make_test_tasks(&config);

        let action = handle_retry_all(&mut app, &mut tasks);
        assert!(matches!(action, Action::Continue));

        let event = rx.recv().await.expect("refresh task must report");
        let action = handle_task_event(&mut app, event);
        assert!(matches!(action, Action::RestartRunner { .. }));
    }

    fn cli_files() -> ChangedFiles {
        ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: crate::git::CLI_FILES_BASE_REF.to_string(),
        }
    }

    #[tokio::test]
    async fn test_retry_all_in_files_mode_keeps_file_list() {
        let config = test_config();
        let mut app = App::new(config.clone(), cli_files(), vec![], "main".to_string());
        let (mut tasks, mut rx) = make_test_tasks(&config); // git would fail here

        handle_retry_all(&mut app, &mut tasks);

        let event = rx.recv().await.expect("refresh task must report");
        let TaskEvent::RetryAllReady {
            changed_files,
            checks,
        } = event
        else {
            panic!("expected RetryAllReady, got {:?}", event);
        };
        assert_eq!(changed_files.files, vec!["src/Foo.php".to_string()]);
        assert_eq!(changed_files.base_ref, crate::git::CLI_FILES_BASE_REF);
        assert_eq!(checks.len(), 1, "php-lint must still match the file");
    }

    #[tokio::test]
    async fn test_retry_selected_in_files_mode_skips_git() {
        let config = test_config();
        let checks = vec![make_test_check("php-lint", "fast")];
        let mut app = App::new(config.clone(), cli_files(), checks, "main".to_string());
        app.results.get_mut("php-lint").unwrap().status = crate::runner::CheckStatus::Passed;
        let (mut tasks, mut rx) = make_test_tasks(&config); // git would fail here

        handle_retry_selected(&mut app, &mut tasks);

        let event = rx.recv().await.expect("refresh task must report");
        assert!(
            matches!(event, TaskEvent::CheckRefreshed { .. }),
            "--files mode must not report a git refresh failure, got {:?}",
            event
        );
    }

    #[test]
    fn test_check_not_applicable_restores_previous_result() {
        let config = test_config();
        let changed_files = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: "development".to_string(),
        };
        let checks = vec![make_test_check("php-lint", "fast")];
        let mut app = App::new(config, changed_files, checks, "main".to_string());
        let mut previous = CheckResult::pending("php-lint");
        previous.status = crate::runner::CheckStatus::Passed;

        handle_task_event(&mut app, TaskEvent::CheckNotApplicable(previous));

        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            crate::runner::CheckStatus::Passed
        );
        assert!(app.view.status_message.is_some());
    }

    #[tokio::test]
    async fn test_abort_all_stops_spawned_tasks() {
        let config = test_config();
        let (mut tasks, mut rx) = make_test_tasks(&config);
        tasks.spawn(|_, tx| async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let _ = tx.send(TaskEvent::FixAllDone).await;
        });

        tasks.set.abort_all();
        while tasks.set.join_next().await.is_some() {}
        drop(tasks);

        assert!(rx.recv().await.is_none(), "aborted task must not send");
    }

    #[tokio::test]
    async fn test_status_expired_clears_due_message() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        app.set_status_message(StatusKind::info(), "Command copied to clipboard");
        app.view.status_message.as_mut().unwrap().kind = StatusKind::Info {
            expires_at: std::time::Instant::now(),
        };
        handle_message(&mut app, Message::StatusExpired, &mut tasks);
        assert!(app.view.status_message.is_none(), "due message cleared");

        app.set_status_message(StatusKind::info(), "fresh");
        handle_message(&mut app, Message::StatusExpired, &mut tasks);
        assert!(
            app.view.status_message.is_some(),
            "stale wakeup keeps fresh message"
        );
    }

    #[tokio::test]
    async fn test_retry_all_ignores_second_press_while_refreshing() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_retry_all(&mut app, &mut tasks);
        handle_retry_all(&mut app, &mut tasks);

        assert_eq!(
            tasks.set.len(),
            1,
            "second R must not spawn another refresh"
        );
    }

    #[test]
    fn test_run_all_files_progress_cleared_by_result() {
        let config = test_config();
        let mut app = make_test_app(&config);
        app.set_status_message(StatusKind::Progress, "Running for all files...");

        handle_task_event(
            &mut app,
            TaskEvent::RetryResult(CheckResult::pending("php-lint")),
        );

        assert!(app.view.status_message.is_none());
    }

    #[tokio::test]
    async fn test_quit_key_works_while_status_message_shown() {
        let config = test_config();
        let mut app = make_test_app(&config);
        app.set_status_message(StatusKind::info(), "Command copied to clipboard");
        let (mut tasks, _rx) = make_test_tasks(&config);

        let key = press(KeyCode::Char('q'), KeyModifiers::NONE);
        let action = handle_message(&mut app, Message::KeyPress(key), &mut tasks);

        assert!(
            matches!(action, Action::Quit),
            "q must quit even while a status message is displayed"
        );
    }

    #[tokio::test]
    async fn test_ctrl_c_works_while_status_message_shown() {
        let config = test_config();
        let mut app = make_test_app(&config);
        app.set_status_message(StatusKind::info(), "some message");
        let (mut tasks, _rx) = make_test_tasks(&config);

        let key = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let action = handle_message(&mut app, Message::KeyPress(key), &mut tasks);

        assert!(matches!(action, Action::Quit));
    }

    #[tokio::test]
    async fn test_other_key_dismisses_status_message() {
        let config = test_config();
        let mut app = make_test_app_with_checks(
            &config,
            vec![
                make_test_check("php-lint", "fast"),
                make_test_check("phpstan", "fast"),
            ],
        );
        app.set_status_message(StatusKind::info(), "some message");
        let (mut tasks, _rx) = make_test_tasks(&config);

        let key = press(KeyCode::Char('j'), KeyModifiers::NONE);
        let action = handle_message(&mut app, Message::KeyPress(key), &mut tasks);

        assert!(matches!(action, Action::Continue));
        assert!(
            app.view.status_message.is_none(),
            "non-quit key clears the message"
        );
        assert_eq!(
            app.view.selected_check, 1,
            "key still acts while dismissing the message"
        );
    }

    #[test]
    fn test_handle_key_event_quit() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        let action = handle_key_event(
            &mut app,
            press(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(matches!(action, Action::Quit));
    }

    #[test]
    fn test_handle_key_event_toggle_filter() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        let action = handle_key_event(
            &mut app,
            press(KeyCode::Char('f'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(matches!(action, Action::Continue));
        assert_eq!(app.view.status_filter, crate::ui::app::StatusFilter::Failed);
    }

    #[test]
    fn test_handle_key_event_unknown_key_continues() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        let action = handle_key_event(
            &mut app,
            press(KeyCode::Char('z'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(matches!(action, Action::Continue));
    }

    #[test]
    fn test_handle_message_runner_event_all_finished() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        let action = handle_message(
            &mut app,
            Message::RunnerEvent(RunnerEvent::AllFinished),
            &mut tasks,
        );

        assert!(matches!(action, Action::Continue));
        assert!(app.run.all_finished);
    }

    #[test]
    fn test_handle_message_retry_result_stored() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        let result = CheckResult {
            check_id: "php-lint".to_string(),
            status: crate::runner::CheckStatus::Passed,
            output: "OK".to_string(),
            error_output: String::new(),
            duration_ms: 42,
            started_at: None,
            finished_at: None,
        };

        handle_message(
            &mut app,
            Message::Task(TaskEvent::RetryResult(result)),
            &mut tasks,
        );

        assert_eq!(
            app.results.get("php-lint").map(|r| r.status.clone()),
            Some(crate::runner::CheckStatus::Passed)
        );
    }

    #[tokio::test]
    async fn test_fix_all_done_clears_running_flag() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        app.start_fix_all(2);
        assert!(app.fix.all_running);

        // Done arrives even if individual result sends were lost
        let action = handle_message(&mut app, Message::Task(TaskEvent::FixAllDone), &mut tasks);

        assert!(matches!(action, Action::Continue));
        assert!(!app.fix.all_running, "Done must clear fix_all_running");
    }

    #[tokio::test]
    async fn test_fix_all_result_does_not_finish_run() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        app.start_fix_all(2);

        let result = CheckResult::pending("some-check");
        handle_message(
            &mut app,
            Message::Task(TaskEvent::FixAllResult(result)),
            &mut tasks,
        );

        assert!(
            app.fix.all_running,
            "intermediate results must not finish the run"
        );
        assert_eq!(app.fix.all_results.len(), 1);
    }
}
