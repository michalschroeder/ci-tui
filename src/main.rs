use anyhow::Result;
use ci_tui::cli::{missing_config_error, run_config_path, Cli, Command};
use ci_tui::runner::{ExecTarget, RealCommandExecutor};
use ci_tui::{checks, commands, config, fix, git, list, preflight, simple, ui};
use std::io::IsTerminal;

fn git_detect_changes_or_exit(
    project_root: &std::path::Path,
    git_config: &ci_tui::config::GitConfig,
    base_override: Option<&str>,
) -> git::ChangedFiles {
    let result = match base_override {
        Some(base_ref) => git::get_changed_files(project_root, base_ref),
        None => git::detect_changes(project_root, git_config),
    };
    match result {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e:#}");
            match base_override {
                Some(base_ref) => eprintln!(
                    "\nCheck that `{base_ref}` exists locally (tags / remote branches may need `git fetch`)."
                ),
                None => eprintln!(
                    "\nNo base ref could be resolved against the current repository. \
                    If this is intentional, bypass git with --files <paths...>."
                ),
            }
            std::process::exit(1);
        }
    }
}

/// `--files` entry as a changed-file path. With `repo_root` (local mode only),
/// cwd-relative paths are rewritten repo-relative (matching git output);
/// otherwise (docker mode) they are kept as given.
fn cli_file_path(
    cwd: &std::path::Path,
    repo_root: Option<&std::path::Path>,
    path: &std::path::Path,
) -> String {
    match repo_root {
        Some(root) => git::to_repo_relative(cwd, root, path),
        None => path.to_string_lossy().into_owned(),
    }
}

/// Local-mode execution root: the git repo root, or `cwd` (with a warning)
/// when it cannot be resolved (e.g. `--files` outside a repo).
fn local_exec_root(cwd: &std::path::Path) -> std::path::PathBuf {
    git::repo_root(cwd).unwrap_or_else(|e| {
        eprintln!("Warning: {e:#}; running checks from the current directory");
        cwd.to_path_buf()
    })
}

/// Run `init` / `validate` and print the outcome.
fn run_subcommand(command: Command, config: Option<std::path::PathBuf>) -> Result<()> {
    let path = command.config_path(config);
    match command {
        Command::Init { .. } => {
            commands::init(&path)?;
            println!("Wrote {}", path.display());
            println!(
                "Edit the checks section, then run: ci-tui --config {}",
                path.display()
            );
        }
        Command::Validate { .. } => {
            commands::validate(&path)?;
            println!("OK: {} is valid", path.display());
        }
    }
    Ok(())
}

/// Exit code for a Ctrl-C interrupted run (128 + SIGINT)
const INTERRUPTED_EXIT_CODE: i32 = 130;

fn main() -> Result<()> {
    // The runtime is dropped at the end of this statement, before exiting:
    // that drops every still-running command future, whose guards kill its
    // process group / `docker run` container (quit, Ctrl-C).
    let code = tokio::runtime::Runtime::new()?.block_on(run())?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// Run `fut`, or stop at Ctrl-C. Commands run in their own process group,
/// so the terminal's SIGINT no longer reaches them: dropping `fut` (and then
/// the runtime) kills them instead.
async fn interruptible(fut: impl std::future::Future<Output = Result<()>>) -> Result<i32> {
    tokio::select! {
        result = fut => result.map(|()| 0),
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nInterrupted");
            Ok(INTERRUPTED_EXIT_CODE)
        }
    }
}

/// Process exit code of the whole run
async fn run() -> Result<i32> {
    // Install color-eyre for better panic handling (errors are ignored if it fails)
    let _ = color_eyre::install();

    let cli = Cli::parse_checked();

    if let Some(command) = cli.command {
        return run_subcommand(command, cli.config).map(|()| 0);
    }

    // Use current working directory as project root
    let project_root = std::env::current_dir()?;

    // Enforced here, not via clap `required`: clap forbids required global args.
    let Some(config_path) = run_config_path(cli.config, &project_root) else {
        missing_config_error().exit();
    };

    // Auto-detect TUI mode: use simple mode if stdout is not a terminal
    let simple_mode = cli.simple || !std::io::stdout().is_terminal();

    // Load configuration
    let config = config::load_config(&config_path)?;

    // Local mode runs commands from the repo root so repo-relative {files}
    // resolve from any subdirectory. Docker mode keeps cwd (compose project dir).
    let repo_root = match config.runner {
        ExecTarget::Local(_) => Some(local_exec_root(&project_root)),
        ExecTarget::Docker(_) => None,
    };
    let exec_root = repo_root.clone().unwrap_or_else(|| project_root.clone());

    // Get changed files: from --files arg or git detection. In local mode
    // `--files` (cwd-relative) are rewritten repo-relative to match git paths.
    let mut changed_files = if cli.files.is_empty() {
        git_detect_changes_or_exit(&project_root, &config.git, cli.base.as_deref())
    } else {
        git::ChangedFiles {
            files: cli
                .files
                .iter()
                .map(|p| cli_file_path(&project_root, repo_root.as_deref(), p))
                .collect(),
            base_ref: git::CLI_FILES_BASE_REF.to_string(),
        }
    };
    changed_files.apply_ignore_patterns(config.compiled_ignore_patterns());

    // Dry run: explain check selection, execute nothing
    if cli.list {
        let explained = list::explain_checks(&config, &changed_files, &exec_root);
        print!(
            "{}",
            list::render(&config, &changed_files, cli.base.as_deref(), &explained)
        );
        return Ok(0);
    }

    // Determine which checks to run (fix mode reuses it only for the probe)
    let checks_to_run = checks::determine_checks(&config, &changed_files, &exec_root);

    // Docker: warn (non-fatal) when changed files won't resolve in the
    // containers the checks use. Console modes print now; the TUI shows them
    // in-app (alternate screen).
    let docker_warnings = preflight::docker_warnings(
        &config.runner,
        &checks_to_run,
        &changed_files.files,
        &exec_root,
        &RealCommandExecutor,
    )
    .await;
    if cli.fix || simple_mode {
        for warning in &docker_warnings {
            eprintln!("Warning: {warning}");
        }
    }

    // Run fix mode if requested
    if cli.fix {
        return interruptible(fix::run(config, changed_files, exec_root)).await;
    }

    if simple_mode {
        // Run in simple console mode
        interruptible(simple::run(config, changed_files, checks_to_run, exec_root)).await
    } else {
        // Run the TUI
        ui::run(
            config,
            changed_files,
            checks_to_run,
            project_root,
            exec_root,
            (!docker_warnings.is_empty()).then(|| docker_warnings.join("; ")),
        )
        .await
    }
}
