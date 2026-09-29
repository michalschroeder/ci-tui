use anyhow::Result;
use ci_tui::cli::{filter_error, missing_config_error, run_config_path, Cli, Command};
use ci_tui::exit;
use ci_tui::runner::ExecTarget;
use ci_tui::{
    cache, checks, commands, config, filter, fix, git, list, preflight, schema, simple, ui,
};
use std::io::IsTerminal;

fn git_detect_changes_or_exit(
    project_root: &std::path::Path,
    git_config: &ci_tui::config::GitConfig,
    base_override: Option<&str>,
    staged: bool,
) -> git::ChangedFiles {
    let (result, hint) = match (staged, base_override) {
        (true, _) => (
            git::get_staged_files(project_root),
            "--staged reads the git index; see the git error above.".to_string(),
        ),
        (false, Some(base_ref)) => (
            git::get_changed_files(project_root, base_ref),
            format!("Check that `{base_ref}` exists locally (tags / remote branches may need `git fetch`)."),
        ),
        (false, None) => (
            git::detect_changes(project_root, git_config),
            "No base ref could be resolved against the current repository. \
            If this is intentional, bypass git with --files <paths...>."
                .to_string(),
        ),
    };
    result.unwrap_or_else(|e| {
        eprintln!("Error: {e:#}\n\n{hint}");
        std::process::exit(exit::ENV_ERROR);
    })
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

/// Result cache for a run (not fix mode). Keys read the changed files where
/// their paths resolve: the repo root for git paths, `exec_root` for
/// `--files`. Disabled outside a git repo.
fn open_cache(
    cwd: &std::path::Path,
    exec_root: &std::path::Path,
    changed_files: &git::ChangedFiles,
    read: bool,
) -> cache::ResultCache {
    let root = match changed_files.is_cli_files() {
        true => Ok(exec_root.to_path_buf()),
        false => git::repo_root(cwd),
    };
    root.map_or_else(
        |_| cache::ResultCache::disabled(),
        |root| cache::ResultCache::open(cwd, root, read),
    )
}

/// Print the config JSON Schema; a closed pipe (`ci-tui schema | head`) is not an error.
fn print_schema() -> Result<()> {
    use std::io::Write;
    match std::io::stdout()
        .lock()
        .write_all(schema::generate().as_bytes())
    {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e.into()),
        _ => Ok(()),
    }
}

/// Run `init` / `validate` / `schema` and print the outcome.
fn run_subcommand(command: Command, config: Option<std::path::PathBuf>) -> Result<()> {
    let Some(path) = command.config_path(config) else {
        return print_schema();
    };
    if let Command::Init { .. } = command {
        commands::init(&path)?;
        println!("Wrote {}", path.display());
        println!(
            "Edit the checks section, then run: ci-tui --config {}",
            path.display()
        );
    } else {
        commands::validate(&path)?;
        println!("OK: {} is valid", path.display());
    }
    Ok(())
}

fn main() {
    // The runtime is dropped at the end of this statement, before exiting:
    // that drops every still-running command future, whose guards kill its
    // process group / `docker run` container (quit, Ctrl-C).
    let result = tokio::runtime::Runtime::new()
        .map_err(anyhow::Error::from)
        .and_then(|runtime| runtime.block_on(run()));
    let code = result.unwrap_or_else(|e| {
        eprintln!("Error: {e:?}");
        exit::code_for(&e)
    });
    if code != exit::SUCCESS {
        std::process::exit(code);
    }
}

/// Run `fut`, or stop at Ctrl-C. Commands run in their own process group,
/// so the terminal's SIGINT no longer reaches them: dropping `fut` (and then
/// the runtime) kills them instead.
async fn interruptible(fut: impl std::future::Future<Output = Result<()>>) -> Result<i32> {
    tokio::select! {
        result = fut => result.map(|()| exit::SUCCESS),
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nInterrupted");
            Ok(exit::INTERRUPTED)
        }
    }
}

/// Set the console color switch and install color-eyre for better panic
/// reports (errors are ignored if it fails), themeless with color off
fn init_color(color: bool) {
    ci_tui::color::set_enabled(color);
    let hook = if color {
        color_eyre::config::HookBuilder::default()
    } else {
        color_eyre::config::HookBuilder::blank()
    };
    let _ = hook.install();
}

