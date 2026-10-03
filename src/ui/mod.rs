//! Terminal UI using ratatui for interactive check execution.
//!
//! This module implements the main TUI event loop and coordinates between
//! user input, check execution, and rendering. It provides lifecycle-event
//! feedback during check execution with keyboard-driven navigation; check
//! output is streamed live while a check runs (runner and retry /
//! on-demand / run-all-files tasks), auto-following the tail.
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
//! - [`external`]: Output in `$PAGER` / `$EDITOR` (`o` / `O`) and log saving (`w`)
//! - [`notify`]: End-of-run bell + desktop notification (`notify` / `--notify`)

pub mod app;
pub mod dashboard;
pub mod external;
pub mod notify;

use crate::cache::{stamp_keys, ResultCache};
use crate::checks::{select_checks, CheckToRun, Decision, Selected};
use crate::config::CiConfig;
use crate::git::{current_branch, get_changed_files, get_staged_files, ChangedFiles};
use crate::runner::{
    run_check_cancellable, run_check_with_command_with_executor, CancelRegistry, CheckResult,
    CheckRunner, CheckStatus, OutputSink, RealCommandExecutor, RunnerEvent,
};
use crate::watch::WatchBatch;
use anyhow::Result;
use app::{App, GroupSetup, StatusKind};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use external::Viewer;
use ratatui::prelude::*;
use std::collections::HashSet;
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
/// Redraw tick for running checks' live elapsed timers (tenths resolution)
const LIVE_TIMER_INTERVAL: Duration = Duration::from_millis(100);
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
/// Lines to scroll per mouse wheel tick (smaller than a page scroll)
const MOUSE_SCROLL_LINES: usize = 3;

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
    Mouse(MouseEvent),
    RunnerEvent(RunnerEvent),
    SystemStats(SystemStats),
    Task(TaskEvent),
    /// Status message auto-dismiss deadline reached
    StatusExpired,
    /// Live elapsed timers advanced
    Tick,
    /// `--watch`: checks a batch of saves affects ([`crate::watch`])
    Watch(WatchBatch),
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
    /// Retry / on-demand result
    RetryResult(CheckResult),
    /// Result the result cache does not record: a run-all-files run (its
    /// command is not the one the check's key covers), or a single run
    /// whose group pre-commands failed (the check did not run)
    UnrecordedResult(CheckResult),
    /// Live output chunk ([`RunnerEvent::CheckOutput`]) of a task-run check
    Output(RunnerEvent),
    /// Git refresh for retry-all finished; restart the runner
    RetryAllReady {
        changed_files: ChangedFiles,
        checks: Vec<CheckToRun>,
        /// Test discovery warnings from the refresh
        warnings: Vec<String>,
    },
}

/// Read keyboard, mouse and resize events and send them through a channel
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
            // Only the mouse kinds handle_mouse_event acts on; drop Moved/Drag/Up
            // and other buttons here rather than churning the event channel
            Ok(
                event @ Event::Mouse(MouseEvent {
                    kind:
                        MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::ScrollUp
                        | MouseEventKind::ScrollDown,
                    ..
                }),
            ) => event,
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

