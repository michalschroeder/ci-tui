mod app;
mod dashboard;

use crate::checks::{determine_checks, CheckToRun};
use crate::config::CiConfig;
use crate::git::{current_branch, get_changed_files, ChangedFiles};
use crate::runner::{run_check_with_command, run_fix_command, run_single_check, CheckResult, CheckRunner, RunnerEvent};
use anyhow::Result;
use app::App;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io::{self, stdout};
use std::panic;
use std::path::PathBuf;
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
    FixAllResult(CheckResult, bool),  // (result, is_last)
    RetryResult(CheckResult),
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
    // Use tokio mpsc unbounded channel for async/await compatibility
    let (tx, rx) = mpsc::unbounded_channel();

    let handle = thread::Builder::new()
        .name("keyboard-input".to_string())
        .spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                if event::poll(KEYBOARD_POLL_TIMEOUT).unwrap_or(false) {
                    if let Ok(Event::Key(key)) = event::read() {
                        // Filter for Press events only (Windows sends Press+Release)
                        if key.kind == KeyEventKind::Press {
                            // If send fails, receiver is dropped - exit thread
                            // Use blocking_send from std::thread context
                            if tx.send(key).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        })
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

            let cpu_usage: f32 = system.cpus().iter()
                .map(|cpu| cpu.cpu_usage())
                .sum::<f32>() / system.cpus().len().max(1) as f32;

            let stats = SystemStats {
                cpu_usage,
                mem_used: system.used_memory(),
                mem_total: system.total_memory(),
            };

            // Non-blocking send - if channel is full, skip this update
            // This prevents backpressure from affecting the stats worker
            if tx.try_send(stats).is_err() {
                // Channel full or closed, continue anyway
            }
        }
    })
}

/// Actions that can result from key handling
enum KeyAction {
    /// Continue the main loop
    None,
    /// Exit the application
    Quit,
    /// Retry all checks with refreshed git state
    RetryAll {
        new_changed_files: ChangedFiles,
        new_checks: Vec<CheckToRun>,
    },
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
    fix_all_tx: mpsc::Sender<(CheckResult, bool)>,
    retry_tx: mpsc::Sender<CheckResult>,
    project_root: Arc<PathBuf>,
    /// Docker project directory path (Arc for cheap cloning into async tasks)
    docker_project_dir: Arc<str>,
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
    use base64::{Engine as _, engine::general_purpose::STANDARD};
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
    project_root: &PathBuf,
    checks: Vec<CheckToRun>,
) -> (JoinHandle<Result<()>>, mpsc::Receiver<RunnerEvent>) {
    let (event_tx, event_rx) = mpsc::channel::<RunnerEvent>(RUNNER_CHANNEL_CAPACITY);
    let runner = CheckRunner::new(config.clone(), project_root);

    let handle = tokio::spawn(async move {
        runner.run_checks(checks, event_tx).await
    });

    (handle, event_rx)
}

