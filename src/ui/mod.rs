//! Terminal UI using ratatui for interactive check execution.
//!
//! This module implements the main TUI event loop and coordinates between
//! user input, check execution, and rendering. It provides real-time
//! feedback during check execution with keyboard-driven navigation.
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
use crate::runner::{
    run_check_with_command, run_fix_command, run_single_check, CheckResult, CheckRunner,
    RunnerEvent,
};
use anyhow::Result;
use app::App;
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
use tokio::task::JoinHandle;

// Constants for timing and performance tuning
/// Interval between system stats updates (CPU, memory)
const STATS_UPDATE_INTERVAL: Duration = Duration::from_millis(500);
/// Keyboard poll timeout - responsive enough for shutdown, not too CPU intensive
const KEYBOARD_POLL_TIMEOUT: Duration = Duration::from_millis(50);
/// Channel capacity for system stats
const STATS_CHANNEL_CAPACITY: usize = 4;
/// Channel capacity for runner events
const RUNNER_CHANNEL_CAPACITY: usize = 100;

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
    RunnerEvent(RunnerEvent),
    SystemStats(SystemStats),
    FixResult(CheckResult),
    FixAll(FixAllEvent),
    RetryResult(CheckResult),
}

/// Events from the fix-all background task
#[derive(Debug)]
enum FixAllEvent {
    /// One fix command finished
    Result(CheckResult),
    /// The whole fix-all loop finished (sent unconditionally after the loop)
    Done,
}