/// The keyboard thread and its stop flag; stopped while a viewer owns the
/// terminal, then started afresh
struct KeyboardThread {
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl KeyboardThread {
    fn start() -> (Self, mpsc::UnboundedReceiver<Event>) {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (rx, handle) = spawn_keyboard_thread(Arc::clone(&shutdown));
        let thread = Self {
            shutdown,
            handle: Some(handle),
        };
        (thread, rx)
    }

    /// Stop reading and wait for the thread (at most one poll); idempotent
    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
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

/// Redraw tick for live elapsed timers. Own, not the stats worker's:
/// sysinfo stalls under high CPU load
fn live_timer_tick() -> tokio::time::Interval {
    let mut tick = tokio::time::interval(LIVE_TIMER_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick
}

/// Action result from handle_message
enum Action {
    Continue,
    Quit,
    /// Suspend the TUI and show a check's output in `$PAGER` / `$EDITOR`
    OpenViewer(ViewerRequest),
    RestartRunner {
        new_changed_files: ChangedFiles,
        new_checks: Vec<CheckToRun>,
        warnings: Vec<String>,
    },
}

/// What `o` / `O` open: the selected check's plain-text log in `viewer`
struct ViewerRequest {
    viewer: Viewer,
    check_id: String,
    text: String,
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
    /// Cancel switches shared with the runner ('s' key)
    cancels: CancelRegistry,
    /// Result cache key root ([`ResultCache::key_root`]); refreshed checks
    /// get keys only with it
    key_root: Option<Arc<Path>>,
}

impl TaskCtx {
    /// Context sharing the runner's `cancels`
    fn new(
        project_root: PathBuf,
        exec_root: &Path,
        config: Arc<CiConfig>,
        cancels: &CancelRegistry,
        key_root: Option<Arc<Path>>,
    ) -> Self {
        Self {
            project_root: Arc::new(project_root),
            exec_root: Arc::new(exec_root.to_path_buf()),
            config,
            cancels: cancels.clone(),
            key_root,
        }
    }

    /// Run `command` as `check` (check's container override and env apply),
    /// streaming live output to `sink`. The only runner call site here.
    async fn run(&self, check: &CheckToRun, command: &str, sink: &OutputSink) -> CheckResult {
        run_check_with_command_with_executor(
            check,
            command,
            &self.exec_root,
            &self.config.runner,
            &RealCommandExecutor,
            self.config.max_output_lines,
            sink,
        )
        .await
    }

    /// [`Self::run_streaming`], after the group's pre-commands with `setup`
    /// (see [`GroupSetup::Run`]). `None` when one failed: the check did not
    /// run, its failed result went to `tx` as
    /// [`TaskEvent::UnrecordedResult`] (a cached pass stays).
    async fn run_after_setup(
        &self,
        check: &CheckToRun,
        command: &str,
        setup: bool,
        tx: &mpsc::Sender<TaskEvent>,
    ) -> Option<CheckResult> {
        if setup && !self.run_setup(check.group(), tx).await {
            let result = CheckResult::setup_failed(check.id(), check.group());
            let _ = tx.send(TaskEvent::UnrecordedResult(result)).await;
            return None;
        }
        Some(self.run_streaming(check, command, tx).await)
    }

    /// [`Self::run`], streaming live output to `tx` as [`TaskEvent::Output`]
    /// (all chunks are sent before this returns, so before the result),
    /// after a `CheckStarted` that starts the live timer
    async fn run_streaming(
        &self,
        check: &CheckToRun,
        command: &str,
        tx: &mpsc::Sender<TaskEvent>,
    ) -> CheckResult {
        let started = RunnerEvent::CheckStarted {
            check_id: check.id().to_string(),
        };
        let _ = tx.send(TaskEvent::Output(started)).await;
        let (out_tx, mut out_rx) = mpsc::channel(RUNNER_CHANNEL_CAPACITY);
        let run = async move {
            // Sink dropped at the end of this block, ending `forward`
            let sink = OutputSink::new(check.id(), out_tx);
            self.run(check, command, &sink).await
        };
        let (result, ()) = tokio::join!(run, forward_output(&mut out_rx, tx));
        result
    }

    /// Run `group`'s pre-commands, their events to `tx` as
    /// [`TaskEvent::Output`]; false when one failed
    async fn run_setup(&self, group: &str, tx: &mpsc::Sender<TaskEvent>) -> bool {
        let (out_tx, mut out_rx) = mpsc::channel(RUNNER_CHANNEL_CAPACITY);
        let runner = CheckRunner::new(Arc::clone(&self.config), &self.exec_root);
        // `out_tx` dropped at the end of this block, ending `forward`
        let setup = async move { runner.run_group_setup(group, &out_tx).await };
        let (ok, ()) = tokio::join!(setup, forward_output(&mut out_rx, tx));
        ok
    }

    /// [`Self::run_after_setup`], cancellable with 's' (then reports
    /// `Cancelled`); sends the result to `tx` as `done(result)`
    async fn run_cancellable(
        &self,
        check: &CheckToRun,
        command: &str,
        setup: bool,
        tx: &mpsc::Sender<TaskEvent>,
        done: fn(CheckResult) -> TaskEvent,
    ) {
        let run = self.run_after_setup(check, command, setup, tx);
        if let Some(result) = run_check_cancellable(&self.cancels, check.id(), run).await {
            let _ = tx.send(done(result)).await;
        }
    }

    /// Checks `changed_files` selects (with `only`, just that one), with
    /// fresh result cache keys. Blocking: keys read the files.
    fn select(&self, changed_files: &ChangedFiles, only: Option<&str>) -> Selected {
        let mut selected = select_checks(&self.config, changed_files, &self.exec_root, None);
        selected
            .checks
            .retain(|c| only.is_none_or(|id| c.id() == id));
        if let Some(root) = &self.key_root {
            let checks = &mut selected.checks;
            stamp_keys(checks, &self.config, changed_files, root, &self.exec_root);
        }
        selected
    }

    /// Re-detect changed files and matching checks ([`Self::select`]).
    /// Blocking (git + file system): call from `spawn_blocking`.
    /// `--files` lists are kept as-is (no git base to diff against); only
    /// the checks are re-determined. `--staged` re-reads the git index.
    fn refresh(
        &self,
        previous: ChangedFiles,
        only: Option<&str>,
    ) -> Result<(ChangedFiles, Selected)> {
        let changed_files = if previous.is_cli_files() {
            previous
        } else {
            let mut changed = self.redetect(&previous)?;
            changed.apply_ignore_patterns(self.config.compiled_ignore_patterns());
            changed
        };
        let selected = self.select(&changed_files, only);
        Ok((changed_files, selected))
    }

    /// Git re-detection for [`Self::refresh`]: the index for `--staged`,
    /// else a diff against the previous base ref.
    fn redetect(&self, previous: &ChangedFiles) -> Result<ChangedFiles> {
        if previous.is_staged() {
            get_staged_files(&self.project_root)
        } else {
            get_changed_files(&self.project_root, &previous.base_ref)
        }
    }

    /// [`Self::refresh`] on the blocking pool
    async fn refresh_async(
        &self,
        previous: ChangedFiles,
        only: Option<String>,
    ) -> Result<(ChangedFiles, Selected)> {
        let ctx = self.clone();
        tokio::task::spawn_blocking(move || ctx.refresh(previous, only.as_deref())).await?
    }
}

/// Forward runner output events to `tx` as [`TaskEvent::Output`] until all
/// senders are dropped
async fn forward_output(rx: &mut mpsc::Receiver<RunnerEvent>, tx: &mpsc::Sender<TaskEvent>) {
    while let Some(event) = rx.recv().await {
        let _ = tx.send(TaskEvent::Output(event)).await;
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

    /// Spawn a fix run of `command` for `check`. Output is not streamed
    /// (the fix result shows once done). Cancellable like
    /// [`Self::spawn_check`], but 's' only acts on a `Running` check, which
    /// a fixed (failed) check is not.
    fn spawn_fix(&mut self, check: CheckToRun, command: String) {
        self.spawn(|ctx, tx| async move {
            let sink = OutputSink::none();
            let run = ctx.run(&check, &command, &sink);
            let result = run_check_cancellable(&ctx.cancels, check.id(), run).await;
            let _ = tx.send(TaskEvent::FixResult(result)).await;
        });
    }

    /// Spawn a task running `command` as `check` (after its group's
    /// pre-commands with `setup`), streaming its output, cancellable with
    /// 's'; reports the result as `done(result)` (a failed setup as
    /// [`TaskEvent::UnrecordedResult`]).
    fn spawn_check(
        &mut self,
        check: CheckToRun,
        command: String,
        setup: bool,
        done: fn(CheckResult) -> TaskEvent,
    ) {
        self.spawn(|ctx, tx| async move {
            ctx.run_cancellable(&check, &command, setup, &tx, done)
                .await;
        });
    }
}

/// Retry `check` after a git refresh (and its group's pre-commands with
/// `setup`). `previous` is restored if the check no longer applies to the
/// refreshed file list. With `auto_only` (`--watch`), a check the refresh
/// leaves on-demand / skipped shows that instead of running. Cancellable
/// with 's' from the start (the check shows as running during the refresh
/// too).
async fn retry_with_refresh(
    ctx: TaskCtx,
    tx: mpsc::Sender<TaskEvent>,
    check: CheckToRun,
    previous: CheckResult,
    changed_files: ChangedFiles,
    setup: bool,
    auto_only: bool,
) {
    let check_id = check.id().to_string();
    let run = refresh_and_run(&ctx, &tx, check, previous, changed_files, setup, auto_only);
    // None: not applicable or failed setup, already reported
    if let Some(result) = run_check_cancellable(&ctx.cancels, &check_id, run).await {
        let _ = tx.send(TaskEvent::RetryResult(result)).await;
    }
}

/// Body of [`retry_with_refresh`]; `None` when the check no longer applies
/// (or no longer runs automatically with `auto_only`) or its setup failed
/// (see [`TaskCtx::run_after_setup`])
async fn refresh_and_run(
    ctx: &TaskCtx,
    tx: &mpsc::Sender<TaskEvent>,
    check: CheckToRun,
    previous: CheckResult,
    changed_files: ChangedFiles,
    setup: bool,
    auto_only: bool,
) -> Option<CheckResult> {
    let only = Some(check.id().to_string());
    let check = match ctx.refresh_async(changed_files, only).await {
        Ok((changed_files, selected)) => {
            let Some(new_check) = selected.checks.into_iter().find(|c| c.id() == check.id()) else {
                let _ = tx.send(TaskEvent::CheckNotApplicable(previous)).await;
                return None;
            };
            let _ = tx
                .send(TaskEvent::CheckRefreshed {
                    changed_files,
                    check: Box::new(new_check.clone()),
                })
                .await;
            if auto_only && new_check.decision() != Decision::Run {
                let result = app::initial_result_for_check(&new_check);
                let _ = tx.send(TaskEvent::UnrecordedResult(result)).await;
                return None;
            }
            new_check
        }
        // Git refresh failed - run with the existing check
        Err(_) => {
            let _ = tx.send(TaskEvent::GitRefreshFailed).await;
            check
        }
    };
    let command = &check.resolved_command;
    ctx.run_after_setup(&check, command, setup, tx).await
}

/// Restore terminal to normal state (called on exit and panic)
///
/// Errors are intentionally ignored because this is called during cleanup
/// and panic handling - we must attempt restoration regardless of errors.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

/// Restores the terminal when dropped. Covers any early return via `?`
/// between terminal setup and the explicit cleanup at the end of `run()`
/// (e.g. `Terminal::new` or a mid-loop `terminal.draw` failing) that would
/// otherwise leave the shell in raw mode / alt screen / mouse capture.
/// `restore_terminal` is idempotent (errors ignored), so it running again
/// on the normal success path is harmless.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Enter the TUI's terminal modes on stdout. The guard restores the
/// terminal when dropped, also on a setup failure after raw mode is on.
fn setup_terminal() -> io::Result<(TerminalGuard, Terminal<CrosstermBackend<io::Stdout>>)> {
    let guard = TerminalGuard;
    let mut stdout = io::stdout();
    enter_tui(&mut stdout)?;
    Ok((guard, Terminal::new(CrosstermBackend::new(stdout))?))
}

/// `--watch` batches receiver ([`crate::watch`]) and, if `watch`, the
/// watcher feeding it (dropping it stops watching); errors when the
/// watcher cannot start
fn start_watch(
    watch: bool,
    cwd: &Path,
    config: &Arc<CiConfig>,
    exec_root: &Path,
) -> Result<(
    Option<crate::watch::Watch>,
    mpsc::UnboundedReceiver<WatchBatch>,
)> {
    let (tx, rx) = mpsc::unbounded_channel();
    let start = || crate::watch::start(cwd, Arc::clone(config), exec_root.to_path_buf(), tx);
    Ok((watch.then(start).transpose()?, rx))
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

/// Enter the TUI's terminal modes: raw mode, alternate screen, mouse capture
fn enter_tui(w: &mut impl io::Write) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(w, EnterAlternateScreen, EnableMouseCapture)
}

/// Leave the modes [`enter_tui`] set
fn leave_tui(w: &mut impl io::Write) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(w, LeaveAlternateScreen, DisableMouseCapture)
}

/// A pager / editor running on the blocking pool while the TUI is suspended
type ViewerTask = (Viewer, JoinHandle<Result<(), String>>);

/// Write `request`'s log into the session's private viewer dir (created on
/// first use, removed at TUI exit)
fn viewer_file(
    dir: &mut Option<tempfile::TempDir>,
    request: &ViewerRequest,
) -> io::Result<PathBuf> {
    if dir.is_none() {
        *dir = Some(external::viewer_dir()?);
    }
    let dir = dir.as_ref().expect("just created");
    external::write_viewer_file(dir.path(), &request.check_id, &request.text)
}

/// Suspend the TUI and run `viewer` on `path` in the background, so the
/// main loop keeps draining check events (a stalled channel would block
/// running checks). The caller stops the keyboard thread first so keys
/// reach the viewer.
fn start_viewer(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    viewer: Viewer,
    path: PathBuf,
) -> io::Result<ViewerTask> {
    leave_tui(terminal.backend_mut())?;
    terminal.show_cursor()?;
    let command = viewer.command();
    let task = tokio::task::spawn_blocking(move || external::run_viewer(&command, &path));
    Ok((viewer, task))
}

/// Restore the TUI after the viewer exited; a failed run (not found,
/// non-zero exit) becomes an error status message. `Err` only when the
/// terminal cannot be restored.
fn finish_viewer(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    viewer: Viewer,
    outcome: Result<Result<(), String>, tokio::task::JoinError>,
) -> io::Result<()> {
    enter_tui(terminal.backend_mut())?;
    // Drop input buffered for the viewer (e.g. read ahead by crossterm
    // before it started), so it cannot replay into the TUI (a `q` meant for
    // `less` quitting ci-tui)
    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    terminal.clear()?;
    if let Err(message) = outcome.unwrap_or_else(|e| Err(e.to_string())) {
        let label = viewer.label();
        app.set_status_message(StatusKind::Error, format!("{label} failed: {message}"));
    }
    Ok(())
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
    cancels: &CancelRegistry,
) -> (JoinHandle<Result<()>>, mpsc::Receiver<RunnerEvent>) {
    let (event_tx, event_rx) = mpsc::channel::<RunnerEvent>(RUNNER_CHANNEL_CAPACITY);
    let runner = CheckRunner::new(config.clone(), project_root).with_cancels(cancels.clone());

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

/// The selected check's id and plain-text log, or the status message why
/// there is none (group header / pre-command row, no output yet)
fn selected_log(app: &App) -> Result<(String, String), &'static str> {
    let check = app
        .selected_check()
        .ok_or("Select a check to use its output")?;
    app.results
        .get(check.id())
        .and_then(|result| external::check_log_text(&check.resolved_command, result))
        .map(|text| (check.id().to_string(), text))
        .ok_or("No output yet for this check")
}

/// Handle 'o' / 'O' key: open the selected check's output in `viewer`
/// (the main loop suspends the TUI for it)
fn handle_open_output(app: &mut App, viewer: Viewer) -> Action {
    match selected_log(app) {
        Ok((check_id, text)) => Action::OpenViewer(ViewerRequest {
            viewer,
            check_id,
            text,
        }),
        Err(message) => {
            app.set_status_message(StatusKind::info(), message);
            Action::Continue
        }
    }
}

/// Handle 'w' key: save the selected check's output to
/// `.ci-tui/logs/<check>.log` under the project root
fn handle_save_log(app: &mut App, tasks: &Tasks) -> Action {
    let (check_id, text) = match selected_log(app) {
        Ok(log) => log,
        Err(message) => {
            app.set_status_message(StatusKind::info(), message);
            return Action::Continue;
        }
    };
    let root = tasks.ctx.project_root.as_path();
    match external::save_log(root, &check_id, &text) {
        Ok(path) => {
            let shown = path.strip_prefix(root).unwrap_or(&path);
            app.set_status_message(
                StatusKind::info(),
                format!("Saved log to {}", shown.display()),
            );
        }
        Err(e) => app.set_status_message(StatusKind::Error, format!("Save log failed: {e}")),
    }
    Action::Continue
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
    let Some(setup) = claim_setup(app, &check) else {
        return Action::Continue;
    };
    spawn_refreshed_run(app, tasks, check, setup, false);
    Action::Continue
}

/// Re-run `check` like 'r' (its group setup claimed: `setup` runs the
/// pre-commands first). Marks it running now (also blocks a second start);
/// git refresh runs off the event loop so large repos do not freeze the UI.
/// A user run ('r') also clears fix results; an `auto_only` one (`--watch`)
/// leaves them.
fn spawn_refreshed_run(
    app: &mut App,
    tasks: &mut Tasks,
    check: CheckToRun,
    setup: bool,
    auto_only: bool,
) {
    let previous = app
        .results
        .get(check.id())
        .cloned()
        .unwrap_or_else(|| CheckResult::pending(check.id()));
    let changed_files = app.changed_files.clone();
    if auto_only {
        app.mark_running(check.id());
    } else {
        app.reset_check_for_retry(check.id());
    }
    tasks.spawn(|ctx, tx| {
        retry_with_refresh(ctx, tx, check, previous, changed_files, setup, auto_only)
    });
}

/// Start the `--watch` re-runs queued in `app` ([`App::queue_watch`]) that
/// can start, like 'r' (git refresh, then the check; recorded in the result
/// cache). They run the check as the saves selected it, so a failed refresh
/// runs that one, not the listed one (maybe on-demand / skipped). None
/// start while a fix or its verification runs ([`App::fixing`]). A check
/// the runner still owes (pending, queued) stays queued until the runner
/// takes it (then dropped: that run sees the save) or the run ends with it
/// still pending (then started). A running one is cancelled (once) and stays
/// queued until its result arrives, so that stale result cannot land on the
/// new run. One whose group has no free slot ([`App::group_slot_free`]) or
/// whose pre-commands are busy stays queued. A check this run does not list
/// is dropped: the list holds every check the config can select.
fn start_watch_runs(app: &mut App, tasks: &mut Tasks) {
    if app.fixing() {
        return;
    }
    for mut queued in std::mem::take(&mut app.run.watch_queue) {
        let Some(result) = app.results.get(queued.check.id()) else {
            continue;
        };
        match result.status {
            CheckStatus::Pending if app.run.all_finished => {}
            CheckStatus::Pending | CheckStatus::Queued => {
                queued.runner_owed = true;
                app.run.watch_queue.push(queued);
                continue;
            }
            _ if queued.runner_owed => continue,
            CheckStatus::Running => {
                cancel_once(&mut queued, &tasks.ctx.cancels);
                app.run.watch_queue.push(queued);
                continue;
            }
            _ => {}
        }
        let (group, id) = (
            queued.check.group().to_string(),
            queued.check.id().to_string(),
        );
        let setup = match app.group_slot_free(&group) {
            true => app.claim_group_setup(&group, &id),
            false => GroupSetup::Busy,
        };
        match setup {
            GroupSetup::Busy => app.run.watch_queue.push(queued),
            setup => spawn_refreshed_run(app, tasks, queued.check, setup == GroupSetup::Run, true),
        }
    }
}

/// Cancel the running check of `queued`, once per queued re-run
fn cancel_once(queued: &mut app::WatchRun, cancels: &CancelRegistry) {
    if !queued.cancel_sent {
        cancels.cancel(queued.check.id());
        queued.cancel_sent = true;
    }
}

/// `msg` may let a queued `--watch` re-run start: a batch, a task event
/// (not an output chunk), or a runner event that frees a check, a group
/// slot or a group setup
fn wakes_watch(msg: &Message) -> bool {
    match msg {
        Message::Watch(_) => true,
        Message::RunnerEvent(event) | Message::Task(TaskEvent::Output(event)) => matches!(
            event,
            RunnerEvent::CheckFinished { .. }
                | RunnerEvent::PreCommandFinished { .. }
                | RunnerEvent::GroupFinished { .. }
                | RunnerEvent::AllFinished
        ),
        Message::Task(_) => true,
        _ => false,
    }
}

/// Checks whose fix passed in `event`, to verify ([`app::FixState::verify`]):
/// a single fix ('x') at its result, fix-all ('X') once done
fn fixes_to_verify(app: &App, event: &TaskEvent) -> Vec<String> {
    if !app.fix.verify {
        return Vec::new();
    }
    let results = match event {
        TaskEvent::FixResult(result) => std::slice::from_ref(result),
        TaskEvent::FixAllDone => &app.fix.all_results,
        _ => return Vec::new(),
    };
    results
        .iter()
        .filter(|r| r.status == CheckStatus::Passed)
        .map(|r| r.check_id.clone())
        .collect()
}

/// Re-run the fixed checks `check_ids` that are not running again: marked
/// running now (the fix result stays shown), run by [`run_verifications`],
/// results recorded like 'r'. No git refresh: a fix does not change which
/// checks apply. The first check of a group whose pre-commands are due runs
/// them for the group; skipped while they are busy ([`claim_setup`]).
fn verify_fixes(app: &mut App, tasks: &mut Tasks, check_ids: Vec<String>) {
    let mut jobs = Vec::new();
    let mut setup_groups = HashSet::new();
    for id in check_ids {
        let finished = app.results.get(&id).is_some_and(|r| r.status.is_finished());
        let check = app.checks.iter().find(|c| c.id() == id);
        let Some(check) = check.filter(|_| finished).cloned() else {
            continue;
        };
        let setup = match setup_groups.contains(check.group()) {
            true => Some(false),
            false => claim_setup(app, &check),
        };
        let Some(setup) = setup else {
            continue;
        };
        if setup {
            setup_groups.insert(check.group().to_string());
        }
        app.mark_verifying(check.id());
        jobs.push((check, setup));
    }
    if !jobs.is_empty() {
        tasks.spawn(|ctx, tx| run_verifications(ctx, tx, jobs));
    }
}

/// [`verify_fixes`] task: one check at a time in `jobs` order, so groups'
/// `parallel: false` / `max_parallel` hold. All are cancellable with 's'
/// from the start (waiting too).
async fn run_verifications(
    ctx: TaskCtx,
    tx: mpsc::Sender<TaskEvent>,
    jobs: Vec<(CheckToRun, bool)>,
) {
    let turn = tokio::sync::Semaphore::new(1);
    let no_setup = std::sync::Mutex::new(HashSet::new());
    let mut runs: Vec<_> = jobs
        .iter()
        .map(|(check, setup)| Box::pin(verify_one(&ctx, &tx, check, *setup, &turn, &no_setup)))
        .collect();
    // Join all; first polled in `jobs` order, so queued in that order on the
    // fair `turn`
    std::future::poll_fn(|cx| {
        runs.retain_mut(|run| std::future::Future::poll(run.as_mut(), cx).is_pending());
        match runs.is_empty() {
            true => std::task::Poll::Ready(()),
            false => std::task::Poll::Pending,
        }
    })
    .await;
}

/// One verification of [`run_verifications`]: once it has the `turn`, its
/// group's pre-commands with `setup`, then `check`. When those failed or
/// were cancelled, the group is in `no_setup`: its later checks fail
/// without running.
async fn verify_one(
    ctx: &TaskCtx,
    tx: &mpsc::Sender<TaskEvent>,
    check: &CheckToRun,
    setup: bool,
    turn: &tokio::sync::Semaphore,
    no_setup: &std::sync::Mutex<HashSet<String>>,
) {
    let group = check.group();
    let mut set_up = !setup;
    let run = async {
        let _turn = turn.acquire().await;
        let broken = no_setup.lock().unwrap().contains(group);
        if broken || (setup && !ctx.run_setup(group, tx).await) {
            return None;
        }
        set_up = true;
        Some(ctx.run_streaming(check, &check.resolved_command, tx).await)
    };
    let result = run_check_cancellable(&ctx.cancels, check.id(), run).await;
    if !set_up {
        no_setup.lock().unwrap().insert(group.to_string());
    }
    let event = match result {
        Some(result) => TaskEvent::RetryResult(result),
        None => TaskEvent::UnrecordedResult(CheckResult::setup_failed(check.id(), group)),
    };
    let _ = tx.send(event).await;
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
    let Some(setup) = claim_setup(app, &check) else {
        return Action::Continue;
    };
    app.mark_running(check.id());
    let command = check.resolved_command.clone();
    tasks.spawn_check(check, command, setup, TaskEvent::RetryResult);
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
    let Some(setup) = claim_setup(app, &check) else {
        return Action::Continue;
    };
    app.reset_check_for_retry(check.id());
    app.set_status_message(
        StatusKind::progress_for(check.id()),
        "Running for all files...",
    );
    tasks.spawn_check(check, all_files_cmd, setup, TaskEvent::UnrecordedResult);
    Action::Continue
}

/// Claim the group's pre-commands for a single run of `check`: `Some(true)`
/// runs them first. `None` while the runner is due to run them or they are
/// running (the footer says so): the check must not run before its setup.
fn claim_setup(app: &mut App, check: &CheckToRun) -> Option<bool> {
    match app.claim_group_setup(check.group(), check.id()) {
        GroupSetup::Busy => {
            let group = check.group();
            let text = format!("Pre-commands of group `{group}` pending: try again once they ran");
            app.set_status_message(StatusKind::info(), text);
            None
        }
        setup => Some(setup == GroupSetup::Run),
    }
}

/// Handle 's' key: cancel the selected running check. Its runner or retry
/// task then reports `Cancelled`; other checks keep running.
fn handle_cancel_selected(app: &mut App, tasks: &mut Tasks) -> Action {
    if !app.selected_capabilities().can_cancel {
        return Action::Continue;
    }
    if let Some(check) = app.selected_check() {
        tasks.ctx.cancels.cancel(check.id());
    }
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
        let (changed_files, Selected { checks, warnings }) =
            match ctx.refresh_async(previous, None).await {
                Ok(refreshed) => refreshed,
                // Git refresh failed - restart with no changed files
                Err(_) => {
                    let changed_files = ChangedFiles {
                        files: vec![],
                        base_ref,
                    };
                    let selected = ctx.select(&changed_files, None);
                    (changed_files, selected)
                }
            };
        let _ = tx
            .send(TaskEvent::RetryAllReady {
                changed_files,
                checks,
                warnings,
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
    tasks.spawn_fix(job.check, job.command);
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
            let result = ctx.run(&job.check, &job.command, &OutputSink::none()).await;
            let _ = tx.send(TaskEvent::FixAllResult(result)).await;
        }
        // Always signal completion, decoupled from per-result send success.
        // Previously completion rode on an is_last flag on the final result;
        // a failed send left fix_all_running=true and disabled r/t/x/X forever.
        let _ = tx.send(TaskEvent::FixAllDone).await;
    });
    Action::Continue
}

/// Handle a key press while the `?` help overlay is shown: only `?`/`Esc`
/// close it, everything else is swallowed (no normal action dispatches).
fn handle_help_key_event(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Esc => app.close_help(),
        _ => {}
    }
    Action::Continue
}

/// Handle a key press while an output search (`/`) is open, whether still
/// typing or confirmed (highlight visible). While typing, every key is
/// intercepted (characters build the query, `Enter` confirms, `Backspace`
/// edits, `Esc` cancels). Once confirmed, only `Esc` (clear highlight) is
/// intercepted; anything else returns `None` so the caller falls through to
/// normal handling (list nav, retry, etc. keep acting while a highlight is
/// shown).
fn handle_search_key_event(app: &mut App, key: KeyEvent) -> Option<Action> {
    let typing = app.view.search.as_ref()?.typing;
    if !typing {
        return match key.code {
            KeyCode::Esc => {
                app.cancel_search();
                Some(Action::Continue)
            }
            _ => None,
        };
    }
    match key.code {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => dashboard::confirm_search(app),
        KeyCode::Backspace => app.search_backspace(),
        KeyCode::Char(c) => app.search_push(c),
        _ => {}
    }
    Some(Action::Continue)
}

/// Selection/scroll navigation keys: list jump (g/G, n/N, arrows/jk) and
/// output scroll (PageUp/Down, Home/End, J/K, Ctrl-d/u). Returns `false` for
/// anything it doesn't handle, so the caller can fall through to actions.
fn handle_nav_key_event(app: &mut App, key: KeyEvent) -> bool {
    match (key.code, key.modifiers) {
        (KeyCode::Up | KeyCode::Char('k'), _) => app.previous_check(),
        (KeyCode::Down | KeyCode::Char('j'), _) => app.next_check(),
        (KeyCode::Char('g'), KeyModifiers::NONE) => app.select_first(),
        (KeyCode::Char('G'), KeyModifiers::SHIFT) => app.select_last(),
        (KeyCode::Char('n'), KeyModifiers::NONE) => app.select_next_failed(),
        (KeyCode::Char('N'), KeyModifiers::SHIFT) => app.select_prev_failed(),
        (KeyCode::PageUp, _) => app.scroll_up(PAGE_SCROLL_LINES),
        (KeyCode::PageDown, _) => app.scroll_down(PAGE_SCROLL_LINES),
        (KeyCode::Home, _) => app.scroll_to_top(),
        (KeyCode::End, _) => app.scroll_to_bottom(),
        (KeyCode::Char('J'), KeyModifiers::SHIFT) => app.scroll_down(1),
        (KeyCode::Char('K'), KeyModifiers::SHIFT) => app.scroll_up(1),
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            app.scroll_down(app.view.output_visible_lines / 2)
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            app.scroll_up(app.view.output_visible_lines / 2)
        }
        _ => return false,
    }
    true
}