/// Handle a key event and return the action for the main loop
fn handle_key_event(
    app: &mut App,
    key: KeyEvent,
    channels: &EventChannels,
    config: &CiConfig,
) -> KeyAction {
    match (key.code, key.modifiers) {
        // Quit
        (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            KeyAction::Quit
        }
        // Navigation
        (KeyCode::Up | KeyCode::Char('k'), _) => {
            app.previous_check();
            KeyAction::None
        }
        (KeyCode::Down | KeyCode::Char('j'), _) => {
            app.next_check();
            KeyAction::None
        }
        (KeyCode::PageUp, _) => {
            app.scroll_up(10);
            KeyAction::None
        }
        (KeyCode::PageDown, _) => {
            app.scroll_down(10);
            KeyAction::None
        }
        // Filter by status
        (KeyCode::Char('f'), _) => {
            app.toggle_failed_filter();
            KeyAction::None
        }
        (KeyCode::Char('a'), _) => {
            app.show_all();
            KeyAction::None
        }
        // Retry selected check only
        (KeyCode::Char('r'), KeyModifiers::NONE) => {
            if app.can_retry_selected() {
                if let Some(check) = app.selected_check() {
                    let check = check.clone();
                    app.reset_check_for_retry(check.id());
                    let retry_tx = channels.retry_tx.clone();
                    let project_root = Arc::clone(&channels.project_root);
                    let docker_dir = Arc::clone(&channels.docker_project_dir);
                    let global_env = Arc::clone(&channels.global_env);
                    tokio::spawn(async move {
                        let result = run_single_check(&check, &project_root, &docker_dir, &global_env).await;
                        let _ = retry_tx.send(result).await;
                    });
                }
            }
            KeyAction::None
        }
        // Trigger on-demand test (functional/integration tests)
        (KeyCode::Char('t'), KeyModifiers::NONE) => {
            if app.can_trigger_selected() {
                if let Some(check) = app.selected_check() {
                    let check = check.clone();
                    app.trigger_on_demand_check(check.id());
                    let retry_tx = channels.retry_tx.clone();
                    let project_root = Arc::clone(&channels.project_root);
                    let docker_dir = Arc::clone(&channels.docker_project_dir);
                    let global_env = Arc::clone(&channels.global_env);
                    tokio::spawn(async move {
                        let result = run_single_check(&check, &project_root, &docker_dir, &global_env).await;
                        let _ = retry_tx.send(result).await;
                    });
                }
            }
            KeyAction::None
        }
        // Run selected check for ALL files (no filtering)
        (KeyCode::Char('A'), KeyModifiers::SHIFT) => {
            if app.can_run_all_files() {
                if let Some(check) = app.selected_check() {
                    let check = check.clone();
                    let all_files_cmd = check.get_command_for_all_files();
                    app.reset_check_for_retry(check.id());
                    app.status_message = Some("Running for all files...".to_string());
                    let retry_tx = channels.retry_tx.clone();
                    let project_root = Arc::clone(&channels.project_root);
                    let docker_dir = Arc::clone(&channels.docker_project_dir);
                    let global_env = Arc::clone(&channels.global_env);
                    tokio::spawn(async move {
                        let result = run_check_with_command(&check, &all_files_cmd, &project_root, &docker_dir, &global_env).await;
                        let _ = retry_tx.send(result).await;
                    });
                }
            }
            KeyAction::None
        }
        // Copy command to clipboard (OSC 52)
        (KeyCode::Char('c'), KeyModifiers::NONE) => {
            if let Some(check) = app.selected_check() {
                let command = &check.resolved_command;
                copy_to_clipboard(command);
                app.status_message = Some("Command copied to clipboard".to_string());
            }
            KeyAction::None
        }
        // Toggle full command display
        (KeyCode::Char('e'), KeyModifiers::NONE) => {
            app.toggle_full_command();
            KeyAction::None
        }
        // Retry ALL - refresh git and rerun all checks
        (KeyCode::Char('R'), KeyModifiers::SHIFT) => {
            // Refresh git changes and apply ignore patterns
            let base_ref = app.changed_files.base_ref.clone();
            let mut new_changed_files = get_changed_files(&channels.project_root, &base_ref)
                .unwrap_or(ChangedFiles {
                    files: vec![],
                    base_ref,
                });
            new_changed_files.apply_ignore_patterns(&config.ignore_patterns);

            // Re-determine checks
            let new_checks = determine_checks(config, &new_changed_files, &channels.project_root);

            KeyAction::RetryAll {
                new_changed_files,
                new_checks,
            }
        }
        // Run fix for selected check
        (KeyCode::Char('x'), KeyModifiers::NONE) => {
            if app.can_fix_selected() {
                if let Some((fix_cmd, service)) = app.get_selected_fix_command() {
                    app.start_fix();
                    let fix_tx = channels.fix_tx.clone();
                    let project_root = Arc::clone(&channels.project_root);
                    let docker_dir = Arc::clone(&channels.docker_project_dir);
                    let global_env = Arc::clone(&channels.global_env);
                    tokio::spawn(async move {
                        let result = run_fix_command(&fix_cmd, &project_root, &docker_dir, &service, &global_env).await;
                        let _ = fix_tx.send(result).await;
                    });
                }
            }
            KeyAction::None
        }
        // Run fix for ALL failed checks
        (KeyCode::Char('X'), KeyModifiers::SHIFT) => {
            if app.can_fix_all() {
                let fix_commands = app.get_all_fix_commands();
                if !fix_commands.is_empty() {
                    let total = fix_commands.len();
                    app.start_fix_all(total);
                    let fix_all_tx = channels.fix_all_tx.clone();
                    let project_root = Arc::clone(&channels.project_root);
                    let docker_dir = Arc::clone(&channels.docker_project_dir);
                    let global_env = Arc::clone(&channels.global_env);
                    tokio::spawn(async move {
                        for (i, (_check_id, fix_cmd, service)) in fix_commands.into_iter().enumerate() {
                            let result = run_fix_command(&fix_cmd, &project_root, &docker_dir, &service, &global_env).await;
                            let is_last = i == total - 1;
                            let _ = fix_all_tx.send((result, is_last)).await;
                        }
                    });
                }
            }
            KeyAction::None
        }
        _ => KeyAction::None,
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
            // Clear status message on any key press
            if app.status_message.is_some() {
                app.status_message = None;
                app.needs_redraw = true;
                return Ok(Action::Continue);
            }

            // Dispatch to key handler
            match handle_key_event(app, key, channels, config) {
                KeyAction::Quit => Ok(Action::Quit),
                KeyAction::RetryAll { new_changed_files, new_checks } => {
                    Ok(Action::RestartRunner { new_changed_files, new_checks })
                }
                KeyAction::None => {
                    app.needs_redraw = true;
                    Ok(Action::Continue)
                }
            }
        }
        Message::RunnerEvent(event) => {
            app.handle_runner_event(event);
            app.needs_redraw = true;
            Ok(Action::Continue)
        }
        Message::SystemStats(stats) => {
            app.update_stats(stats.cpu_usage, stats.mem_used, stats.mem_total);
            app.needs_redraw = true;
            Ok(Action::Continue)
        }
        Message::FixResult(result) => {
            app.finish_fix(result);
            app.needs_redraw = true;
            Ok(Action::Continue)
        }
        Message::FixAllResult(result, is_last) => {
            app.add_fix_all_result(result);
            if is_last {
                app.finish_fix_all();
            }
            app.needs_redraw = true;
            Ok(Action::Continue)
        }
        Message::RetryResult(result) => {
            app.results.insert(result.check_id.clone(), result);
            app.needs_redraw = true;
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
    let project_root_str = project_root.to_string_lossy().to_string();

    // Create app state
    let mut app = App::new(config.clone(), changed_files, checks.clone(), branch_name, project_root_str);

    // Start the runner in background
    let (mut runner_handle, mut event_rx) = start_runner(&config, &project_root, checks);

    // Create event channels for async operations
    let (fix_tx, mut fix_rx) = mpsc::channel(1);
    let (fix_all_tx, mut fix_all_rx) = mpsc::channel::<(CheckResult, bool)>(10);
    let (retry_tx, mut retry_rx) = mpsc::channel::<CheckResult>(1);
    let project_root = Arc::new(project_root);
    let docker_project_dir: Arc<str> = config.docker.project_dir.clone().into();
    let global_env: Arc<std::collections::HashMap<String, String>> = Arc::new(config.docker.env.clone());

    let channels = EventChannels {
        fix_tx,
        fix_all_tx,
        retry_tx,
        project_root: Arc::clone(&project_root),
        docker_project_dir,
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
            Some((result, is_last)) = fix_all_rx.recv() => Message::FixAllResult(result, is_last),

            // Retry command results
            Some(result) = retry_rx.recv() => Message::RetryResult(result),

            // All channels closed - exit
            else => break,
        };

        // Handle the message and get the action
        match handle_message(&mut app, msg, &channels, &config)? {
            Action::Quit => break,
            Action::RestartRunner { new_changed_files, new_checks } => {
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
            terminal.draw(|f| dashboard::render(&app, f))?;
            app.needs_redraw = false;
        }
    }

    // Cleanup - signal keyboard thread to shutdown and wait for it
    keyboard_shutdown.store(true, Ordering::Relaxed);
    runner_handle.abort();
    stats_handle.abort();

    // Wait for keyboard thread to finish (with timeout to avoid hanging)
    let _ = keyboard_thread.join();

    restore_terminal();
    terminal.show_cursor()?;

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
        println!("\n\x1b[32m✓ All {} checks passed in {}\x1b[0m", total, elapsed_str);
    } else {
        println!("\n\x1b[31m✗ {} of {} checks failed in {}\x1b[0m", failed, total, elapsed_str);

        // Show failed checks
        for (id, result) in &app.results {
            if result.status == crate::runner::CheckStatus::Failed {
                println!("  - {}", id);
            }
        }
    }
}
