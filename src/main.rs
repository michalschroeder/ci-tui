use anyhow::Result;
use ci_tui::cli::{filter_error, missing_config_error, run_config, Cli, Command, RunConfig};
use ci_tui::exit;
use ci_tui::runner::ExecTarget;
use ci_tui::{
    cache, checks, commands, config, filter, fix, git, list, preflight, report, schema, simple, ui,
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

/// Changed files minus `ignore_patterns`: from --files, the git index
/// (--staged) or git detection in `project_root`. `--files` (cwd-relative) are
/// rewritten relative to `exec_root` (local mode: the repo root, matching git
/// paths); paths outside it are kept as given.
fn changed_files(
    cli: &Cli,
    config: &config::CiConfig,
    cwd: &std::path::Path,
    project_root: &std::path::Path,
    exec_root: &std::path::Path,
) -> git::ChangedFiles {
    let mut changed_files = if cli.files.is_empty() {
        git_detect_changes_or_exit(project_root, &config.git, cli.base.as_deref(), cli.staged)
    } else {
        git::ChangedFiles {
            files: cli
                .files
                .iter()
                .map(|p| git::to_relative(cwd, exec_root, p))
                .collect(),
            base_ref: git::CLI_FILES_BASE_REF.to_string(),
        }
    };
    changed_files.apply_ignore_patterns(config.compiled_ignore_patterns());
    changed_files
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
    let Some(path) = command.config_path(config, &std::env::current_dir()?) else {
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
async fn interruptible(fut: impl std::future::Future<Output = Result<i32>>) -> Result<i32> {
    tokio::select! {
        result = fut => result,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nInterrupted");
            Ok(exit::INTERRUPTED)
        }
    }
}

/// Exit code 0 once `fut` succeeds
async fn succeeded(fut: impl std::future::Future<Output = Result<()>>) -> Result<i32> {
    fut.await.map(|()| exit::SUCCESS)
}

/// Set the console color switches and install color-eyre for better panic
/// reports (errors are ignored if it fails), themeless with color off
fn init_color(colors: ci_tui::color::ConsoleColor) {
    ci_tui::color::set_enabled(colors);
    let hook = if colors.stdout {
        color_eyre::config::HookBuilder::default()
    } else {
        color_eyre::config::HookBuilder::blank()
    };
    let _ = hook.install();
}

/// Process exit code of the whole run
async fn run() -> Result<i32> {
    let cli = Cli::parse_checked();
    init_color(cli.color());

    if let Some(command) = cli.command {
        return run_subcommand(command, cli.config).map(|()| exit::SUCCESS);
    }
    let report = report::ReportOptions::new(cli.format, cli.output.clone())?;

    // Project root: the discovered config's dir (as if launched there), cwd
    // with --config. Enforced here, not via clap `required`: clap forbids
    // required global args.
    let cwd = std::env::current_dir()?;
    let Some(RunConfig {
        path: config_path,
        root: project_root,
    }) = run_config(cli.config.clone(), &cwd)
    else {
        missing_config_error().exit();
    };

    // Simple mode if stdout is not a terminal; a json / junit report implies it
    let simple_mode = cli.simple || report.writes_report() || !std::io::stdout().is_terminal();

    // Load configuration (plus `ci-tui.local.yaml` overrides)
    let mut config = config::load_config(&config_path)?;

    // --only / --group: narrow the config so every mode sees the same subset.
    // Unknown ids are a usage error (exit 2), like other bad flag values.
    if let Err(e) = filter::apply(&mut config, &cli.only, &cli.groups) {
        filter_error(e).exit();
    }
    // --jobs / --notify override the config's (and local overrides') values
    cli.override_config(&mut config);

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
    // resolve from any subdirectory. Docker mode keeps the project root
    // (compose project dir).
    let exec_root = match config.runner {
        ExecTarget::Local(_) => local_exec_root(&project_root),
        ExecTarget::Docker(_) => project_root.clone(),
    };
    let changed_files = changed_files(&cli, &config, &cwd, &project_root, &exec_root);

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

    // Run fix mode if requested; passing fixes are verified against the
    // selected checks
    if cli.fix {
        let verify = (!cli.no_verify).then_some(checks_to_run);
        let fix = succeeded(fix::run(config, changed_files, exec_root, verify));
        return interruptible(fix).await;
    }

    if simple_mode {
        // Run in simple console mode
        interruptible(simple::run(
            config,
            changed_files,
            checks_to_run,
            exec_root,
            cache,
            report,
        ))
        .await
    } else {
        // Run the TUI
        let options = ui::TuiOptions::new(&cli, &config, filter_notice, cache);
        ui::run(
            config,
            changed_files,
            checks_to_run,
            project_root,
            exec_root,
            warnings,
            options,
        )
        .await
    }
}