/// Handle a key event and return the action for the main loop
fn handle_key_event(app: &mut App, key: KeyEvent, tasks: &mut Tasks) -> Action {
    if app.view.help_visible {
        return handle_help_key_event(app, key);
    }
    if let Some(action) = handle_search_key_event(app, key) {
        return action;
    }
    if handle_nav_key_event(app, key) {
        return Action::Continue;
    }
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Quit,
        (KeyCode::Char('?'), _) => {
            app.toggle_help();
            Action::Continue
        }
        (KeyCode::Char('/'), _) => {
            app.open_search();
            Action::Continue
        }
        (KeyCode::Char('m'), KeyModifiers::NONE) => {
            app.toggle_stats();
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
        // Fold/unfold on a group header; no-op on checks and pre-commands.
        // Enter while typing a search never reaches here (confirms search).
        (KeyCode::Char(' ') | KeyCode::Enter, KeyModifiers::NONE) => {
            app.toggle_selected_group();
            Action::Continue
        }
        (KeyCode::Char('r'), KeyModifiers::NONE) => handle_retry_selected(app, tasks),
        (KeyCode::Char('t'), KeyModifiers::NONE) => handle_trigger_on_demand(app, tasks),
        (KeyCode::Char('s'), KeyModifiers::NONE) => handle_cancel_selected(app, tasks),
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
        (KeyCode::Char('o'), KeyModifiers::NONE) => handle_open_output(app, Viewer::Pager),
        (KeyCode::Char('O'), KeyModifiers::SHIFT) => handle_open_output(app, Viewer::Editor),
        (KeyCode::Char('w'), KeyModifiers::NONE) => handle_save_log(app, tasks),
        _ => Action::Continue,
    }
}

/// Handle a mouse event: wheel scroll over the output panel, left-click to
/// select a row (check, pre-command or group header) in the checks list. Anything else (clicks/scroll outside
/// those panels, other buttons) is ignored.
fn handle_mouse_event(app: &mut App, mouse: MouseEvent) -> Action {
    if app.view.help_visible {
        return Action::Continue;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp if app.is_over_output(mouse.column, mouse.row) => {
            app.scroll_up(MOUSE_SCROLL_LINES);
        }
        MouseEventKind::ScrollDown if app.is_over_output(mouse.column, mouse.row) => {
            app.scroll_down(MOUSE_SCROLL_LINES);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            app.select_check_at_position(mouse.column, mouse.row);
        }
        _ => {}
    }
    Action::Continue
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
        TaskEvent::Output(event) => app.handle_runner_event(event),
        TaskEvent::RetryResult(result) | TaskEvent::UnrecordedResult(result) => {
            let check_id = result.check_id.clone();
            app.set_retry_result(result);
            app.finish_progress(&check_id);
        }
        TaskEvent::RetryAllReady {
            changed_files,
            checks,
            warnings,
        } => {
            return Action::RestartRunner {
                new_changed_files: changed_files,
                new_checks: checks,
                warnings,
            }
        }
    }
    Action::Continue
}

/// Records finished checks in the result cache on its own thread, in
/// arrival order: file hashing and the write stay off the event loop.
/// Dropping it waits until every queued result is recorded.
struct Recorder {
    /// Queue; `None` once dropped, ending the thread
    tx: Option<std::sync::mpsc::Sender<(CheckToRun, CheckResult)>>,
    thread: Option<thread::JoinHandle<()>>,
    /// Write error (at most one: the cache is then off)
    errors: std::sync::mpsc::Receiver<io::Error>,
}

impl Recorder {
    fn start(mut cache: ResultCache) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<(CheckToRun, CheckResult)>();
        let (error_tx, errors) = std::sync::mpsc::channel();
        let thread = thread::spawn(move || {
            rx.into_iter()
                .filter_map(|(check, result)| cache.record(&check, &result).err())
                .for_each(|e| drop(error_tx.send(e)));
        });
        Self {
            tx: Some(tx),
            thread: Some(thread),
            errors,
        }
    }

    /// Queue the check result in `msg` (runner, retry or on-demand; not a
    /// run-all-files one), with the check as `app` has it now. A write
    /// error of an earlier result shows in the footer; it never stops the
    /// run.
    fn record(&self, app: &mut App, msg: &Message) {
        if let Ok(e) = self.errors.try_recv() {
            app.set_status_message(StatusKind::Error, format!("Result cache not saved: {e}"));
        }
        let (Message::RunnerEvent(RunnerEvent::CheckFinished { result })
        | Message::Task(TaskEvent::RetryResult(result))) = msg
        else {
            return;
        };
        // Only status and `cached` count: leave the output behind
        let result = CheckResult {
            status: result.status.clone(),
            cached: result.cached,
            ..CheckResult::pending(&result.check_id)
        };
        let check = app.checks.iter().find(|c| c.id() == result.check_id);
        if let (Some(tx), Some(check)) = (&self.tx, check) {
            let _ = tx.send((check.clone(), result));
        }
    }

    /// Stop queueing and wait until every queued result is recorded
    fn flush(&mut self) {
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Show checks `cache` finds unchanged since their last pass as cached in
/// `app`; returns the others, for the runner. Only the initial run uses
/// this: retries and on-demand runs always execute.
fn skip_cached(app: &mut App, cache: &ResultCache, checks: Vec<CheckToRun>) -> Vec<CheckToRun> {
    let (cached, runnable) = cache.split_fresh(checks);
    app.mark_cached(cached.iter().map(CheckToRun::id));
    runnable
}

/// Handle a message from the event loop and update app state
/// All state changes go through this function via &mut App
fn handle_message(app: &mut App, msg: Message, tasks: &mut Tasks) -> Action {
    // Any user input during the run cancels the run-end auto-select (only
    // click/scroll mouse events get here: the input thread filters the rest)
    if matches!(msg, Message::KeyPress(_) | Message::Mouse(_)) {
        app.run.user_interacted = true;
    }
    let wakes_watch = wakes_watch(&msg);
    let action = match msg {
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
        Message::Mouse(mouse) => handle_mouse_event(app, mouse),
        Message::RunnerEvent(event) => {
            app.handle_runner_event(event);
            Action::Continue
        }
        Message::SystemStats(stats) => {
            app.update_stats(stats.cpu_usage, stats.mem_used, stats.mem_total);
            Action::Continue
        }
        Message::Task(event) => {
            let fixed = fixes_to_verify(app, &event);
            let action = handle_task_event(app, event);
            verify_fixes(app, tasks, fixed);
            action
        }
        Message::StatusExpired => {
            app.expire_status_message(std::time::Instant::now());
            Action::Continue
        }
        Message::Tick => {
            app.needs_redraw = true;
            Action::Continue
        }
        Message::Watch(batch) => {
            app.queue_watch(batch);
            Action::Continue
        }
    };
    // Queued `--watch` re-runs start once nothing holds them back
    if wakes_watch && !app.run.watch_queue.is_empty() {
        start_watch_runs(app, tasks);
    }
    action
}

/// TUI options from the CLI. Color follows the process-wide
/// switch ([`crate::color::enabled`]) set from `--color` / `--no-color` / `NO_COLOR`.
pub struct TuiOptions {
    /// Active `--only` / `--group` filter, shown in the header
    pub filter_notice: Option<String>,
    /// Start with the CPU/MEM stats panel shown (off with `--no-stats`)
    pub show_stats: bool,
    /// Result cache: skips unchanged checks at start, records finished runs
    pub cache: ResultCache,
    /// Bell + desktop notification when a full run finishes (config
    /// `notify` or `--notify`)
    pub notify: bool,
    /// Quit when the run finishes (`--exit-on-finish`); exit code as for `q`
    pub exit_on_finish: bool,
    /// Re-run a check after its fix ('x' / 'X') passed (off with `--no-verify`)
    pub verify: bool,
    /// Re-run checks affected by file saves (`--watch`)
    pub watch: bool,
}

impl TuiOptions {
    /// From the CLI flags; `notify` also from config
    pub fn new(
        cli: &crate::cli::Cli,
        config: &CiConfig,
        filter_notice: Option<String>,
        cache: ResultCache,
    ) -> Self {
        Self {
            filter_notice,
            show_stats: !cli.no_stats,
            cache,
            notify: cli.notify || config.notify,
            exit_on_finish: cli.exit_on_finish,
            verify: !cli.no_verify,
            watch: cli.watch,
        }
    }
}

/// App state for the TUI on the current branch, with `options` applied
fn new_app(
    config: &CiConfig,
    changed_files: ChangedFiles,
    checks: &[CheckToRun],
    project_root: &Path,
    options: &TuiOptions,
) -> App {
    let branch_name = current_branch(project_root).unwrap_or_else(|_| "unknown".to_string());
    let mut app = App::new(config.clone(), changed_files, checks.to_vec(), branch_name);
    app.filter_notice = options.filter_notice.clone();
    app.color = crate::color::enabled();
    app.view.stats_visible = options.show_stats;
    app.fix.verify = options.verify;
    app.watching = options.watch;
    app
}

/// The run is over and nothing else changes its outcome: the runner sent
/// `AllFinished` (full runs only: initial, 'R', 'a'; a restart resets it),
/// no retry / on-demand run / fix / 'R' refresh is in flight ([`App::busy`])
/// and no viewer owns the terminal
fn run_settled(app: &App, viewer_open: bool) -> bool {
    app.run.all_finished && !app.busy() && !viewer_open
}

/// Once per run, when it settled: the end-of-run notification to the TUI's
/// terminal if `notify` is on (best effort)
fn report_run_end(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    notify: bool,
    viewer_open: bool,
) {
    if app.run.end_reported || !run_settled(app, viewer_open) {
        return;
    }
    app.run.end_reported = true;
    if notify {
        let message = format!("ci-tui: {}", finished_text(app, &app.count_by_status()));
        let _ = notify::emit(terminal.backend_mut(), &message);
    }
}

/// `--exit-on-finish` quit due: the run settled (checked each loop turn, so
/// it quits once a viewer exits or in-flight work ends)
fn exit_due(app: &App, exit_on_finish: bool, viewer_open: bool) -> bool {
    exit_on_finish && run_settled(app, viewer_open)
}

/// Run the TUI; returns the process exit code (1 if any check failed).
/// `startup_warnings` (test discovery, docker preflight) are shown in the footer until a
/// keypress, since stderr is hidden behind the alternate screen.
/// `options.filter_notice` (`--only` / `--group`) stays in the header.
///
/// The caller must drop the tokio runtime before exiting: that drops the
/// aborted runner/task futures, whose guards kill still-running commands.
pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
    exec_root: PathBuf,
    startup_warnings: Vec<String>,
    options: TuiOptions,
) -> Result<i32> {
    // `--watch` fails here, before the terminal is touched (exit 3)
    let config_arc = Arc::new(config.clone());
    let (_watch, mut watch_rx) =
        start_watch(options.watch, &project_root, &config_arc, &exec_root)?;

    // Install panic hook to restore terminal on panic
    install_panic_hook();
    // Ctrl-C is a key event in raw mode; SIGINT only arrives with the TUI
    // suspended (Ctrl-C in `$PAGER`, sent to the whole foreground process
    // group) and must not kill ci-tui. A registered handler keeps the
    // default action off for the TUI's lifetime (external `kill -INT` too).
    #[cfg(unix)]
    let _sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt());

    // Setup terminal; the guard restores it on any early return below
    let (_terminal_guard, mut terminal) = setup_terminal()?;

    let mut app = new_app(&config, changed_files, &checks, &project_root, &options);
    app.show_warnings(&startup_warnings);
    let checks = skip_cached(&mut app, &options.cache, checks);

    // Start the runner in background ('s' cancels through `cancels`)
    let cancels = CancelRegistry::default();
    let (mut runner_handle, mut event_rx) = start_runner(&config, &exec_root, checks, &cancels);

    // Spawner for fix/retry/refresh tasks and the receiver for their events
    let key_root = options.cache.key_root().map(Arc::from);
    let ctx = TaskCtx::new(project_root, &exec_root, config_arc, &cancels, key_root);
    let (mut tasks, mut task_rx) = Tasks::new(ctx);
    let recorder = Recorder::start(options.cache);

    // Start background stats worker - runs sysinfo queries without blocking UI
    let (stats_tx, mut stats_rx) = mpsc::channel::<SystemStats>(STATS_CHANNEL_CAPACITY);
    let stats_handle = spawn_stats_worker(stats_tx);

    // Start dedicated OS thread for keyboard input
    // CRITICAL: Using std::thread ensures keyboard events are processed by the OS
    // scheduler even when Tokio is starved of CPU time by Docker containers.
    let (mut keyboard, mut input_rx) = KeyboardThread::start();
    // `o` / `O`: the running viewer (TUI suspended) and its temp dir
    let mut viewer_task: Option<ViewerTask> = None;
    let mut viewer_dir: Option<tempfile::TempDir> = None;
    let mut live_tick = live_timer_tick();

    // Main event loop - uses tokio::select! for event-driven responsiveness
    //
    // Architecture for responsive keyboard handling under high CPU load:
    // 1. biased; ensures keyboard events are checked first (highest priority)
    // 2. Each channel becomes a select! branch - wakes on any event
    // 3. All state transitions go through handle_message with explicit Message enum
    // 4. Render only if state changed (dirty flag)
    // Ends early with `--exit-on-finish` once the run finished (deferred
    // while a viewer runs)
    while !exit_due(&app, options.exit_on_finish, viewer_task.is_some()) {
        // Status message auto-dismiss deadline (disabled branch when None)
        let status_deadline = app.status_message_deadline();

        // Wait for the next event from any channel
        // biased; ensures keyboard is checked first for immediate responsiveness
        let msg = tokio::select! {
            biased;

            // Keyboard (and resize) events have highest priority. Off while
            // a viewer runs: keys the old thread read belong to the viewer.
            Some(event) = input_rx.recv(), if viewer_task.is_none() => match event {
                Event::Key(key) => Message::KeyPress(key),
                Event::Mouse(mouse) => Message::Mouse(mouse),
                _ => Message::Resize,
            },

            // Viewer exited: restore the TUI, read keys again, repaint as
            // after a resize (the terminal may have been resized meanwhile)
            outcome = async { (&mut viewer_task.as_mut().expect("guarded").1).await },
                if viewer_task.is_some() =>
            {
                let (viewer, _) = viewer_task.take().expect("guarded");
                finish_viewer(&mut terminal, &mut app, viewer, outcome)?;
                (keyboard, input_rx) = KeyboardThread::start();
                Message::Resize
            }

            // Status message TTL (ahead of busy channels so it cannot starve)
            _ = tokio::time::sleep_until(
                status_deadline.unwrap_or_else(std::time::Instant::now).into()
            ), if status_deadline.is_some() => Message::StatusExpired,

            // Live elapsed timers, only while a check runs
            _ = live_tick.tick(), if app.has_live_timers() => Message::Tick,

            // Runner lifecycle/status events
            Some(event) = event_rx.recv() => Message::RunnerEvent(event),

            // System stats from background worker
            Some(stats) = stats_rx.recv() => Message::SystemStats(stats),

            // Fix / retry / refresh task events
            Some(event) = task_rx.recv() => Message::Task(event),

            // `--watch`: checks affected by saved files
            Some(batch) = watch_rx.recv(), if options.watch => Message::Watch(batch),

            // All channels closed - exit
            else => break,
        };

        // Handle the message and get the action
        recorder.record(&mut app, &msg);
        match handle_message(&mut app, msg, &mut tasks) {
            Action::Quit => break,
            Action::RestartRunner {
                new_changed_files,
                new_checks,
                warnings,
            } => {
                // Abort old runner and background tasks (dropping their
                // futures kills process groups / `docker run` containers),
                // then start fresh. The new channel drops late results from
                // tasks started before the reset.
                runner_handle.abort();
                tasks.set.abort_all();
                (tasks, task_rx) = Tasks::new(tasks.ctx.clone());
                app.reset_for_retry(new_changed_files, new_checks.clone());
                app.show_warnings(&warnings);
                let (new_handle, new_rx) = start_runner(&config, &exec_root, new_checks, &cancels);
                runner_handle = new_handle;
                event_rx = new_rx;
            }
            Action::OpenViewer(request) => match viewer_file(&mut viewer_dir, &request) {
                Ok(path) => {
                    // Stop reading keys (waits out one poll) so they reach
                    // the viewer; restarted when it exits
                    keyboard.stop();
                    viewer_task = Some(start_viewer(&mut terminal, request.viewer, path)?);
                }
                Err(e) => app.set_status_message(StatusKind::Error, format!("Temp file: {e}")),
            },
            Action::Continue => {}
        }
        // Before the `--exit-on-finish` check in the loop condition
        report_run_end(
            &mut terminal,
            &mut app,
            options.notify,
            viewer_task.is_some(),
        );

        // Render if state changed (not while a viewer owns the terminal)
        if app.needs_redraw && viewer_task.is_none() {
            terminal.draw(|f| dashboard::render(&mut app, f))?;
            app.needs_redraw = false;
        }
    }

    // Cleanup - stop the keyboard thread (waits out one poll).
    // Aborted futures kill their commands when dropped: here, or at the
    // latest when main drops the runtime (before process exit).
    keyboard.stop();
    runner_handle.abort();
    tasks.set.abort_all();
    stats_handle.abort();
    let _ = runner_handle.await;
    while tasks.set.join_next().await.is_some() {}

    // Restore terminal: use the backend's own stdout handle to ensure
    // LeaveAlternateScreen goes through the same IO path as all TUI writes.
    leave_tui(terminal.backend_mut())?;
    terminal.show_cursor()?;
    // Drop terminal before printing to ensure all backend IO is flushed
    drop(terminal);

    // Print summary
    print_summary(&app);

    // Non-zero exit on failure so `ci-tui && git push` is safe (terminal already restored)
    Ok(app.exit_code())
}