/// Process exit code of the whole run
async fn run() -> Result<i32> {
    let cli = Cli::parse_checked();
    init_color(cli.color_enabled());

    if let Some(command) = cli.command {
        return run_subcommand(command, cli.config).map(|()| exit::SUCCESS);
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
    let mut config = config::load_config(&config_path)?;

    // --only / --group: narrow the config so every mode sees the same subset.
    // Unknown ids are a usage error (exit 2), like other bad flag values.
    if let Err(e) = filter::apply(&mut config, &cli.only, &cli.groups) {
        filter_error(e).exit();
    }
    // --jobs overrides the config's global parallel cap
    config.max_parallel = cli.jobs.or(config.max_parallel);
    // --notify or config `notify: true` enables the TUI end-of-run notification
    config.notify |= cli.notify;

    // Say checks were excluded: console modes on stderr (keeps --list stdout
    // clean), the TUI in its header.
    let filter_notice = filter::describe(&cli.only, &cli.groups);
    if let Some(notice) = filter_notice
        .as_ref()
        .filter(|_| cli.list || cli.fix || simple_mode)
    {
        eprintln!("{notice}");
    }

    // Local mode runs commands from the repo root so repo-relative {files}
    // resolve from any subdirectory. Docker mode keeps cwd (compose project dir).
    let repo_root = match config.runner {
        ExecTarget::Local(_) => Some(local_exec_root(&project_root)),
        ExecTarget::Docker(_) => None,
    };
    let exec_root = repo_root.clone().unwrap_or_else(|| project_root.clone());

    // Get changed files: from --files arg, the git index (--staged) or git
    // detection. In local mode `--files` (cwd-relative) are rewritten
    // repo-relative to match git paths.
    let mut changed_files = if cli.files.is_empty() {
        git_detect_changes_or_exit(&project_root, &config.git, cli.base.as_deref(), cli.staged)
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

    // Result cache: skips checks unchanged since their last pass. Fix mode
    // only runs fix commands.
    let cache = match cli.fix {
        true => cache::ResultCache::disabled(),
        false => open_cache(&project_root, &exec_root, &changed_files, !cli.no_cache),
    };

    // Dry run: explain check selection, execute nothing
    if cli.list {
        let explained = list::explain_checks_cached(&config, &changed_files, &exec_root, &cache);
        print!(
            "{}",
            list::render(&config, &changed_files, cli.base.as_deref(), &explained)
        );
        return Ok(exit::SUCCESS);
    }

    // Determine which checks to run, keyed for the cache (fix mode reuses it
    // only for the probe)
    let checks::Selected {
        checks: checks_to_run,
        mut warnings,
    } = checks::select_checks(&config, &changed_files, &exec_root, cache.key_root());

    // Docker preflight, fatal if commands would run in docker mode but it is
    // down: on-demand checks only run on request, fix mode runs its own
    // commands, cached checks only in the TUI ('r'). Non-fatal warnings
    // (failed test discovery, then Docker ones): console modes print now,
    // the TUI shows them in-app.
    let runs = |c: &&checks::CheckToRun| !(c.is_on_demand() || simple_mode && cache.is_fresh(c));
    let docker_needed = match cli.fix {
        true => fix::has_fixes(&config, &changed_files),
        false => checks_to_run.iter().any(|c| runs(&c)),
    };
    let (target, files) = (&config.runner, &changed_files.files);
    let preflight = preflight::run(target, docker_needed, &exec_root, &checks_to_run, files);
    warnings.extend(preflight.await?);
    if cli.fix || simple_mode {
        for warning in &warnings {
            eprintln!("Warning: {warning}");
        }
    }

    // Run fix mode if requested
    if cli.fix {
        return interruptible(fix::run(config, changed_files, exec_root)).await;
    }

    if simple_mode {
        // Run in simple console mode
        interruptible(simple::run(
            config,
            changed_files,
            checks_to_run,
            exec_root,
            cache,
        ))
        .await
    } else {
        // Run the TUI
        ui::run(
            config,
            changed_files,
            checks_to_run,
            project_root,
            exec_root,
            warnings,
            ui::TuiOptions::new(filter_notice, !cli.no_stats, cache, cli.exit_on_finish),
        )
        .await
    }
}