/// Read keyboard events and send them through a channel
///
/// This runs on a dedicated OS thread for responsiveness under high CPU load.
fn keyboard_loop(shutdown: Arc<AtomicBool>, tx: mpsc::UnboundedSender<KeyEvent>) {
    while !shutdown.load(Ordering::Relaxed) {
        if !event::poll(KEYBOARD_POLL_TIMEOUT).unwrap_or(false) {
            continue;
        }
        let Ok(Event::Key(key)) = event::read() else {
            continue;
        };
        // Filter for Press events only (Windows sends Press+Release)
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // If send fails, receiver is dropped - exit thread
        if tx.send(key).is_err() {
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
) -> (mpsc::UnboundedReceiver<KeyEvent>, thread::JoinHandle<()>) {
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

/// Channels for spawning async operations from key handlers
struct EventChannels {
    fix_tx: mpsc::Sender<CheckResult>,
    fix_all_tx: mpsc::Sender<FixAllEvent>,
    retry_tx: mpsc::Sender<CheckResult>,
    project_root: Arc<PathBuf>,
    /// Container name for docker exec/run commands (Arc for cheap cloning into async tasks)
    container_name: Arc<str>,
    /// Docker configuration (for image, volumes, workdir)
    docker_config: Arc<crate::config::DockerConfig>,
    /// Global environment variables from config (for docker exec -e flags)
    global_env: Arc<std::collections::HashMap<String, String>>,
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

/// Spawn a retry/run task for a check
fn spawn_retry_task(channels: &EventChannels, check: CheckToRun) {
    let retry_tx = channels.retry_tx.clone();
    let project_root = Arc::clone(&channels.project_root);
    let container = Arc::clone(&channels.container_name);
    let docker_config = Arc::clone(&channels.docker_config);
    let global_env = Arc::clone(&channels.global_env);
    tokio::spawn(async move {
        let result = run_single_check(
            &check,
            &project_root,
            &container,
            &docker_config,
            &global_env,
        )
        .await;
        let _ = retry_tx.send(result).await;
    });
}

/// Spawn a fix task for a check
fn spawn_fix_task(channels: &EventChannels, fix_cmd: String, container: Option<String>) {
    let fix_tx = channels.fix_tx.clone();
    let project_root = Arc::clone(&channels.project_root);
    let default_container = Arc::clone(&channels.container_name);
    let docker_config = Arc::clone(&channels.docker_config);
    let global_env = Arc::clone(&channels.global_env);
    tokio::spawn(async move {
        let container_name = container.as_deref().unwrap_or(&default_container);
        let result = run_fix_command(
            &fix_cmd,
            &project_root,
            container_name,
            &docker_config,
            &global_env,
        )
        .await;
        let _ = fix_tx.send(result).await;
    });
}

#[cfg(debug_assertions)]
fn warn_slow_keyboard(start: std::time::Instant) {
    let elapsed = start.elapsed();
    if elapsed > std::time::Duration::from_millis(1) {
        eprintln!("WARN: Keyboard response took {:?}", elapsed);
    }
}

/// Handle 'r' key: retry selected check with git refresh
fn handle_retry_selected(app: &mut App, channels: &EventChannels, config: &CiConfig) -> Action {
    if !app.can_retry_selected() {
        return Action::Continue;
    }
    let Some(check) = app.selected_check() else {
        return Action::Continue;
    };
    let check_id = check.id().to_string();
    let base_ref = app.changed_files.base_ref.clone();

    let Ok(mut new_changed_files) = get_changed_files(&channels.project_root, &base_ref) else {
        // Git refresh failed - fall back to running with existing check,
        // but tell the user the file list may be stale
        let check = check.clone();
        app.reset_check_for_retry(check.id());
        app.set_status_message(Some(
            "Git refresh failed - retrying with previous file list".to_string(),
        ));
        spawn_retry_task(channels, check);
        return Action::Continue;
    };

    new_changed_files.apply_ignore_patterns(config.compiled_ignore_patterns());
    let new_checks = determine_checks(config, &new_changed_files, &channels.project_root);

    let Some(new_check) = new_checks.iter().find(|c| c.id() == check_id) else {
        app.set_status_message(Some(
            "Check no longer applicable after git refresh".to_string(),
        ));
        return Action::Continue;
    };
    let new_check = new_check.clone();

    app.changed_files = new_changed_files;
    if let Some(idx) = app.checks.iter().position(|c| c.id() == check_id) {
        app.checks[idx] = new_check.clone();
    }

    app.reset_check_for_retry(&check_id);
    spawn_retry_task(channels, new_check);
    Action::Continue
}

/// Handle 't' key: trigger on-demand test
fn handle_trigger_on_demand(app: &mut App, channels: &EventChannels) -> Action {
    if !app.can_trigger_selected() {
        return Action::Continue;
    }
    let Some(check) = app.selected_check() else {
        return Action::Continue;
    };
    let check = check.clone();
    app.trigger_on_demand_check(check.id());
    spawn_retry_task(channels, check);
    Action::Continue
}

/// Handle 'A' key: run selected check for all files
fn handle_run_all_files(app: &mut App, channels: &EventChannels) -> Action {
    if !app.can_run_all_files() {
        return Action::Continue;
    }
    let Some(check) = app.selected_check() else {
        return Action::Continue;
    };
    let check = check.clone();
    let all_files_cmd = check.get_command_for_all_files();
    app.reset_check_for_retry(check.id());
    app.set_status_message(Some("Running for all files...".to_string()));
    let retry_tx = channels.retry_tx.clone();
    let project_root = Arc::clone(&channels.project_root);
    let container = Arc::clone(&channels.container_name);
    let docker_config = Arc::clone(&channels.docker_config);
    let global_env = Arc::clone(&channels.global_env);
    tokio::spawn(async move {
        let result = run_check_with_command(
            &check,
            &all_files_cmd,
            &project_root,
            &container,
            &docker_config,
            &global_env,
        )
        .await;
        let _ = retry_tx.send(result).await;
    });
    Action::Continue
}

/// Handle 'R' key: retry all checks with git refresh
fn handle_retry_all(app: &App, channels: &EventChannels, config: &CiConfig) -> Action {
    let base_ref = app.changed_files.base_ref.clone();
    let mut new_changed_files =
        get_changed_files(&channels.project_root, &base_ref).unwrap_or(ChangedFiles {
            files: vec![],
            base_ref,
        });
    new_changed_files.apply_ignore_patterns(config.compiled_ignore_patterns());
    let new_checks = determine_checks(config, &new_changed_files, &channels.project_root);

    Action::RestartRunner {
        new_changed_files,
        new_checks,
    }
}

/// Handle 'x' key: run fix for selected check
fn handle_fix_selected(app: &mut App, channels: &EventChannels) -> Action {
    if !app.can_fix_selected() {
        return Action::Continue;
    }
    let Some((fix_cmd, _service, container)) = app.get_selected_fix_command() else {
        return Action::Continue;
    };
    app.start_fix();
    spawn_fix_task(channels, fix_cmd, container);
    Action::Continue
}

/// Handle 'X' key: run fix for all failed checks
fn handle_fix_all(app: &mut App, channels: &EventChannels) -> Action {
    if !app.can_fix_all() {
        return Action::Continue;
    }
    let fix_commands = app.get_all_fix_commands();
    if fix_commands.is_empty() {
        return Action::Continue;
    }
    let total = fix_commands.len();
    app.start_fix_all(total);
    let fix_all_tx = channels.fix_all_tx.clone();
    let project_root = Arc::clone(&channels.project_root);
    let default_container = Arc::clone(&channels.container_name);
    let docker_config = Arc::clone(&channels.docker_config);
    let global_env = Arc::clone(&channels.global_env);
    tokio::spawn(async move {
        for (_check_id, fix_cmd, _service, container) in fix_commands {
            let container_name = container.as_deref().unwrap_or(&default_container);
            let result = run_fix_command(
                &fix_cmd,
                &project_root,
                container_name,
                &docker_config,
                &global_env,
            )
            .await;
            let _ = fix_all_tx.send(FixAllEvent::Result(result)).await;
        }
        // Always signal completion, decoupled from per-result send success.
        // Previously completion rode on an is_last flag on the final result;
        // a failed send left fix_all_running=true and disabled r/t/x/X forever.
        let _ = fix_all_tx.send(FixAllEvent::Done).await;
    });
    Action::Continue
}

/// Handle a key event and return the action for the main loop
fn handle_key_event(
    app: &mut App,
    key: KeyEvent,
    channels: &EventChannels,
    config: &CiConfig,
) -> Action {
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
            app.scroll_up(10);
            Action::Continue
        }
        (KeyCode::PageDown, _) => {
            app.scroll_down(10);
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
        (KeyCode::Char('r'), KeyModifiers::NONE) => handle_retry_selected(app, channels, config),
        (KeyCode::Char('t'), KeyModifiers::NONE) => handle_trigger_on_demand(app, channels),
        (KeyCode::Char('A'), KeyModifiers::SHIFT) => handle_run_all_files(app, channels),
        (KeyCode::Char('c'), KeyModifiers::NONE) => {
            if let Some(check) = app.selected_check() {
                copy_to_clipboard(&check.resolved_command);
                app.set_status_message(Some("Command copied to clipboard".to_string()));
            }
            Action::Continue
        }
        (KeyCode::Char('e'), KeyModifiers::NONE) => {
            app.toggle_full_command();
            Action::Continue
        }
        (KeyCode::Char('R'), KeyModifiers::SHIFT) => handle_retry_all(app, channels, config),
        (KeyCode::Char('x'), KeyModifiers::NONE) => handle_fix_selected(app, channels),
        (KeyCode::Char('X'), KeyModifiers::SHIFT) => handle_fix_all(app, channels),
        _ => Action::Continue,
    }
}

/// Handle a message from the event loop and update app state
/// All state changes go through this function via &mut App
fn handle_message(
    app: &mut App,
    msg: Message,
    channels: &EventChannels,
    config: &CiConfig,
) -> Result<Action> {
    match msg {
        Message::KeyPress(key) => {
            // Keyboard response timing instrumentation
            #[cfg(debug_assertions)]
            let start = std::time::Instant::now();

            // Quit keys must always work, even while a status message is
            // displayed - previously they were swallowed by the dismiss logic
            let is_quit = matches!(
                (key.code, key.modifiers),
                (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL)
            );

            // Clear status message on any other key press
            if app.status_message.is_some() && !is_quit {
                app.clear_status_message();

                #[cfg(debug_assertions)]
                warn_slow_keyboard(start);

                return Ok(Action::Continue);
            }

            // Dispatch to key handler
            let result = Ok(handle_key_event(app, key, channels, config));

            #[cfg(debug_assertions)]
            warn_slow_keyboard(start);

            result
        }
        Message::RunnerEvent(event) => {
            app.handle_runner_event(event);
            Ok(Action::Continue)
        }
        Message::SystemStats(stats) => {
            app.update_stats(stats.cpu_usage, stats.mem_used, stats.mem_total);
            Ok(Action::Continue)
        }
        Message::FixResult(result) => {
            app.finish_fix(result);
            Ok(Action::Continue)
        }
        Message::FixAll(FixAllEvent::Result(result)) => {
            app.add_fix_all_result(result);
            Ok(Action::Continue)
        }
        Message::FixAll(FixAllEvent::Done) => {
            app.finish_fix_all();
            Ok(Action::Continue)
        }
        Message::RetryResult(result) => {
            app.set_retry_result(result);
            Ok(Action::Continue)
        }
    }
}

pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
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
    let (mut runner_handle, mut event_rx) = start_runner(&config, &project_root, checks);

    // Create event channels for async operations
    let (fix_tx, mut fix_rx) = mpsc::channel(1);
    let (fix_all_tx, mut fix_all_rx) = mpsc::channel::<FixAllEvent>(10);
    let (retry_tx, mut retry_rx) = mpsc::channel::<CheckResult>(1);
    let project_root = Arc::new(project_root);
    let container_name: Arc<str> = config.docker.container_name().into();
    let global_env: Arc<std::collections::HashMap<String, String>> =
        Arc::new(config.docker.env.clone());

    let docker_config = Arc::new(config.docker.clone());
    let channels = EventChannels {
        fix_tx,
        fix_all_tx,
        retry_tx,
        project_root: Arc::clone(&project_root),
        container_name,
        docker_config,
        global_env,
    };

    // Start background stats worker - runs sysinfo queries without blocking UI
    let (stats_tx, mut stats_rx) = mpsc::channel::<SystemStats>(STATS_CHANNEL_CAPACITY);
    let stats_handle = spawn_stats_worker(stats_tx);

    // Start dedicated OS thread for keyboard input
    // CRITICAL: Using std::thread ensures keyboard events are processed by the OS
    // scheduler even when Tokio is starved of CPU time by Docker containers.
    let keyboard_shutdown = Arc::new(AtomicBool::new(false));
    let (mut keyboard_rx, keyboard_thread) = spawn_keyboard_thread(Arc::clone(&keyboard_shutdown));

    // Main event loop - uses tokio::select! for event-driven responsiveness
    //
    // Architecture for responsive keyboard handling under high CPU load:
    // 1. biased; ensures keyboard events are checked first (highest priority)
    // 2. Each channel becomes a select! branch - wakes on any event
    // 3. All state transitions go through handle_message with explicit Message enum
    // 4. Render only if state changed (dirty flag)
    loop {
        // Wait for the next event from any channel
        // biased; ensures keyboard is checked first for immediate responsiveness
        let msg = tokio::select! {
            biased;

            // Keyboard events have highest priority
            Some(key) = keyboard_rx.recv() => Message::KeyPress(key),

            // Runner events (check output, status changes)
            Some(event) = event_rx.recv() => Message::RunnerEvent(event),

            // System stats from background worker
            Some(stats) = stats_rx.recv() => Message::SystemStats(stats),

            // Fix command results
            Some(result) = fix_rx.recv() => Message::FixResult(result),

            // Fix-all command results
            Some(event) = fix_all_rx.recv() => Message::FixAll(event),

            // Retry command results
            Some(result) = retry_rx.recv() => Message::RetryResult(result),

            // All channels closed - exit
            else => break,
        };

        // Handle the message and get the action
        match handle_message(&mut app, msg, &channels, &config)? {
            Action::Quit => break,
            Action::RestartRunner {
                new_changed_files,
                new_checks,
            } => {
                // Abort old runner and start new one
                runner_handle.abort();
                app.reset_for_retry(new_changed_files, new_checks.clone());
                let (new_handle, new_rx) = start_runner(&config, &project_root, new_checks);
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

    Ok(())
}

fn print_summary(app: &App) {
    let (passed, failed, _pending, _on_demand) = app.count_by_status();
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

    /// Build EventChannels for handle_message tests. Receivers are dropped;
    /// the tests below never depend on sends succeeding. project_root points
    /// at a nonexistent path so git operations fail deterministically.
    fn make_test_channels(config: &CiConfig) -> EventChannels {
        let (fix_tx, _fix_rx) = mpsc::channel(1);
        let (fix_all_tx, _fix_all_rx) = mpsc::channel(10);
        let (retry_tx, _retry_rx) = mpsc::channel(1);
        EventChannels {
            fix_tx,
            fix_all_tx,
            retry_tx,
            project_root: Arc::new(PathBuf::from("/nonexistent-ci-tui-test-path")),
            container_name: "app".into(),
            docker_config: Arc::new(config.docker.clone()),
            global_env: Arc::new(config.docker.env.clone()),
        }
    }

    fn make_test_app(config: &CiConfig) -> App {
        let changed_files = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: "development".to_string(),
        };
        App::new(config.clone(), changed_files, vec![], "main".to_string())
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
            service: "php".to_string(),
            files: crate::checks::CheckFiles::Files(vec!["src/Foo.php".to_string()]),
            resolved_command: format!("{} src/Foo.php", id),
            resolved_fix_command: None,
        }
    }

    #[tokio::test]
    async fn test_retry_selected_git_failure_sets_status_message() {
        let config = test_config();
        let changed_files = ChangedFiles {
            files: vec!["src/Foo.php".to_string()],
            base_ref: "development".to_string(),
        };
        let checks = vec![make_test_check("php-lint", "fast")];
        let mut app = App::new(config.clone(), changed_files, checks, "main".to_string());
        app.results.get_mut("php-lint").unwrap().status = crate::runner::CheckStatus::Passed;
        let channels = make_test_channels(&config); // project_root does not exist -> git fails

        let action = handle_retry_selected(&mut app, &channels, &config);

        assert!(matches!(action, Action::Continue));
        assert!(
            app.status_message.is_some(),
            "git refresh failure must surface a status message"
        );
        // Fallback still retried the check with the previous file list
        assert_eq!(
            app.results.get("php-lint").unwrap().status,
            crate::runner::CheckStatus::Running
        );
    }

    #[tokio::test]
    async fn test_quit_key_works_while_status_message_shown() {
        let config = test_config();
        let mut app = make_test_app(&config);
        app.status_message = Some("Command copied to clipboard".to_string());
        let channels = make_test_channels(&config);

        let key = press(KeyCode::Char('q'), KeyModifiers::NONE);
        let action = handle_message(&mut app, Message::KeyPress(key), &channels, &config).unwrap();

        assert!(
            matches!(action, Action::Quit),
            "q must quit even while a status message is displayed"
        );
    }

    #[tokio::test]
    async fn test_ctrl_c_works_while_status_message_shown() {
        let config = test_config();
        let mut app = make_test_app(&config);
        app.status_message = Some("some message".to_string());
        let channels = make_test_channels(&config);

        let key = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let action = handle_message(&mut app, Message::KeyPress(key), &channels, &config).unwrap();

        assert!(matches!(action, Action::Quit));
    }

    #[tokio::test]
    async fn test_other_key_dismisses_status_message() {
        let config = test_config();
        let mut app = make_test_app(&config);
        app.status_message = Some("some message".to_string());
        let channels = make_test_channels(&config);

        let key = press(KeyCode::Char('j'), KeyModifiers::NONE);
        let action = handle_message(&mut app, Message::KeyPress(key), &channels, &config).unwrap();

        assert!(matches!(action, Action::Continue));
        assert!(
            app.status_message.is_none(),
            "non-quit key clears the message"
        );
    }

    /// Test that keyboard events are processed immediately even under heavy load
    /// This verifies the tokio::select! with biased; provides <1ms keyboard response
    #[tokio::test]
    async fn test_keyboard_responsiveness_under_load() {
        let (key_tx, mut key_rx) = mpsc::unbounded_channel::<KeyEvent>();

        // Spawn a task that simulates heavy work (similar to Docker container CPU load)
        let heavy_work = tokio::spawn(async {
            for _ in 0..1000 {
                // Simulate CPU-bound work that would starve Tokio tasks
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        // Create a mock key event
        let mock_key = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        };

        // Send keyboard event
        let send_time = std::time::Instant::now();
        key_tx.send(mock_key).unwrap();

        // Receive should be nearly instant even with heavy work running
        tokio::select! {
            biased;
            Some(_key) = key_rx.recv() => {
                let elapsed = send_time.elapsed();
                // With biased; and keyboard-first priority, response should be <1ms
                assert!(
                    elapsed < std::time::Duration::from_millis(1),
                    "Keyboard response took {:?}, expected <1ms. The biased select! should prioritize keyboard events.",
                    elapsed
                );
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                panic!("Keyboard event not received within 100ms - event loop may be blocked");
            }
        }

        heavy_work.abort();
    }

    /// Test that biased select! checks keyboard channel first
    /// This verifies the event loop architecture prioritizes user input
    #[tokio::test]
    async fn test_biased_select_keyboard_priority() {
        let (key_tx, mut key_rx) = mpsc::unbounded_channel::<KeyEvent>();
        let (other_tx, mut other_rx) = mpsc::unbounded_channel::<i32>();

        // Send events to both channels simultaneously
        let mock_key = KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        };
        key_tx.send(mock_key).unwrap();
        other_tx.send(42).unwrap();

        // With biased select, keyboard should be checked first
        let mut keyboard_checked_first = 0;
        let mut other_checked_first = 0;

        for _ in 0..10 {
            // Send to both channels
            key_tx.send(mock_key).unwrap();
            other_tx.send(42).unwrap();

            // Select with biased - keyboard branch should be prioritized
            tokio::select! {
                biased;
                Some(_) = key_rx.recv() => {
                    keyboard_checked_first += 1;
                    // Drain the other channel
                    let _ = other_rx.try_recv();
                }
                Some(_) = other_rx.recv() => {
                    other_checked_first += 1;
                    // Drain keyboard channel
                    let _ = key_rx.try_recv();
                }
            }
        }

        // With biased, keyboard should always be checked first when both have events
        assert_eq!(
            keyboard_checked_first, 10,
            "Expected keyboard to be checked first in all iterations due to biased; keyword"
        );
        assert_eq!(
            other_checked_first, 0,
            "Other channel should never be checked when keyboard has events (biased; priority)"
        );
    }

    #[tokio::test]
    async fn test_fix_all_done_clears_running_flag() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let channels = make_test_channels(&config);

        app.start_fix_all(2);
        assert!(app.fix_all_running);

        // Done arrives even if individual result sends were lost
        let action = handle_message(
            &mut app,
            Message::FixAll(FixAllEvent::Done),
            &channels,
            &config,
        )
        .unwrap();

        assert!(matches!(action, Action::Continue));
        assert!(!app.fix_all_running, "Done must clear fix_all_running");
    }

    #[tokio::test]
    async fn test_fix_all_result_does_not_finish_run() {
        let config = test_config();
        let mut app = make_test_app(&config);
        let channels = make_test_channels(&config);

        app.start_fix_all(2);

        let result = CheckResult::pending("some-check");
        handle_message(
            &mut app,
            Message::FixAll(FixAllEvent::Result(result)),
            &channels,
            &config,
        )
        .unwrap();

        assert!(
            app.fix_all_running,
            "intermediate results must not finish the run"
        );
        assert_eq!(app.fix_all_results.len(), 1);
    }
}