/// Run outcome line: same wording as the finished dashboard header
fn finished_text(app: &App, counts: &app::StatusCounts) -> String {
    let elapsed = dashboard::format_elapsed(app.elapsed_time());
    dashboard::finished_status_text(counts, &elapsed)
}

/// Print the final summary: same wording as the dashboard header, colored
/// green (all passed), yellow (some cancelled / pending) or red (exit code
/// 1: a check or group pre-command failed); plain with color off
fn print_summary(app: &App) {
    let counts = app.count_by_status();
    let text = finished_text(app, &counts);
    let color = if app.exit_code() != crate::exit::SUCCESS {
        31
    } else if counts.cancelled > 0 || counts.pending > 0 {
        33
    } else {
        32
    };
    let line = format!("\n\x1b[{}m{}\x1b[0m", color, text);
    println!("{}", crate::color::paint(&line, app.color));

    // Show failed checks
    for (id, result) in &app.results {
        if result.status.is_failure() {
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
            cancels: CancelRegistry::default(),
            key_root: None,
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
                timeout: None,
                error_pattern: None,
            },
            service: Some("php".to_string()),
            files: crate::checks::CheckFiles::Files(vec!["src/Foo.php".to_string()]),
            resolved_command: format!("{} src/Foo.php", id),
            resolved_fix_command: None,
            cache_key: None,
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
            ..
        } = event
        else {
            panic!("expected RetryAllReady, got {:?}", event);
        };
        assert_eq!(changed_files.files, vec!["src/Foo.php".to_string()]);
        assert_eq!(changed_files.base_ref, crate::git::CLI_FILES_BASE_REF);
        assert_eq!(checks.len(), 1, "php-lint must still match the file");
    }

    /// Git env vars set by hooks (e.g. pre-commit) that redirect git to the
    /// outer repo's index / dir instead of the test's tempdir repo.
    const OUTER_GIT_ENV: [&str; 4] = ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE", "GIT_PREFIX"];

    /// Run `git args` in `dir` with [`OUTER_GIT_ENV`] cleared, panicking on failure.
    fn git(dir: &std::path::Path, args: &[&str]) {
        let mut cmd = std::process::Command::new("git");
        for var in OUTER_GIT_ENV {
            cmd.env_remove(var);
        }
        let status = cmd
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git must be installed");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn test_refresh_in_staged_mode_rereads_index() {
        // `refresh` runs git with the process env; skip when a hook redirects it
        if OUTER_GIT_ENV.iter().any(|v| std::env::var_os(v).is_some()) {
            eprintln!("skipped: outer git env set (running inside a git hook)");
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        std::fs::write(repo.path().join("Staged.php"), "<?php").unwrap();
        std::fs::write(repo.path().join("Unstaged.php"), "<?php").unwrap();
        git(repo.path(), &["add", "Staged.php"]);
        let (mut tasks, _rx) = make_test_tasks(&test_config());
        tasks.ctx.project_root = Arc::new(repo.path().to_path_buf());
        let previous = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: crate::git::STAGED_BASE_REF.to_string(),
        };

        let (changed_files, _) = tasks.ctx.refresh(previous, None).unwrap();

        assert_eq!(changed_files.files, vec!["Staged.php".to_string()]);
        assert!(changed_files.is_staged());
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

    #[tokio::test]
    async fn test_run_all_files_progress_cleared_only_by_own_result() {
        let config = test_config();
        let mut app = make_test_app_with_checks(
            &config,
            vec![
                make_test_check("php-lint", "fast"),
                make_test_check("phpstan", "fast"),
            ],
        );
        app.results.get_mut("php-lint").unwrap().status = crate::runner::CheckStatus::Passed;
        let (mut tasks, _rx) = make_test_tasks(&config);
        assert_eq!(app.selected_check().map(|c| c.id()), Some("php-lint"));

        handle_run_all_files(&mut app, &mut tasks);
        let text = |app: &App| app.view.status_message.as_ref().map(|m| m.text.clone());
        assert_eq!(text(&app).as_deref(), Some("Running for all files..."));

        // Edge case: another check's retry/on-demand result arrives first
        handle_task_event(
            &mut app,
            TaskEvent::RetryResult(CheckResult::pending("phpstan")),
        );
        assert_eq!(
            text(&app).as_deref(),
            Some("Running for all files..."),
            "unrelated result must not clear progress"
        );

        handle_task_event(
            &mut app,
            TaskEvent::RetryResult(CheckResult::pending("php-lint")),
        );
        assert!(app.view.status_message.is_none(), "own result clears it");
    }

    #[test]
    fn test_refresh_progress_survives_retry_result() {
        let config = test_config();
        let mut app = make_test_app(&config);
        app.start_refresh();

        handle_task_event(
            &mut app,
            TaskEvent::RetryResult(CheckResult::pending("php-lint")),
        );

        assert_eq!(
            app.view.status_message.as_ref().map(|m| m.text.as_str()),
            Some("Refreshing changed files...")
        );
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
            app.selected_check().map(|c| c.id()),
            Some("phpstan"),
            "key still acts while dismissing the message"
        );
    }

    /// #141: the live timer tick redraws
    #[tokio::test]
    async fn test_tick_redraws() {
        let config = test_config();
        let mut app = make_test_app(&config);
        app.needs_redraw = false;
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_message(&mut app, Message::Tick, &mut tasks);

        assert!(app.needs_redraw);
    }

    /// Spawn a task holding a cancellable registration for `check_id`; it
    /// reports a cancelled result once cancelled
    fn spawn_cancellable(tasks: &mut Tasks, check_id: &'static str) {
        tasks.spawn(move |ctx, tx| async move {
            let run = std::future::pending::<CheckResult>();
            let result = run_check_cancellable(&ctx.cancels, check_id, run).await;
            let _ = tx.send(TaskEvent::RetryResult(result)).await;
        });
    }

    #[tokio::test]
    async fn test_cancel_key_cancels_running_check() {
        let config = test_config();
        let mut app = make_test_app_with_checks(&config, vec![make_test_check("php-lint", "fast")]);
        app.results.get_mut("php-lint").unwrap().status = crate::runner::CheckStatus::Running;
        let (mut tasks, mut rx) = make_test_tasks(&config);
        spawn_cancellable(&mut tasks, "php-lint");
        tokio::task::yield_now().await; // let the task register

        let key = press(KeyCode::Char('s'), KeyModifiers::NONE);
        let action = handle_key_event(&mut app, key, &mut tasks);

        assert!(matches!(action, Action::Continue));
        let event = rx.recv().await.expect("cancelled task must report");
        handle_task_event(&mut app, event);
        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            crate::runner::CheckStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn test_cancel_key_ignores_non_running_check() {
        let config = test_config();
        let mut app = make_test_app_with_checks(&config, vec![make_test_check("php-lint", "fast")]);
        app.results.get_mut("php-lint").unwrap().status = crate::runner::CheckStatus::Passed;
        let (mut tasks, mut rx) = make_test_tasks(&config);
        spawn_cancellable(&mut tasks, "php-lint");
        tokio::task::yield_now().await;

        let key = press(KeyCode::Char('s'), KeyModifiers::NONE);
        handle_key_event(&mut app, key, &mut tasks);
        tokio::task::yield_now().await;

        assert!(rx.try_recv().is_err(), "non-running check must not cancel");
        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            crate::runner::CheckStatus::Passed
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

    /// Feed `app` through `handle_message`: `check_id` finished with
    /// `status` (if any), then the runner's AllFinished
    fn finish_run(app: &mut App, config: &CiConfig, finished: Option<(&str, CheckStatus)>) {
        let (mut tasks, _rx) = make_test_tasks(config);
        if let Some((id, status)) = finished {
            let result = CheckResult {
                status,
                ..CheckResult::pending(id)
            };
            let msg = Message::RunnerEvent(RunnerEvent::CheckFinished { result });
            handle_message(app, msg, &mut tasks);
        }
        let msg = Message::RunnerEvent(RunnerEvent::AllFinished);
        handle_message(app, msg, &mut tasks);
    }

    #[rstest::rstest]
    #[case::passed(CheckStatus::Passed, crate::exit::SUCCESS)]
    #[case::failed(CheckStatus::Failed, crate::exit::CHECKS_FAILED)]
    #[case::timed_out(CheckStatus::TimedOut, crate::exit::CHECKS_FAILED)]
    fn test_exit_on_finish_exit_code(#[case] status: CheckStatus, #[case] code: i32) {
        let config = test_config();
        let mut app = make_test_app_with_checks(&config, vec![make_test_check("php-lint", "fast")]);
        finish_run(&mut app, &config, Some(("php-lint", status)));
        assert!(exit_due(&app, true, false));
        assert_eq!(app.exit_code(), code);
    }

    #[test]
    fn test_exit_on_finish_ignores_skipped_and_on_demand() {
        let config = test_config();
        let mut on_demand = make_test_check("behat", "fast");
        on_demand.files = crate::checks::CheckFiles::OnDemand;
        let mut skipped = make_test_check("phpcs", "fast");
        skipped.files = crate::checks::CheckFiles::SkippedNoMatch;
        let mut app = make_test_app_with_checks(&config, vec![on_demand, skipped]);
        finish_run(&mut app, &config, None);
        assert!(exit_due(&app, true, false));
        assert_eq!(app.exit_code(), crate::exit::SUCCESS);
    }

    #[test]
    fn test_exit_on_finish_fails_when_group_setup_failed() {
        // A failed group pre-command stops the run: its checks never run
        let config = setup_config();
        let mut app = make_test_app_with_checks(&config, vec![make_test_check("lint", "db")]);
        app.handle_runner_event(RunnerEvent::PreCommandFinished {
            group: "db".to_string(),
            name: "seed".to_string(),
            success: false,
            output: String::new(),
            duration_ms: 1,
        });
        finish_run(&mut app, &config, None);
        assert!(exit_due(&app, true, false));
        assert_eq!(app.exit_code(), crate::exit::CHECKS_FAILED);
        let text = finished_text(&app, &app.count_by_status());
        assert!(
            text.starts_with("0/1 passed, 0 failed, 1 pending in "),
            "{text}"
        );
    }

    #[rstest::rstest]
    #[case::finished(true, true, false, true)]
    #[case::flag_off(true, false, false, false)]
    #[case::viewer_open(true, true, true, false)]
    #[case::not_finished(false, true, false, false)]
    fn test_exit_due(
        #[case] all_finished: bool,
        #[case] exit_on_finish: bool,
        #[case] viewer_open: bool,
        #[case] expected: bool,
    ) {
        let mut app = make_test_app(&test_config());
        app.run.all_finished = all_finished;
        assert_eq!(exit_due(&app, exit_on_finish, viewer_open), expected);
    }

    #[rstest::rstest]
    #[case::retry_running(CheckStatus::Running)]
    #[case::waiting_for_setup(CheckStatus::Queued)]
    fn test_exit_due_waits_for_single_run(#[case] status: CheckStatus) {
        let config = test_config();
        let mut app = make_test_app_with_checks(&config, vec![make_test_check("php-lint", "fast")]);
        finish_run(&mut app, &config, Some(("php-lint", CheckStatus::Failed)));
        app.results.get_mut("php-lint").unwrap().status = status;
        assert!(!exit_due(&app, true, false));
    }

    #[test]
    fn test_exit_due_waits_for_fix_and_refresh() {
        let mut app = make_test_app(&test_config());
        app.run.all_finished = true;
        app.fix.running = true;
        assert!(!exit_due(&app, true, false), "fix in flight");
        app.fix.running = false;
        app.run.refresh_pending = true;
        assert!(!exit_due(&app, true, false), "'R' refresh replaces the run");
    }

    #[test]
    fn test_finished_text_total_excludes_skipped_and_on_demand() {
        let config = test_config();
        let mut on_demand = make_test_check("behat", "fast");
        on_demand.files = crate::checks::CheckFiles::OnDemand;
        let mut skipped = make_test_check("phpcs", "fast");
        skipped.files = crate::checks::CheckFiles::SkippedNoMatch;
        let checks = vec![make_test_check("php-lint", "fast"), on_demand, skipped];
        let mut app = make_test_app_with_checks(&config, checks);
        finish_run(&mut app, &config, Some(("php-lint", CheckStatus::Passed)));
        let text = finished_text(&app, &app.count_by_status());
        assert!(text.starts_with("✓ All 1 checks passed in "), "{text}");
        assert!(text.ends_with(" +1 on-demand"), "{text}");
    }

    #[test]
    fn test_handle_message_retry_result_stored() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        let result = CheckResult {
            status: crate::runner::CheckStatus::Passed,
            output: "OK".to_string(),
            duration_ms: 42,
            ..CheckResult::pending("php-lint")
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

    /// Tempdir holding `src/Foo.php` (the test checks' file), with a result
    /// cache file at `cache_file` in it
    fn cache_in_tempdir(cache_file: &str) -> (tempfile::TempDir, ResultCache) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/Foo.php"), "<?php").unwrap();
        let root = dir.path().to_path_buf();
        let cache = ResultCache::at(dir.path().join(cache_file), root, true);
        (dir, cache)
    }

    /// `make_test_check` keyed from the files under `root`
    fn keyed_check(config: &CiConfig, id: &str, root: &Path) -> CheckToRun {
        let mut check = make_test_check(id, "fast");
        let changed = make_test_app(config).changed_files;
        let checks = std::slice::from_mut(&mut check);
        crate::cache::stamp_keys(checks, config, &changed, root, root);
        assert!(check.cache_key.is_some());
        check
    }

    fn with_status(id: &str, status: CheckStatus) -> CheckResult {
        CheckResult {
            status,
            ..CheckResult::pending(id)
        }
    }

    fn finished(id: &str, status: CheckStatus) -> Message {
        let result = with_status(id, status);
        Message::RunnerEvent(RunnerEvent::CheckFinished { result })
    }

    #[test]
    fn test_skip_cached_marks_fresh_checks_and_runs_the_rest() {
        let config = test_config();
        let (dir, mut cache) = cache_in_tempdir("results.json");
        let lint = keyed_check(&config, "php-lint", dir.path());
        let unit = keyed_check(&config, "phpunit", dir.path());
        let mut app = make_test_app_with_checks(&config, vec![lint.clone(), unit.clone()]);
        let pass = with_status("php-lint", CheckStatus::Passed);
        cache.record(&lint, &pass).unwrap();

        let runnable = skip_cached(&mut app, &cache, vec![lint, unit]);

        let ids: Vec<&str> = runnable.iter().map(CheckToRun::id).collect();
        assert_eq!(ids, ["phpunit"]);
        assert!(app.results["php-lint"].cached);
        assert!(!app.results["phpunit"].cached);
    }

    #[test]
    fn test_recorder_records_runner_and_retry_results_not_all_files_runs() {
        let config = test_config();
        let (dir, cache) = cache_in_tempdir("results.json");
        let lint = keyed_check(&config, "php-lint", dir.path());
        let unit = keyed_check(&config, "phpunit", dir.path());
        let mut app = make_test_app_with_checks(&config, vec![lint.clone(), unit.clone()]);
        let recorder = Recorder::start(cache);

        recorder.record(&mut app, &finished("php-lint", CheckStatus::Passed));
        recorder.record(&mut app, &finished("phpunit", CheckStatus::Passed));
        let retry_failed = with_status("phpunit", CheckStatus::Failed);
        recorder.record(
            &mut app,
            &Message::Task(TaskEvent::RetryResult(retry_failed)),
        );
        let all_files = with_status("phpunit", CheckStatus::Passed);
        recorder.record(
            &mut app,
            &Message::Task(TaskEvent::UnrecordedResult(all_files)),
        );
        drop(recorder);

        let root = dir.path().to_path_buf();
        let reopened = ResultCache::at(dir.path().join("results.json"), root, true);
        assert!(reopened.is_fresh(&lint), "runner pass recorded");
        assert!(
            !reopened.is_fresh(&unit),
            "retry failure dropped it; the all-files pass is not stored"
        );
    }

    #[test]
    fn test_recorder_write_error_sets_status_message() {
        let config = test_config();
        let (dir, cache) = cache_in_tempdir("ci-tui/results.json");
        std::fs::write(dir.path().join("ci-tui"), "").unwrap();
        let lint = keyed_check(&config, "php-lint", dir.path());
        let mut app = make_test_app_with_checks(&config, vec![lint]);
        let mut recorder = Recorder::start(cache);

        recorder.record(&mut app, &finished("php-lint", CheckStatus::Passed));
        // Wait for the write, then any message shows its error
        recorder.flush();
        recorder.record(&mut app, &Message::Task(TaskEvent::FixAllDone));

        let message = app.view.status_message.as_ref().expect("status message");
        assert!(message.text.contains("Result cache"), "{}", message.text);
    }

    /// Edge case: local-mode group `db` whose `seed` pre-command creates the
    /// `seeded` file that check `lint` needs
    fn setup_config() -> CiConfig {
        setup_config_with("echo seeded > seeded")
    }

    /// [`setup_config`] with `seed` running `command`
    fn setup_config_with(command: &str) -> CiConfig {
        let yaml = r#"
version: 2
runner: local
local: { shell: sh }
git: { base_branch: main, fallback_branch: HEAD~1 }
file_patterns: {}
checks:
  db:
    pre_commands:
      - { name: seed, command: "SEED" }
    checks:
      lint: { name: Lint, command: "test -e seeded" }
"#;
        serde_yaml::from_str(&yaml.replace("SEED", command)).unwrap()
    }

    #[test]
    fn test_group_setup_claimed_by_one_run_until_its_result() {
        let config = setup_config();
        let checks = vec![make_test_check("lint", "db"), make_test_check("unit", "db")];
        let mut app = make_test_app_with_checks(&config, checks);
        assert_eq!(
            app.claim_group_setup("db", "lint"),
            GroupSetup::Busy,
            "pending checks: the runner runs the pre-commands"
        );
        app.mark_cached(["lint", "unit"]);
        assert_eq!(app.pre_commands[0].status, app::PreCommandStatus::Skipped);
        assert_eq!(app.claim_group_setup("db", "lint"), GroupSetup::Run);
        assert_eq!(
            app.claim_group_setup("db", "unit"),
            GroupSetup::Busy,
            "lint's run owns them"
        );

        // Cancelled mid-setup: claim and running row released
        let (group, name) = ("db".to_string(), "seed".to_string());
        let started = RunnerEvent::PreCommandStarted {
            group: group.clone(),
            name: name.clone(),
        };
        app.handle_runner_event(started);
        app.set_retry_result(CheckResult::cancelled("lint"));
        assert_eq!(app.pre_commands[0].status, app::PreCommandStatus::Skipped);
        assert_eq!(app.run.current_pre_command, None);
        assert_eq!(app.claim_group_setup("db", "unit"), GroupSetup::Run);

        let finished = RunnerEvent::PreCommandFinished {
            group,
            name,
            success: true,
            output: String::new(),
            duration_ms: 1,
        };
        app.handle_runner_event(finished);
        app.set_retry_result(with_status("unit", CheckStatus::Passed));
        assert_eq!(app.claim_group_setup("db", "lint"), GroupSetup::Ready);
    }

    /// 'r' on a cached check while the runner still owes its group's
    /// pre-commands (a sibling is pending) does not start it
    #[test]
    fn test_retry_waits_for_group_setup_the_runner_owes() {
        let config = setup_config();
        let checks = vec![make_test_check("unit", "db"), make_test_check("lint", "db")];
        let mut app = make_test_app_with_checks(&config, checks);
        app.mark_cached(["lint"]);
        app.select_last();
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_retry_selected(&mut app, &mut tasks);

        let pre_command = &app.pre_commands[0];
        assert_eq!(
            pre_command.status,
            app::PreCommandStatus::Pending,
            "runner-owed"
        );
        assert!(app.results["lint"].cached, "not started");
        assert!(tasks.set.is_empty());
        let message = app.view.status_message.as_ref().expect("status message");
        assert!(message.text.contains("pending"), "{}", message.text);
    }

    /// 'r' on the cached `lint` of [`setup_config_with`] `seed` (group setup
    /// never ran), in a tempdir: the app after the task's final result,
    /// whether that result is one the result cache records, and whether the
    /// check's live timer started after the setup passed
    async fn retry_cached_lint(seed: &str) -> (App, bool, bool) {
        let config = setup_config_with(seed);
        let dir = tempfile::tempdir().unwrap();
        let mut check = make_test_check("lint", "db");
        check.resolved_command = "test -e seeded".to_string();
        let mut app = make_test_app_with_checks(&config, vec![check]);
        app.mark_cached(["lint"]);
        app.select_last();
        let (mut tasks, mut rx) = Tasks::new(TaskCtx {
            project_root: Arc::new(dir.path().to_path_buf()),
            exec_root: Arc::new(dir.path().to_path_buf()),
            config: Arc::new(config),
            cancels: CancelRegistry::default(),
            key_root: None,
        });

        handle_retry_selected(&mut app, &mut tasks);
        let mut timed_after_setup = false;
        let mut started = false;
        loop {
            let event = rx.recv().await.expect("task event");
            let recorded = match &event {
                TaskEvent::RetryResult(_) => Some(true),
                TaskEvent::UnrecordedResult(_) => Some(false),
                _ => None,
            };
            let starts = matches!(&event, TaskEvent::Output(RunnerEvent::CheckStarted { .. }));
            if !started {
                assert!(app.results["lint"].live_since().is_none(), "timed early");
            }
            handle_task_event(&mut app, event);
            if starts {
                started = true;
                timed_after_setup = app.pre_commands[0].status == app::PreCommandStatus::Passed
                    && app.results["lint"].live_since().is_some();
            }
            if let Some(recorded) = recorded {
                return (app, recorded, timed_after_setup);
            }
        }
    }

    /// 'r' on a cached check whose group was skipped runs the group's
    /// pre-commands first
    #[tokio::test]
    async fn test_retry_runs_group_setup_that_never_ran() {
        let (app, recorded, timed_after_setup) = retry_cached_lint("echo seeded > seeded").await;

        assert!(recorded);
        assert!(timed_after_setup, "#141: live timer excludes the setup");
        assert_eq!(app.results["lint"].status, CheckStatus::Passed);
        assert_eq!(app.pre_commands[0].status, app::PreCommandStatus::Passed);
    }

    /// A failed group setup fails the check without it running: not
    /// recorded, so its cached pass stays; the next run retries the setup
    #[tokio::test]
    async fn test_failed_group_setup_is_not_recorded() {
        let (app, recorded, _) = retry_cached_lint("exit 1").await;

        assert!(!recorded);
        assert_eq!(app.results["lint"].status, CheckStatus::Failed);
        assert_eq!(app.pre_commands[0].status, app::PreCommandStatus::Failed);
        assert_eq!(app.group_setup("db"), GroupSetup::Run);
    }

    /// 't' runs through the task channel; output streams as
    /// `TaskEvent::Output` before the final result
    #[tokio::test]
    async fn test_trigger_on_demand_streams_output() {
        let mut config = test_config();
        config.runner = crate::config::ExecTarget::Local(crate::config::LocalConfig {
            shell: "sh".to_string(),
            ..Default::default()
        });
        let mut check = make_test_check("php-lint", "fast");
        check.files = crate::checks::CheckFiles::OnDemand;
        check.resolved_command = "echo streamed".to_string();
        let mut app = make_test_app_with_checks(&config, vec![check]);
        let (mut tasks, mut rx) = Tasks::new(TaskCtx {
            project_root: Arc::new(PathBuf::from("/tmp")),
            exec_root: Arc::new(PathBuf::from("/tmp")),
            config: Arc::new(config.clone()),
            cancels: CancelRegistry::default(),
            key_root: None,
        });

        handle_trigger_on_demand(&mut app, &mut tasks);

        let mut saw_output = false;
        loop {
            let event = rx.recv().await.expect("task event");
            let done = matches!(event, TaskEvent::RetryResult(_));
            if matches!(&event, TaskEvent::Output(RunnerEvent::CheckOutput { stdout, .. }) if stdout == "streamed\n")
            {
                handle_task_event(&mut app, event);
                assert_eq!(app.results["php-lint"].output, "streamed\n");
                saw_output = true;
                continue;
            }
            handle_task_event(&mut app, event);
            if done {
                break;
            }
        }
        assert!(saw_output, "output streamed before RetryResult");
        assert_eq!(
            app.results["php-lint"].status,
            crate::runner::CheckStatus::Passed
        );
    }

    #[tokio::test]
    async fn test_fix_selected_does_not_stream_output() {
        let mut config = test_config();
        config.runner = crate::config::ExecTarget::Local(crate::config::LocalConfig {
            shell: "sh".to_string(),
            ..Default::default()
        });
        let mut check = make_test_check("php-lint", "fast");
        check.resolved_fix_command = Some("echo fixed".to_string());
        let mut app = make_test_app_with_checks(&config, vec![check]);
        app.results.get_mut("php-lint").unwrap().status = crate::runner::CheckStatus::Failed;
        let (mut tasks, mut rx) = Tasks::new(TaskCtx {
            project_root: Arc::new(PathBuf::from("/tmp")),
            exec_root: Arc::new(PathBuf::from("/tmp")),
            config: Arc::new(config.clone()),
            cancels: CancelRegistry::default(),
            key_root: None,
        });

        handle_fix_selected(&mut app, &mut tasks);

        match rx.recv().await.expect("task event") {
            TaskEvent::FixResult(result) => assert_eq!(result.output, "fixed\n"),
            TaskEvent::Output(_) => panic!("fix output must not stream"),
            _ => panic!("unexpected event"),
        }
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

    /// Local-mode app (in `dir`, not a git repo: refreshes fail and keep
    /// the check) whose failed checks `(id, fix command)` re-run `echo
    /// verified`; fix verification `verify`
    fn fixable_app(
        dir: &Path,
        fixes: &[(&str, &str)],
        verify: bool,
    ) -> (App, Tasks, mpsc::Receiver<TaskEvent>) {
        let mut config = test_config();
        config.runner = crate::config::ExecTarget::Local(crate::config::LocalConfig {
            shell: "sh".to_string(),
            ..Default::default()
        });
        let checks = fixes.iter().map(|(id, fix)| {
            let mut check = make_test_check(id, "fast");
            check.resolved_command = "echo verified".to_string();
            check.resolved_fix_command = Some(fix.to_string());
            check
        });
        let mut app = make_test_app_with_checks(&config, checks.collect());
        finish_run(&mut app, &config, None);
        for (id, _) in fixes {
            app.results.get_mut(*id).unwrap().status = CheckStatus::Failed;
        }
        app.fix.verify = verify;
        let (tasks, rx) = Tasks::new(TaskCtx {
            project_root: Arc::new(dir.to_path_buf()),
            exec_root: Arc::new(dir.to_path_buf()),
            config: Arc::new(config),
            cancels: CancelRegistry::default(),
            key_root: None,
        });
        (app, tasks, rx)
    }

    /// Handle task events until (and including) the first `until` matches
    async fn handle_until(
        app: &mut App,
        tasks: &mut Tasks,
        rx: &mut mpsc::Receiver<TaskEvent>,
        until: fn(&TaskEvent) -> bool,
    ) {
        loop {
            let event = rx.recv().await.expect("task event");
            let done = until(&event);
            handle_message(app, Message::Task(event), tasks);
            if done {
                return;
            }
        }
    }

    /// 'x': a passed fix re-runs the check like 'r' (running, cancellable,
    /// keeps `--exit-on-finish` waiting), result recorded like a retry's;
    /// the fix result stays shown
    #[tokio::test]
    async fn test_fix_selected_verifies_by_rerunning_check() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, mut rx) = fixable_app(dir.path(), &[("php-lint", "true")], true);

        handle_fix_selected(&mut app, &mut tasks);
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::FixResult(_))
        })
        .await;

        assert_eq!(app.results["php-lint"].status, CheckStatus::Running);
        assert!(app.selected_capabilities().can_cancel, "'s' cancels it");
        assert!(app.busy() && !exit_due(&app, true, false), "exit waits");
        assert!(app.fix.result.is_some(), "fix result still shown");

        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            let unrecorded = matches!(e, TaskEvent::UnrecordedResult(_));
            assert!(!unrecorded, "verify result must be recorded");
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;
        assert_eq!(app.results["php-lint"].status, CheckStatus::Passed);
        assert!(app.results["php-lint"].output.contains("verified"));
        assert!(app.fix.result.is_some(), "fix result still shown");
        assert!(exit_due(&app, true, false));
    }

    /// 'x' without verification (`--no-verify`) or with a failed fix: no re-run
    #[rstest::rstest]
    #[case::no_verify("true", false)]
    #[case::fix_failed("false", true)]
    #[tokio::test]
    async fn test_fix_selected_without_verification(#[case] fix: &str, #[case] verify: bool) {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, mut rx) = fixable_app(dir.path(), &[("php-lint", fix)], verify);

        handle_fix_selected(&mut app, &mut tasks);
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::FixResult(_))
        })
        .await;

        assert_eq!(app.results["php-lint"].status, CheckStatus::Failed);
        assert!(!app.busy());
        while tasks.set.join_next().await.is_some() {}
        assert!(rx.try_recv().is_err(), "nothing re-run");
    }

    /// 'X': once all fixes ran, each check whose fix passed re-runs; a
    /// failed fix leaves its check failed
    #[tokio::test]
    async fn test_fix_all_verifies_passed_fixes() {
        let dir = tempfile::tempdir().unwrap();
        let fixes = [("php-lint", "true"), ("phpstan", "false")];
        let (mut app, mut tasks, mut rx) = fixable_app(dir.path(), &fixes, true);

        handle_fix_all(&mut app, &mut tasks);
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::FixAllDone)
        })
        .await;

        assert_eq!(app.results["php-lint"].status, CheckStatus::Running);
        assert_eq!(app.results["phpstan"].status, CheckStatus::Failed);
        assert_eq!(app.fix.all_results.len(), 2, "fix-all results still shown");
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;
        assert_eq!(app.results["php-lint"].status, CheckStatus::Passed);
        assert_eq!(app.results["phpstan"].status, CheckStatus::Failed);
    }

    /// 'X' with two fixed checks of a group whose setup is due: the group's
    /// pre-commands run once, then the checks one at a time; no git refresh
    #[tokio::test]
    async fn test_fix_all_verifies_one_at_a_time_after_setup_once() {
        let dir = tempfile::tempdir().unwrap();
        let config = setup_config();
        let checks = ["lint", "unit"].map(|id| {
            let mut check = make_test_check(id, "db");
            check.resolved_command = "test -e seeded".to_string();
            check.resolved_fix_command = Some("true".to_string());
            check
        });
        let mut app = make_test_app_with_checks(&config, checks.to_vec());
        finish_run(&mut app, &config, None);
        for id in ["lint", "unit"] {
            app.results.get_mut(id).unwrap().status = CheckStatus::Failed;
        }
        app.fix.verify = true;
        let (mut tasks, mut rx) = Tasks::new(TaskCtx {
            project_root: Arc::new(dir.path().to_path_buf()),
            exec_root: Arc::new(dir.path().to_path_buf()),
            config: Arc::new(config),
            cancels: CancelRegistry::default(),
            key_root: None,
        });

        handle_fix_all(&mut app, &mut tasks);
        let mut seen = Vec::new();
        while seen
            .iter()
            .filter(|s: &&String| s.starts_with("done"))
            .count()
            < 2
        {
            let event = rx.recv().await.expect("task event");
            match &event {
                TaskEvent::Output(RunnerEvent::PreCommandStarted { name, .. }) => {
                    seen.push(format!("setup {name}"))
                }
                TaskEvent::Output(RunnerEvent::CheckStarted { check_id }) => {
                    seen.push(format!("start {check_id}"))
                }
                TaskEvent::RetryResult(r) => seen.push(format!("done {}", r.check_id)),
                TaskEvent::UnrecordedResult(r) | TaskEvent::CheckNotApplicable(r) => {
                    panic!("unexpected {r:?}")
                }
                TaskEvent::CheckRefreshed { .. } => panic!("no git refresh"),
                _ => {}
            }
            handle_message(&mut app, Message::Task(event), &mut tasks);
        }

        let expected = [
            "setup seed",
            "start lint",
            "done lint",
            "start unit",
            "done unit",
        ];
        assert_eq!(seen, expected);
        assert_eq!(app.results["lint"].status, CheckStatus::Passed);
        assert_eq!(app.results["unit"].status, CheckStatus::Passed);
    }

    /// The verification claims the group setup like a single run: it waits
    /// (not started) while the runner still owes the group's pre-commands
    #[test]
    fn test_fix_verification_waits_for_group_setup() {
        let config = setup_config();
        let mut lint = make_test_check("lint", "db");
        lint.resolved_fix_command = Some("true".to_string());
        let checks = vec![make_test_check("unit", "db"), lint];
        let mut app = make_test_app_with_checks(&config, checks);
        app.results.get_mut("lint").unwrap().status = CheckStatus::Failed;
        app.fix.verify = true;
        let (mut tasks, _rx) = make_test_tasks(&config);

        let fixed = CheckResult {
            status: CheckStatus::Passed,
            ..CheckResult::pending("lint")
        };
        handle_message(
            &mut app,
            Message::Task(TaskEvent::FixResult(fixed)),
            &mut tasks,
        );

        assert_eq!(app.results["lint"].status, CheckStatus::Failed);
        assert!(tasks.set.is_empty());
        let message = app.view.status_message.as_ref().expect("status message");
        assert!(message.text.contains("pending"), "{}", message.text);
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// Set up the output panel so it has something to scroll: long output,
    /// a small visible window, and a known-size rendered area.
    fn app_with_scrollable_output(config: &CiConfig) -> App {
        let mut app = make_test_app_with_checks(config, vec![make_test_check("php-lint", "fast")]);
        app.results.get_mut("php-lint").unwrap().output = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.set_output_visible_lines(5);
        app.output_area_width = 80;
        app.view.output_area = ratatui::layout::Rect::new(10, 0, 80, 7);
        app
    }

    #[tokio::test]
    async fn test_mouse_wheel_scrolls_output_panel_by_small_increment() {
        let config = test_config();
        let mut app = app_with_scrollable_output(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        let action = handle_message(
            &mut app,
            Message::Mouse(mouse(MouseEventKind::ScrollDown, 20, 3)),
            &mut tasks,
        );

        assert!(matches!(action, Action::Continue));
        assert_eq!(
            app.view.output_scroll, MOUSE_SCROLL_LINES,
            "one wheel tick scrolls by the small increment"
        );
        assert!(
            MOUSE_SCROLL_LINES < PAGE_SCROLL_LINES,
            "wheel scroll must be smaller than a page scroll"
        );

        handle_message(
            &mut app,
            Message::Mouse(mouse(MouseEventKind::ScrollUp, 20, 3)),
            &mut tasks,
        );
        assert_eq!(app.view.output_scroll, 0);
    }

    #[tokio::test]
    async fn test_mouse_wheel_outside_output_panel_is_ignored() {
        let config = test_config();
        let mut app = app_with_scrollable_output(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        // Column 0 is left of the output panel (which starts at x=10)
        handle_message(
            &mut app,
            Message::Mouse(mouse(MouseEventKind::ScrollDown, 0, 3)),
            &mut tasks,
        );

        assert_eq!(
            app.view.output_scroll, 0,
            "scroll outside panel must be a no-op"
        );
    }

    #[tokio::test]
    async fn test_mouse_click_selects_check() {
        let config = test_config();
        let mut app = make_test_app_with_checks(
            &config,
            vec![
                make_test_check("php-lint", "fast"),
                make_test_check("phpstan", "fast"),
            ],
        );
        let (mut tasks, _rx) = make_test_tasks(&config);
        // set_checks_list_layout takes the inner content rect (no border rows)
        let area = ratatui::layout::Rect::new(0, 0, 40, 8);
        // Single group "fast": header at content row 0, php-lint row 1, phpstan row 2
        app.set_checks_list_layout(area);

        let action = handle_message(
            &mut app,
            Message::Mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 2)),
            &mut tasks,
        );

        assert!(matches!(action, Action::Continue));
        assert_eq!(
            app.selected_check().map(|c| c.id()),
            Some("phpstan"),
            "clicking a row selects that check, like keyboard navigation"
        );
    }

    #[test]
    fn test_help_key_toggles_overlay() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        let action = handle_key_event(
            &mut app,
            press(KeyCode::Char('?'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(matches!(action, Action::Continue));
        assert!(app.view.help_visible);

        handle_key_event(
            &mut app,
            press(KeyCode::Esc, KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(!app.view.help_visible);
    }

    #[test]
    fn test_m_key_toggles_stats_panel() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        let m = || press(KeyCode::Char('m'), KeyModifiers::NONE);

        handle_key_event(&mut app, m(), &mut tasks);
        assert!(!app.stats_visible());
        handle_key_event(&mut app, m(), &mut tasks);
        assert!(app.stats_visible());
    }

    #[test]
    fn test_help_overlay_swallows_other_keys() {
        let config = test_config();
        let mut app = make_test_app_with_checks(&config, vec![make_test_check("php-lint", "fast")]);
        app.results.get_mut("php-lint").unwrap().status = crate::runner::CheckStatus::Failed;
        let (mut tasks, _rx) = make_test_tasks(&config);
        app.toggle_help();

        // r/x/s must not retry/fix/cancel while help is shown
        handle_key_event(
            &mut app,
            press(KeyCode::Char('r'), KeyModifiers::NONE),
            &mut tasks,
        );
        handle_key_event(
            &mut app,
            press(KeyCode::Char('x'), KeyModifiers::NONE),
            &mut tasks,
        );
        handle_key_event(
            &mut app,
            press(KeyCode::Char('s'), KeyModifiers::NONE),
            &mut tasks,
        );
        handle_key_event(
            &mut app,
            press(KeyCode::Char('j'), KeyModifiers::NONE),
            &mut tasks,
        );

        assert!(app.view.help_visible, "still open after unrelated keys");
        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            crate::runner::CheckStatus::Failed,
            "no retry/fix/cancel dispatched while help is shown"
        );
        assert_eq!(
            app.selected_check().map(|c| c.id()),
            Some("php-lint"),
            "list nav swallowed too"
        );

        // '?' closes it again
        handle_key_event(
            &mut app,
            press(KeyCode::Char('?'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(!app.view.help_visible);
    }

    #[test]
    fn test_g_and_shift_g_jump_list_selection() {
        let config = test_config();
        let mut app = make_test_app_with_checks(
            &config,
            vec![
                make_test_check("php-lint", "fast"),
                make_test_check("phpstan", "fast"),
                make_test_check("phpunit", "fast"),
            ],
        );
        let (mut tasks, _rx) = make_test_tasks(&config);
        app.view.selected_check = 1;

        handle_key_event(
            &mut app,
            press(KeyCode::Char('G'), KeyModifiers::SHIFT),
            &mut tasks,
        );
        assert_eq!(app.view.selected_check, 3, "header + 3 checks");

        handle_key_event(
            &mut app,
            press(KeyCode::Char('g'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert_eq!(app.view.selected_check, 0, "g lands on the group header");
    }

    #[test]
    fn test_home_and_end_scroll_output() {
        let config = test_config();
        let mut app = app_with_scrollable_output(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_key_event(
            &mut app,
            press(KeyCode::End, KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(app.view.output_scroll > 0, "End scrolls to the bottom");

        handle_key_event(
            &mut app,
            press(KeyCode::Home, KeyModifiers::NONE),
            &mut tasks,
        );
        assert_eq!(app.view.output_scroll, 0, "Home scrolls to the top");
    }

    #[test]
    fn test_shift_j_and_k_scroll_output_by_one_line() {
        let config = test_config();
        let mut app = app_with_scrollable_output(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('J'), KeyModifiers::SHIFT),
            &mut tasks,
        );
        assert_eq!(app.view.output_scroll, 1);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('K'), KeyModifiers::SHIFT),
            &mut tasks,
        );
        assert_eq!(app.view.output_scroll, 0);

        // lowercase j/k must remain list navigation, not output scroll
        handle_key_event(
            &mut app,
            press(KeyCode::Char('j'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert_eq!(app.view.output_scroll, 0);
        assert_eq!(app.view.selected_check, 1, "only one check in this fixture");
    }

    #[test]
    fn test_ctrl_d_and_ctrl_u_scroll_half_page() {
        let config = test_config();
        let mut app = app_with_scrollable_output(&config);
        app.set_output_visible_lines(10); // half page = 5
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('d'), KeyModifiers::CONTROL),
            &mut tasks,
        );
        assert_eq!(app.view.output_scroll, 5);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &mut tasks,
        );
        assert_eq!(app.view.output_scroll, 0);
    }

    #[test]
    fn test_n_and_shift_n_navigate_failed_checks() {
        let config = test_config();
        let mut app = make_test_app_with_checks(
            &config,
            vec![
                make_test_check("php-lint", "fast"),
                make_test_check("phpstan", "fast"),
            ],
        );
        app.results.get_mut("phpstan").unwrap().status = crate::runner::CheckStatus::Failed;
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('n'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert_eq!(app.view.selected_check, 2, "jumps to the only failed check");

        handle_key_event(
            &mut app,
            press(KeyCode::Char('N'), KeyModifiers::SHIFT),
            &mut tasks,
        );
        assert_eq!(app.view.selected_check, 2, "wraps back to itself");
    }

    #[test]
    fn test_slash_opens_search_and_types_query() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('/'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(app.view.search.as_ref().unwrap().typing);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('e'), KeyModifiers::NONE),
            &mut tasks,
        );
        handle_key_event(
            &mut app,
            press(KeyCode::Char('r'), KeyModifiers::NONE),
            &mut tasks,
        );
        handle_key_event(
            &mut app,
            press(KeyCode::Char('r'), KeyModifiers::NONE),
            &mut tasks,
        );
        assert_eq!(app.view.search.as_ref().unwrap().query, "err");

        // while typing, keys build the query rather than dispatching (e.g. 'e' expand)
        assert!(!app.view.show_full_command);
    }

    #[test]
    fn test_search_esc_cancels_and_clears_highlight() {
        let config = test_config();
        let mut app = app_with_scrollable_output(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('/'), KeyModifiers::NONE),
            &mut tasks,
        );
        handle_key_event(
            &mut app,
            press(KeyCode::Char('5'), KeyModifiers::NONE),
            &mut tasks,
        );
        handle_key_event(
            &mut app,
            press(KeyCode::Enter, KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(app.view.search.is_some());

        handle_key_event(
            &mut app,
            press(KeyCode::Esc, KeyModifiers::NONE),
            &mut tasks,
        );
        assert!(
            app.view.search.is_none(),
            "Esc clears search state entirely"
        );
    }

    #[test]
    fn test_search_enter_confirms_jumps_and_finds_matches() {
        let config = test_config();
        let mut app = app_with_scrollable_output(&config); // lines "line 1".."line 20"
        let (mut tasks, _rx) = make_test_tasks(&config);

        handle_key_event(
            &mut app,
            press(KeyCode::Char('/'), KeyModifiers::NONE),
            &mut tasks,
        );
        for c in "line 15".chars() {
            handle_key_event(
                &mut app,
                press(KeyCode::Char(c), KeyModifiers::NONE),
                &mut tasks,
            );
        }
        handle_key_event(
            &mut app,
            press(KeyCode::Enter, KeyModifiers::NONE),
            &mut tasks,
        );

        let search = app
            .view
            .search
            .as_ref()
            .expect("search stays open after confirm");
        assert!(!search.typing, "Enter confirms out of typing mode");
        assert_eq!(search.matches.len(), 1, "only one line contains 'line 15'");
    }

    #[tokio::test]
    async fn test_mouse_click_outside_checks_list_is_ignored() {
        let config = test_config();
        let mut app = make_test_app_with_checks(
            &config,
            vec![
                make_test_check("php-lint", "fast"),
                make_test_check("phpstan", "fast"),
            ],
        );
        let (mut tasks, _rx) = make_test_tasks(&config);
        let area = ratatui::layout::Rect::new(0, 0, 40, 10);
        app.set_checks_list_layout(area);

        handle_message(
            &mut app,
            Message::Mouse(mouse(MouseEventKind::Down(MouseButton::Left), 100, 100)),
            &mut tasks,
        );

        assert_eq!(
            app.selected_check().map(|c| c.id()),
            Some("php-lint"),
            "click outside the checks list must not change selection"
        );
    }

    /// Two checks in group "fast" (rows: header, php-lint, phpstan)
    fn make_fold_app(config: &CiConfig) -> App {
        make_test_app_with_checks(
            config,
            vec![
                make_test_check("php-lint", "fast"),
                make_test_check("phpstan", "fast"),
            ],
        )
    }

    #[test]
    fn test_key_and_mouse_input_mark_user_interacted() {
        let config = test_config();
        let (mut tasks, _rx) = make_test_tasks(&config);

        let messages = [
            Message::KeyPress(press(KeyCode::Char('j'), KeyModifiers::NONE)),
            Message::Mouse(mouse(MouseEventKind::ScrollDown, 20, 3)),
            Message::Mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, 0)),
        ];
        for msg in messages {
            let mut app = app_with_scrollable_output(&config);
            assert!(!app.run.user_interacted);
            handle_message(&mut app, msg, &mut tasks);
            assert!(app.run.user_interacted);
        }

        let mut app = app_with_scrollable_output(&config);
        handle_message(&mut app, Message::Resize, &mut tasks);
        assert!(!app.run.user_interacted, "resize is not user input");
    }

    #[test]
    fn test_space_and_enter_toggle_fold_on_header() {
        let config = test_config();
        let mut app = make_fold_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        app.select_first(); // fast header

        for code in [KeyCode::Char(' '), KeyCode::Enter] {
            handle_key_event(&mut app, press(code, KeyModifiers::NONE), &mut tasks);
            assert!(app.is_group_collapsed("fast"), "{:?} folds", code);
            assert_eq!(app.selectable_items().count(), 1, "only the header");

            handle_key_event(&mut app, press(code, KeyModifiers::NONE), &mut tasks);
            assert!(!app.is_group_collapsed("fast"), "{:?} unfolds", code);
            assert_eq!(app.selectable_items().count(), 3);
        }
    }

    #[test]
    fn test_modified_space_and_enter_do_not_fold() {
        let config = test_config();
        let mut app = make_fold_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        app.select_first(); // fast header

        for code in [KeyCode::Char(' '), KeyCode::Enter] {
            for mods in [
                KeyModifiers::CONTROL,
                KeyModifiers::ALT,
                KeyModifiers::SHIFT,
            ] {
                handle_key_event(&mut app, press(code, mods), &mut tasks);
            }
        }

        assert!(!app.is_group_collapsed("fast"));
    }

    #[test]
    fn test_space_and_enter_on_check_do_nothing() {
        let config = test_config();
        let mut app = make_fold_app(&config); // php-lint selected
        let (mut tasks, _rx) = make_test_tasks(&config);

        for code in [KeyCode::Char(' '), KeyCode::Enter] {
            handle_key_event(&mut app, press(code, KeyModifiers::NONE), &mut tasks);
        }

        assert!(!app.is_group_collapsed("fast"));
        assert_eq!(app.selected_check().map(|c| c.id()), Some("php-lint"));
        assert!(tasks.set.is_empty());
    }

    #[test]
    fn test_enter_in_search_confirms_instead_of_folding() {
        let config = test_config();
        let mut app = make_fold_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        app.select_first(); // fast header

        for code in [KeyCode::Char('/'), KeyCode::Char(' '), KeyCode::Enter] {
            handle_key_event(&mut app, press(code, KeyModifiers::NONE), &mut tasks);
        }

        let search = app.view.search.as_ref().expect("search confirmed");
        assert!(!search.typing);
        assert_eq!(search.query, " ", "space typed into the query");
        assert!(!app.is_group_collapsed("fast"));
    }

    #[test]
    fn test_check_actions_on_header_are_noops() {
        let config = test_config();
        let mut app = make_fold_app(&config);
        for id in ["php-lint", "phpstan"] {
            app.results.get_mut(id).unwrap().status = crate::runner::CheckStatus::Failed;
        }
        let (mut tasks, _rx) = make_test_tasks(&config);
        app.select_first(); // fast header

        for (c, m) in [
            ('r', KeyModifiers::NONE),
            ('s', KeyModifiers::NONE),
            ('c', KeyModifiers::NONE),
            ('x', KeyModifiers::NONE),
            ('e', KeyModifiers::NONE),
            ('t', KeyModifiers::NONE),
            ('A', KeyModifiers::SHIFT),
        ] {
            let action = handle_key_event(&mut app, press(KeyCode::Char(c), m), &mut tasks);
            assert!(matches!(action, Action::Continue));
        }

        assert!(tasks.set.is_empty(), "nothing spawned");
        assert!(app.view.status_message.is_none(), "c copied nothing");
        assert!(!app.view.show_full_command);
        assert!(!app.fix.running);
        for id in ["php-lint", "phpstan"] {
            assert_eq!(
                app.results[id].status,
                crate::runner::CheckStatus::Failed,
                "{id} untouched"
            );
        }
    }

    #[tokio::test]
    async fn test_mouse_click_selects_group_header() {
        let config = test_config();
        let mut app = make_fold_app(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        app.set_checks_list_layout(ratatui::layout::Rect::new(0, 0, 40, 8));

        handle_message(
            &mut app,
            Message::Mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 0)),
            &mut tasks,
        );

        assert!(matches!(
            app.selected_item(),
            Some(app::SelectableItem::Group("fast"))
        ));
    }

    fn status_text(app: &App) -> Option<&str> {
        app.view.status_message.as_ref().map(|m| m.text.as_str())
    }

    /// App with a failed `php-lint` (selected) that has output
    fn app_with_output(config: &CiConfig) -> App {
        let mut app = make_test_app_with_checks(config, vec![make_test_check("php-lint", "fast")]);
        let result = app.results.get_mut("php-lint").unwrap();
        result.status = crate::runner::CheckStatus::Failed;
        result.output = "\x1b[31mParse error\x1b[0m\n".to_string();
        app
    }

    #[test]
    fn test_save_log_key_writes_plain_output() {
        let config = test_config();
        let mut app = app_with_output(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);
        let dir = tempfile::tempdir().unwrap();
        tasks.ctx.project_root = Arc::new(dir.path().to_path_buf());

        let key = press(KeyCode::Char('w'), KeyModifiers::NONE);
        let action = handle_message(&mut app, Message::KeyPress(key), &mut tasks);

        assert!(matches!(action, Action::Continue));
        let log = std::fs::read_to_string(dir.path().join(".ci-tui/logs/php-lint.log")).unwrap();
        assert!(log.contains("Parse error\n"), "{log}");
        assert!(!log.contains('\x1b'), "ANSI stripped");
        assert_eq!(
            status_text(&app),
            Some("Saved log to .ci-tui/logs/php-lint.log")
        );
    }

    #[test]
    fn test_save_log_key_failure_shows_error() {
        let config = test_config();
        let mut app = app_with_output(&config);
        let (mut tasks, _rx) = make_test_tasks(&config); // root does not exist
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "").unwrap();
        tasks.ctx.project_root = Arc::new(file); // Edge case: root is a file

        handle_key_event(
            &mut app,
            press(KeyCode::Char('w'), KeyModifiers::NONE),
            &mut tasks,
        );

        let message = app.view.status_message.as_ref().expect("error shown");
        assert!(matches!(message.kind, StatusKind::Error));
        assert!(
            message.text.starts_with("Save log failed"),
            "{}",
            message.text
        );
    }

    #[rstest::rstest]
    #[case::save('w', KeyModifiers::NONE)]
    #[case::pager('o', KeyModifiers::NONE)]
    #[case::editor('O', KeyModifiers::SHIFT)]
    fn test_output_keys_without_selected_check_are_noop(
        #[case] c: char,
        #[case] modifiers: KeyModifiers,
    ) {
        let config = test_config();
        let mut app = make_test_app(&config); // no checks: nothing selected
        let (mut tasks, _rx) = make_test_tasks(&config);
        let dir = tempfile::tempdir().unwrap();
        tasks.ctx.project_root = Arc::new(dir.path().to_path_buf());

        let action = handle_key_event(&mut app, press(KeyCode::Char(c), modifiers), &mut tasks);

        assert!(matches!(action, Action::Continue));
        assert_eq!(status_text(&app), Some("Select a check to use its output"));
        assert!(!dir.path().join(".ci-tui").exists(), "nothing written");
    }

    #[rstest::rstest]
    #[case::save('w', KeyModifiers::NONE)]
    #[case::pager('o', KeyModifiers::NONE)]
    fn test_output_keys_without_output_are_noop(#[case] c: char, #[case] modifiers: KeyModifiers) {
        let config = test_config();
        // Selected check never ran: pending, no output
        let mut app = make_test_app_with_checks(&config, vec![make_test_check("php-lint", "fast")]);
        let (mut tasks, _rx) = make_test_tasks(&config);

        let action = handle_key_event(&mut app, press(KeyCode::Char(c), modifiers), &mut tasks);

        assert!(matches!(action, Action::Continue));
        assert_eq!(status_text(&app), Some("No output yet for this check"));
    }

    #[rstest::rstest]
    #[case::pager('o', KeyModifiers::NONE, Viewer::Pager)]
    #[case::editor('O', KeyModifiers::SHIFT, Viewer::Editor)]
    fn test_open_keys_request_viewer_with_plain_output(
        #[case] c: char,
        #[case] modifiers: KeyModifiers,
        #[case] expected: Viewer,
    ) {
        let config = test_config();
        let mut app = app_with_output(&config);
        let (mut tasks, _rx) = make_test_tasks(&config);

        let action = handle_key_event(&mut app, press(KeyCode::Char(c), modifiers), &mut tasks);

        let Action::OpenViewer(request) = action else {
            panic!("expected OpenViewer");
        };
        assert_eq!(request.viewer, expected);
        assert_eq!(request.check_id, "php-lint");
        assert!(request.text.contains("$ php-lint src/Foo.php\n"));
        assert!(request.text.contains("Parse error\n"));
        assert!(!request.text.contains('\x1b'));
    }

    /// Edge case: local-mode `php-lint` / `yaml-lint` triggered by PHP / YAML
    /// files, echoing their files
    fn watch_config() -> CiConfig {
        serde_yaml::from_str(
            r#"
version: 2
runner: local
local: { shell: sh }
git: { base_branch: main, fallback_branch: HEAD~1 }
file_patterns:
  php: { pattern: '\.php$' }
  yaml: { pattern: '\.ya?ml$' }
checks:
  fast:
    checks:
      php-lint: { name: PHP lint, command: "echo lint {files}", triggers: { file_pattern: php } }
      yaml-lint: { name: YAML lint, command: "echo yaml {files}", triggers: { file_pattern: yaml } }
"#,
        )
        .unwrap()
    }

    /// App in `dir` on `--files` `files` (refreshes keep them, no git) with
    /// the checks they select (`keep` filters them); the run finished, every
    /// check that ran passed
    fn watch_app(
        dir: &Path,
        files: &[&str],
        keep: fn(&CheckToRun) -> bool,
    ) -> (App, Tasks, mpsc::Receiver<TaskEvent>) {
        watch_app_with(watch_config(), dir, files, keep)
    }

    /// [`watch_app`] with `config`
    fn watch_app_with(
        config: CiConfig,
        dir: &Path,
        files: &[&str],
        keep: fn(&CheckToRun) -> bool,
    ) -> (App, Tasks, mpsc::Receiver<TaskEvent>) {
        let changed_files = ChangedFiles {
            files: files.iter().map(|f| f.to_string()).collect(),
            base_ref: crate::git::CLI_FILES_BASE_REF.to_string(),
        };
        let mut checks = select_checks(&config, &changed_files, dir, None).checks;
        checks.retain(keep);
        let mut app = App::new(config.clone(), changed_files, checks, "main".to_string());
        finish_run(&mut app, &config, None);
        let ran = app.results.values_mut();
        ran.filter(|r| r.status == CheckStatus::Pending)
            .for_each(|r| r.status = CheckStatus::Passed);
        let cancels = CancelRegistry::default();
        let ctx = TaskCtx::new(dir.to_path_buf(), dir, Arc::new(config), &cancels, None);
        let (tasks, rx) = Tasks::new(ctx);
        (app, tasks, rx)
    }

    /// `--watch` batch for saving `files` in `dir` now
    fn saved(dir: &Path, files: &[&str]) -> Message {
        saved_at(&watch_config(), dir, files, std::time::Instant::now())
    }

    /// `--watch` batch for saving `files` in `dir` at `at` (`config`)
    fn saved_at(config: &CiConfig, dir: &Path, files: &[&str], at: std::time::Instant) -> Message {
        let files = files.iter().map(|f| f.to_string()).collect();
        watch_batch(crate::watch::affected_checks(config, files, dir), at)
    }

    /// `--watch` batch of `checks` saved at `at`
    fn watch_batch(checks: Vec<CheckToRun>, at: std::time::Instant) -> Message {
        Message::Watch(WatchBatch {
            checks,
            first: at,
            last: at,
        })
    }

    /// Saving a PHP file re-runs `php-lint` like 'r' (git refresh, then the
    /// check, recorded); `yaml-lint` keeps its result
    #[tokio::test]
    async fn test_watch_reruns_only_affected_checks() {
        let dir = tempfile::tempdir().unwrap();
        let files = &["src/Foo.php", "a.yaml"];
        let (mut app, mut tasks, mut rx) = watch_app(dir.path(), files, |_| true);

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);

        assert_eq!(app.results["php-lint"].status, CheckStatus::Running);
        assert_eq!(app.results["yaml-lint"].status, CheckStatus::Passed);
        assert_eq!(tasks.set.len(), 1, "one re-run");
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            let unrecorded = matches!(e, TaskEvent::UnrecordedResult(_));
            assert!(!unrecorded, "re-run result must be recorded");
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;
        assert_eq!(app.results["php-lint"].status, CheckStatus::Passed);
        assert!(app.results["php-lint"].output.contains("lint src/Foo.php"));
        assert_eq!(app.results["yaml-lint"].status, CheckStatus::Passed);
    }

    /// A save selecting a check this run does not list is ignored (the
    /// list holds every check the config can select)
    #[tokio::test]
    async fn test_watch_ignores_unlisted_check() {
        let dir = tempfile::tempdir().unwrap();
        let files = &["src/Foo.php", "a.yaml"];
        let (mut app, mut tasks, _rx) = watch_app(dir.path(), files, |c| c.id() != "php-lint");

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);

        assert!(!app.checks.iter().any(|c| c.id() == "php-lint"));
        assert!(!app.results.contains_key("php-lint"));
        assert!(tasks.set.is_empty());
        assert!(app.run.watch_queue.is_empty());
    }

    /// A running affected check (saved during its run) is cancelled; the
    /// new run starts only once the cancelled result arrived, so that
    /// result cannot overwrite it
    #[tokio::test]
    async fn test_watch_restarts_running_check_after_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, mut rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        app.mark_running("php-lint");
        spawn_cancellable(&mut tasks, "php-lint");
        tokio::task::yield_now().await; // let the task register

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);

        assert_eq!(app.results["php-lint"].status, CheckStatus::Running);
        assert_eq!(app.run.watch_queue.len(), 1, "waits for the cancel");
        let stale = rx.recv().await.expect("cancelled result");
        assert!(
            matches!(&stale, TaskEvent::RetryResult(r) if r.status == CheckStatus::Cancelled),
            "{stale:?}"
        );
        handle_message(&mut app, Message::Task(stale), &mut tasks);
        assert_eq!(
            app.results["php-lint"].status,
            CheckStatus::Running,
            "restarted, not left cancelled"
        );
        assert!(app.run.watch_queue.is_empty());
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;
        assert_eq!(app.results["php-lint"].status, CheckStatus::Passed);
    }

    /// A save for checks the runner still runs (pending, queued) waits;
    /// once the runner ran them (that run saw the save) it is dropped, not
    /// cancelling the runner's run
    #[tokio::test]
    async fn test_watch_save_for_runner_owed_check_dropped_once_runner_runs_it() {
        let dir = tempfile::tempdir().unwrap();
        let files = &["src/Foo.php", "a.yaml"];
        let (mut app, mut tasks, mut rx) = watch_app(dir.path(), files, |_| true);
        app.run.all_finished = false;
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Pending;
        app.results.get_mut("yaml-lint").unwrap().status = CheckStatus::Queued;

        handle_message(&mut app, saved(dir.path(), files), &mut tasks);

        assert!(tasks.set.is_empty(), "nothing spawned");
        assert_eq!(app.run.watch_queue.len(), 2, "kept, not lost");
        assert_eq!(app.results["php-lint"].status, CheckStatus::Pending);
        assert_eq!(app.results["yaml-lint"].status, CheckStatus::Queued);

        spawn_cancellable(&mut tasks, "php-lint"); // the runner's run
        tokio::task::yield_now().await;
        let runner = |event| Message::RunnerEvent(event);
        let started = RunnerEvent::CheckStarted {
            check_id: "php-lint".to_string(),
        };
        handle_message(&mut app, runner(started), &mut tasks);
        let group = "fast".to_string();
        handle_message(
            &mut app,
            runner(RunnerEvent::GroupFinished { group }),
            &mut tasks,
        );
        assert_eq!(app.run.watch_queue.len(), 1, "php-lint dropped");
        for id in ["php-lint", "yaml-lint"] {
            let result = with_status(id, CheckStatus::Passed);
            handle_message(
                &mut app,
                runner(RunnerEvent::CheckFinished { result }),
                &mut tasks,
            );
        }

        assert!(app.run.watch_queue.is_empty(), "both dropped");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err(), "runner's run not cancelled");
        assert_eq!(tasks.set.len(), 1, "no re-run spawned");
        assert_eq!(app.results["php-lint"].status, CheckStatus::Passed);
    }

    /// A save for a check the runner owes runs once the run ended with the
    /// check still pending (its group setup failed)
    #[tokio::test]
    async fn test_watch_save_for_runner_owed_check_runs_if_run_ends_pending() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, _rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        app.run.all_finished = false;
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Pending;

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);
        assert_eq!(app.run.watch_queue.len(), 1);
        let finished = Message::RunnerEvent(RunnerEvent::AllFinished);
        handle_message(&mut app, finished, &mut tasks);

        assert_eq!(app.results["php-lint"].status, CheckStatus::Running);
        assert_eq!(tasks.set.len(), 1);
        assert!(app.run.watch_queue.is_empty());
    }

    /// A pending check the finished run left behind (stopped by a failed
    /// group setup) does re-run
    #[tokio::test]
    async fn test_watch_runs_pending_check_of_finished_run() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, _rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        app.results.get_mut("php-lint").unwrap().status = CheckStatus::Pending;

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);

        assert_eq!(app.results["php-lint"].status, CheckStatus::Running);
        assert_eq!(tasks.set.len(), 1);
    }

    /// A check whose group pre-commands the runner still owes waits queued
    #[tokio::test]
    async fn test_watch_waits_for_group_setup() {
        let config = setup_config();
        let checks = vec![make_test_check("unit", "db"), make_test_check("lint", "db")];
        let mut app = make_test_app_with_checks(&config, checks);
        app.mark_cached(["lint"]);
        let (mut tasks, _rx) = make_test_tasks(&config);

        let batch = watch_batch(
            vec![make_test_check("lint", "db")],
            std::time::Instant::now(),
        );
        handle_message(&mut app, batch, &mut tasks);

        assert!(tasks.set.is_empty());
        assert!(app.results["lint"].cached, "not started");
        assert_eq!(app.run.watch_queue.len(), 1, "still queued");
    }

    /// A check the refresh no longer runs automatically (its file is no
    /// longer changed) shows skipped instead of running
    #[tokio::test]
    async fn test_watch_shows_skipped_when_refresh_drops_files() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, mut rx) = watch_app(dir.path(), &["a.yaml"], |_| true);
        assert_eq!(app.results["php-lint"].status, CheckStatus::Skipped);

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);

        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            assert!(!matches!(e, TaskEvent::RetryResult(_)), "must not run");
            matches!(e, TaskEvent::UnrecordedResult(_))
        })
        .await;
        assert_eq!(app.results["php-lint"].status, CheckStatus::Skipped);
    }
    /// The git refresh of a `--watch` run fails: the check runs as the save
    /// selected it, never as listed (here on-demand)
    #[tokio::test]
    async fn test_watch_refresh_failure_runs_saved_check() {
        if OUTER_GIT_ENV.iter().any(|v| std::env::var_os(v).is_some()) {
            eprintln!("skipped: outer git env set (running inside a git hook)");
            return;
        }
        let dir = tempfile::tempdir().unwrap(); // not a repo: refresh fails
        let (mut app, mut tasks, mut rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        app.changed_files.base_ref = "main".to_string();
        let listed = app
            .checks
            .iter_mut()
            .find(|c| c.id() == "php-lint")
            .unwrap();
        listed.files = crate::checks::CheckFiles::OnDemand;
        listed.resolved_command = "echo listed".to_string();
        app.results
            .insert("php-lint".to_string(), CheckResult::on_demand("php-lint"));

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;

        assert_eq!(
            status_text(&app),
            Some("Git refresh failed - retrying with previous file list")
        );
        let result = &app.results["php-lint"];
        assert_eq!(result.status, CheckStatus::Passed);
        assert!(
            result.output.contains("lint src/Foo.php"),
            "{}",
            result.output
        );
        assert!(!result.output.contains("listed"), "{}", result.output);
    }

    /// No `--watch` run starts while a fix ('x') or fix-all ('X') edits
    /// files; queued ones start once it is done
    #[rstest::rstest]
    #[case::fix(false)]
    #[case::fix_all(true)]
    #[tokio::test]
    async fn test_watch_waits_for_fix(#[case] all: bool) {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, _rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        match all {
            true => app.start_fix_all(1),
            false => app.start_fix(),
        }

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);
        assert!(tasks.set.is_empty());
        assert_eq!(app.run.watch_queue.len(), 1);
        assert_eq!(app.results["php-lint"].status, CheckStatus::Passed);

        let done = match all {
            true => TaskEvent::FixAllDone,
            false => TaskEvent::FixResult(with_status("yaml-lint", CheckStatus::Failed)),
        };
        handle_message(&mut app, Message::Task(done), &mut tasks);

        assert_eq!(app.results["php-lint"].status, CheckStatus::Running);
        assert!(app.run.watch_queue.is_empty());
    }

    /// A save while a fix verification runs neither cancels it nor starts
    /// a run beside it; the re-run starts after the verification's result
    #[tokio::test]
    async fn test_watch_waits_for_fix_verification_without_cancelling_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, mut rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        verify_fixes(&mut app, &mut tasks, vec!["php-lint".to_string()]);
        assert!(app.fixing());

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);
        assert_eq!(app.run.watch_queue.len(), 1);
        assert_eq!(tasks.set.len(), 1, "only the verification");

        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            let cancelled =
                matches!(e, TaskEvent::RetryResult(r) if r.status == CheckStatus::Cancelled);
            assert!(!cancelled, "verification cancelled");
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;
        assert!(!app.fixing());
        assert_eq!(
            app.results["php-lint"].status,
            CheckStatus::Running,
            "re-run started"
        );
        assert!(app.run.watch_queue.is_empty());
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;
        assert_eq!(app.results["php-lint"].status, CheckStatus::Passed);
    }

    /// Edge case: group `serial` with checks `a` and `b`, both triggered by
    /// PHP files; `GROUP` is replaced with the group's settings
    fn serial_config(group: &str) -> CiConfig {
        let yaml = r#"
version: 2
runner: local
local: { shell: sh }
git: { base_branch: main, fallback_branch: HEAD~1 }
file_patterns:
  php: { pattern: '\.php$' }
checks:
  serial:
    GROUP
    checks:
      a: { name: A, command: "echo a {files}", triggers: { file_pattern: php } }
      b: { name: B, command: "echo b {files}", triggers: { file_pattern: php } }
"#;
        serde_yaml::from_str(&yaml.replace("GROUP", group)).unwrap()
    }

    /// `--watch` runs keep the group's concurrency: `parallel: false` (or
    /// `max_parallel: 1`) runs one at a time, the next on the first's result
    #[rstest::rstest]
    #[case::not_parallel("parallel: false")]
    #[case::max_parallel("parallel: true\n    max_parallel: 1")]
    #[tokio::test]
    async fn test_watch_runs_respect_group_concurrency(#[case] group: &str) {
        let config = serial_config(group);
        let dir = tempfile::tempdir().unwrap();
        let files = &["src/Foo.php"];
        let (mut app, mut tasks, mut rx) =
            watch_app_with(config.clone(), dir.path(), files, |_| true);

        let batch = saved_at(&config, dir.path(), files, std::time::Instant::now());
        handle_message(&mut app, batch, &mut tasks);

        assert_eq!(app.results["a"].status, CheckStatus::Running);
        assert_eq!(app.results["b"].status, CheckStatus::Passed, "waits");
        assert_eq!(app.run.watch_queue.len(), 1);
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;
        assert_eq!(app.results["a"].status, CheckStatus::Passed);
        assert_eq!(
            app.results["b"].status,
            CheckStatus::Running,
            "started on a's result"
        );
        handle_until(&mut app, &mut tasks, &mut rx, |e| {
            matches!(e, TaskEvent::RetryResult(_))
        })
        .await;
        assert_eq!(app.results["b"].status, CheckStatus::Passed);
    }

    /// A batch whose saves all fell in a check's finished run (its own
    /// writes, until a grace after its result) does not re-run it; a save
    /// after the grace does
    #[rstest::rstest]
    #[case::own_writes(false)]
    #[case::after_grace(true)]
    #[tokio::test]
    async fn test_watch_skips_own_writes_of_finished_run(#[case] after_grace: bool) {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, _rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        app.mark_running("php-lint");
        let during = std::time::Instant::now();
        app.set_retry_result(with_status("php-lint", CheckStatus::Passed));

        let at = match after_grace {
            true => std::time::Instant::now() + Duration::from_secs(2),
            false => during,
        };
        let batch = saved_at(&watch_config(), dir.path(), &["src/Foo.php"], at);
        handle_message(&mut app, batch, &mut tasks);

        let status = &app.results["php-lint"].status;
        assert_eq!(*status == CheckStatus::Running, after_grace, "{status:?}");
        assert_eq!(tasks.set.len(), usize::from(after_grace));
        assert!(app.run.watch_queue.is_empty());
    }

    /// Queued `--watch` runs are only looked at on messages that can let
    /// them start: not ticks, stats, keys, status expiry or output chunks
    #[tokio::test]
    async fn test_watch_runs_start_only_on_waking_messages() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, _rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        let check = app.checks.iter().find(|c| c.id() == "php-lint").unwrap();
        app.run.watch_queue.push(app::WatchRun::new(check.clone()));
        let stats = SystemStats {
            cpu_usage: 0.0,
            mem_used: 0,
            mem_total: 0,
        };
        let output = RunnerEvent::CheckOutput {
            check_id: "yaml-lint".to_string(),
            stdout: "x".to_string(),
            stderr: String::new(),
        };
        let idle = [
            Message::Tick,
            Message::SystemStats(stats),
            Message::StatusExpired,
            Message::Resize,
            Message::KeyPress(press(KeyCode::Char('z'), KeyModifiers::NONE)),
            Message::RunnerEvent(output.clone()),
            Message::Task(TaskEvent::Output(output)),
        ];

        for msg in idle {
            handle_message(&mut app, msg, &mut tasks);
            assert_eq!(app.run.watch_queue.len(), 1);
            assert!(tasks.set.is_empty());
        }
        let group = "fast".to_string();
        let msg = Message::RunnerEvent(RunnerEvent::GroupFinished { group });
        handle_message(&mut app, msg, &mut tasks);

        assert_eq!(app.results["php-lint"].status, CheckStatus::Running);
    }

    /// A running check is cancelled once per queued re-run, not on every
    /// later message (a later run of it would be cancelled too)
    #[tokio::test]
    async fn test_watch_cancels_running_check_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, mut tasks, mut rx) = watch_app(dir.path(), &["src/Foo.php"], |_| true);
        app.mark_running("php-lint");
        spawn_cancellable(&mut tasks, "php-lint");
        tokio::task::yield_now().await;

        handle_message(&mut app, saved(dir.path(), &["src/Foo.php"]), &mut tasks);
        let stale = rx.recv().await.expect("cancelled result");
        assert!(matches!(&stale, TaskEvent::RetryResult(r) if r.status == CheckStatus::Cancelled));
        spawn_cancellable(&mut tasks, "php-lint"); // registered before the result lands
        tokio::task::yield_now().await;
        let group = "fast".to_string();
        let msg = Message::RunnerEvent(RunnerEvent::GroupFinished { group });
        handle_message(&mut app, msg, &mut tasks);

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err(), "cancelled again");
        assert!(app.run.watch_queue[0].cancel_sent);
    }
}
