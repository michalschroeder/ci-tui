//! Command-line interface: argument parsing and config-path resolution.

use crate::report::Format;
use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use std::ffi::OsString;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

/// CI TUI - Run CI checks for changed files
#[derive(Parser)]
#[command(
    name = "ci-tui",
    version,
    about,
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Path to config file [default: ci-tui.yaml if present; create one with `ci-tui init`]
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Run in simple console mode (no TUI)
    #[arg(short, long)]
    pub simple: bool,

    /// Run only fix commands; each passing fix re-runs its check to verify it
    /// passes now (skip with --no-verify)
    #[arg(long)]
    pub fix: bool,

    /// Do not re-run a check after its fix passed (--fix and TUI `x` / `X`)
    #[arg(long, conflicts_with = "list")]
    pub no_verify: bool,

    /// Run checks on specific files instead of git-detected changes
    #[arg(short, long, num_args = 1.., value_parser = parse_file_arg)]
    pub files: Vec<PathBuf>,

    /// Compare against this git ref instead of `git.base_branch` (exact ref, no fallback)
    #[arg(long, value_name = "REF", conflicts_with = "files", value_parser = parse_base_arg)]
    pub base: Option<String>,

    /// Check only files staged in the git index (pre-commit hook); checks still
    /// read the working tree, so unstaged edits in staged files are included.
    /// Not with --fix: fixes edit the working tree but are never re-staged
    #[arg(long, conflicts_with_all = ["files", "base", "fix"])]
    pub staged: bool,

    /// Print changed files, base ref and which checks would run and why; execute nothing
    #[arg(long, visible_alias = "dry-run", conflicts_with_all = ["simple", "fix"])]
    pub list: bool,

    /// Run only these check ids (comma-separated or repeated); triggers still apply
    #[arg(long, value_name = "ID", value_delimiter = ',')]
    pub only: Vec<String>,

    /// Run only checks in these groups (comma-separated or repeated); with --only, both must match
    #[arg(long = "group", value_name = "GROUP", value_delimiter = ',')]
    pub groups: Vec<String>,

    /// Max checks running at once per parallel group (>= 1) [default: config
    /// `max_parallel`, else CPU count]
    #[arg(short, long, value_name = "N")]
    pub jobs: Option<NonZeroUsize>,

    /// Disable colored output (TUI and console); also set by a non-empty `NO_COLOR` env var
    #[arg(long, global = true)]
    pub no_color: bool,

    /// TUI only: start with the CPU/MEM stats panel hidden (`m` toggles it);
    /// no effect with --simple, --fix or --list
    #[arg(long)]
    pub no_stats: bool,

    /// Run every check, even ones unchanged since their last passing run
    /// (passes are still recorded in `.git/ci-tui/`)
    #[arg(long)]
    pub no_cache: bool,

    /// TUI only: ring the bell and send a desktop notification (OSC 9) when a
    /// full run finishes; also config `notify: true`. No effect with --simple,
    /// --fix or --list
    #[arg(long)]
    pub notify: bool,

    /// TUI only: quit when the run finishes; exit 1 if any check failed, else 0
    #[arg(long, conflicts_with_all = ["simple", "list", "fix"])]
    pub exit_on_finish: bool,

    /// TUI only: re-run the checks a file save affects (like `r`), until you
    /// quit. Watches the repo; skips `ignore_patterns`, gitignored files,
    /// editor temp files, `.git/`, `.ci-tui/` and nested repos
    #[arg(long, conflicts_with_all = ["simple", "list", "fix", "exit_on_finish"])]
    pub watch: bool,

    /// Simple-mode output format; `json` / `junit` imply --simple and put only
    /// the report on stdout (warnings stay on stderr) unless --output is given.
    /// Not with --list, --fix, --watch or --exit-on-finish
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Write the --format json / junit report to FILE; stdout keeps the text output
    #[arg(long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

/// `--format` / `--output` misuse clap cannot express (`--format` has a
/// default, so `conflicts_with` would also reject an explicit `--format text`)
fn report_flag_error(cli: &Cli) -> Option<String> {
    let format = match cli.format {
        Format::Text if cli.output.is_some() => {
            return Some("--output requires --format json or --format junit".to_string())
        }
        Format::Text => return None,
        Format::Json => "json",
        Format::Junit => "junit",
    };
    let modes = [
        ("list", cli.list),
        ("fix", cli.fix),
        ("watch", cli.watch),
        ("exit-on-finish", cli.exit_on_finish),
    ];
    let (mode, _) = modes.into_iter().find(|&(_, on)| on)?;
    Some(format!("--format {format} cannot be used with --{mode}"))
}

/// `--base` value parser: rejects an empty ref (e.g. `--base=$UNSET_VAR`).
fn parse_base_arg(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("git ref must not be empty".to_string());
    }
    Ok(value.to_string())
}

/// `--only` / `--group` values after comma split: trimmed (`"a, b"`), empty
/// items dropped (`a,` or an empty `"$VAR"` means no filter), deduped in order.
fn normalize_ids(ids: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let id = id.trim();
        if !id.is_empty() && !out.iter().any(|o| o == id) {
            out.push(id.to_string());
        }
    }
    out
}

/// `--files` value parser. `--files` takes 1.. values, so a trailing subcommand
/// (`-f a.rs validate`) would otherwise be swallowed as a file path.
fn parse_file_arg(value: &str) -> Result<PathBuf, String> {
    if <Command as Subcommand>::has_subcommand(value) {
        return Err(format!(
            "`{value}` is a subcommand, not a file (subcommands cannot be combined with --files)"
        ));
    }
    Ok(PathBuf::from(value))
}

impl Cli {
    /// Parse process args, exiting with a clap error on invalid combinations.
    pub fn parse_checked() -> Self {
        Self::try_parse_checked(std::env::args_os()).unwrap_or_else(|e| e.exit())
    }

    /// Parse `args` and reject run-mode flags (every non-global top-level
    /// arg) combined with a subcommand.
    ///
    /// Not `args_conflicts_with_subcommands`: that also rejects the global `--config`.
    pub fn try_parse_checked<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let mut command = Self::command();
        let matches = command.try_get_matches_from_mut(args)?;
        if let Some(name) = run_flag_with_subcommand(&command, &matches) {
            return Err(command.error(
                clap::error::ErrorKind::ArgumentConflict,
                format!("--{name} cannot be used with a subcommand"),
            ));
        }
        let mut cli = Self::from_arg_matches(&matches).map_err(|e| e.format(&mut command))?;
        if let Some(message) = report_flag_error(&cli) {
            return Err(command.error(clap::error::ErrorKind::ArgumentConflict, message));
        }
        cli.only = normalize_ids(cli.only);
        cli.groups = normalize_ids(cli.groups);
        Ok(cli)
    }
}

/// Long name of the first run flag (non-global top-level arg) given on the
/// command line, when a subcommand is given too
fn run_flag_with_subcommand(command: &clap::Command, matches: &clap::ArgMatches) -> Option<String> {
    matches.subcommand()?;
    let given = |arg: &&clap::Arg| {
        !arg.is_global_set()
            && matches.value_source(arg.get_id().as_str()) == Some(ValueSource::CommandLine)
    };
    let arg = command.get_arguments().find(given)?;
    Some(arg.get_long().unwrap_or(arg.get_id().as_str()).to_string())
}

impl Cli {
    /// Color on unless `--no-color` or a non-empty `NO_COLOR` env var
    /// (see [`crate::color::should_color`])
    pub fn color_enabled(&self) -> bool {
        crate::color::should_color(
            self.no_color,
            std::env::var_os(crate::color::NO_COLOR_ENV).as_deref(),
        )
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Generate a starter ci-tui.yaml
    Init {
        /// Output path [default: --config value, else ci-tui.yaml]
        path: Option<PathBuf>,
    },
    /// Validate a config file (exit non-zero if invalid)
    Validate {
        /// Config path [default: --config value, else ci-tui.yaml]
        path: Option<PathBuf>,
    },
    /// Print the config JSON Schema (for editor autocomplete / validation)
    Schema,
}

/// Clap-styled usage error for a missing `--config` ([`CONFIG_ERROR`](crate::exit::CONFIG_ERROR)).
pub fn missing_config_error() -> clap::Error {
    Cli::command().error(
        clap::error::ErrorKind::MissingRequiredArgument,
        "--config <CONFIG> is required: no ci-tui.yaml in the current directory (run `ci-tui init` to create one)",
    )
}

/// Clap-styled usage error for a bad `--only` / `--group` value ([`CONFIG_ERROR`](crate::exit::CONFIG_ERROR)).
pub fn filter_error(message: String) -> clap::Error {
    Cli::command().error(clap::error::ErrorKind::InvalidValue, message)
}

impl Command {
    /// Config path for `init` / `validate`: positional arg, then `--config`,
    /// then `ci-tui.yaml`. `None` for `schema`, which reads no config.
    pub fn config_path(&self, config: Option<PathBuf>) -> Option<PathBuf> {
        let (Self::Init { path } | Self::Validate { path }) = self else {
            return None;
        };
        Some(
            path.clone()
                .or(config)
                .unwrap_or_else(|| PathBuf::from(crate::commands::DEFAULT_CONFIG_FILE)),
        )
    }
}

/// Config path for running checks: `--config`, else `ci-tui.yaml` in `cwd` if
/// present. `None` means no config was given or found.
pub fn run_config_path(config: Option<PathBuf>, cwd: &Path) -> Option<PathBuf> {
    config.or_else(|| {
        cwd.join(crate::commands::DEFAULT_CONFIG_FILE)
            .is_file()
            .then(|| PathBuf::from(crate::commands::DEFAULT_CONFIG_FILE))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_config_error() {
        let cli = Cli::try_parse_from(["ci-tui"]).unwrap();
        assert!(cli.command.is_none() && cli.config.is_none());
        let err = missing_config_error();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        assert_eq!(err.exit_code(), crate::exit::CONFIG_ERROR);
    }

    /// Parse `args` and return the resolved `init` / `validate` config path.
    fn resolved(args: &[&str]) -> PathBuf {
        let cli = Cli::try_parse_checked(args).unwrap();
        cli.command
            .expect("expected a subcommand")
            .config_path(cli.config)
            .expect("init / validate resolve a path")
    }

    #[rstest::rstest]
    #[case::fix_before_validate(&["ci-tui", "--fix", "validate"])]
    #[case::simple_before_init(&["ci-tui", "-s", "init"])]
    #[case::files_before_init(&["ci-tui", "--files", "a.rs", "-s", "init"])]
    #[case::base_before_validate(&["ci-tui", "--base", "v1.0", "validate"])]
    #[case::base_then_files(&["ci-tui", "--base", "main", "--files", "a.rs"])]
    #[case::files_then_base(&["ci-tui", "-f", "a.rs", "--base", "main"])]
    #[case::list_with_fix(&["ci-tui", "--list", "--fix"])]
    #[case::list_with_simple(&["ci-tui", "--list", "-s"])]
    #[case::dry_run_with_fix(&["ci-tui", "--fix", "--dry-run"])]
    #[case::list_before_validate(&["ci-tui", "--list", "validate"])]
    #[case::dry_run_before_init(&["ci-tui", "--dry-run", "init"])]
    #[case::simple_before_schema(&["ci-tui", "-s", "schema"])]
    #[case::staged_with_files(&["ci-tui", "--staged", "-f", "a.rs"])]
    #[case::files_then_staged(&["ci-tui", "-f", "a.rs", "--staged"])]
    #[case::staged_with_base(&["ci-tui", "--staged", "--base", "main"])]
    #[case::staged_with_fix(&["ci-tui", "--staged", "--fix"])]
    #[case::fix_then_staged(&["ci-tui", "--fix", "--staged"])]
    #[case::staged_before_validate(&["ci-tui", "--staged", "validate"])]
    #[case::staged_before_schema(&["ci-tui", "--staged", "schema"])]
    #[case::only_before_validate(&["ci-tui", "--only", "a", "validate"])]
    #[case::group_before_init(&["ci-tui", "--group", "g", "init"])]
    #[case::jobs_before_validate(&["ci-tui", "-j", "2", "validate"])]
    #[case::no_stats_before_validate(&["ci-tui", "--no-stats", "validate"])]
    #[case::no_cache_before_validate(&["ci-tui", "--no-cache", "validate"])]
    #[case::notify_before_validate(&["ci-tui", "--notify", "validate"])]
    #[case::exit_on_finish_before_init(&["ci-tui", "--exit-on-finish", "init"])]
    #[case::exit_on_finish_with_simple(&["ci-tui", "--exit-on-finish", "-s"])]
    #[case::simple_then_exit_on_finish(&["ci-tui", "-s", "--exit-on-finish"])]
    #[case::exit_on_finish_with_list(&["ci-tui", "--exit-on-finish", "--list"])]
    #[case::dry_run_then_exit_on_finish(&["ci-tui", "--dry-run", "--exit-on-finish"])]
    #[case::exit_on_finish_with_fix(&["ci-tui", "--exit-on-finish", "--fix"])]
    #[case::list_then_no_verify(&["ci-tui", "--list", "--no-verify"])]
    #[case::no_verify_before_validate(&["ci-tui", "--no-verify", "validate"])]
    #[case::watch_with_simple(&["ci-tui", "--watch", "-s"])]
    #[case::watch_with_list(&["ci-tui", "--watch", "--list"])]
    #[case::dry_run_then_watch(&["ci-tui", "--dry-run", "--watch"])]
    #[case::watch_with_fix(&["ci-tui", "--watch", "--fix"])]
    #[case::watch_with_exit_on_finish(&["ci-tui", "--watch", "--exit-on-finish"])]
    #[case::exit_on_finish_then_watch(&["ci-tui", "--exit-on-finish", "--watch"])]
    #[case::watch_before_validate(&["ci-tui", "--watch", "validate"])]
    #[case::format_json_with_list(&["ci-tui", "--format", "json", "--list"])]
    #[case::dry_run_then_format_junit(&["ci-tui", "--dry-run", "--format=junit"])]
    #[case::format_json_with_fix(&["ci-tui", "--format", "json", "--fix"])]
    #[case::format_junit_with_watch(&["ci-tui", "--format", "junit", "--watch"])]
    #[case::exit_on_finish_then_format(&["ci-tui", "--exit-on-finish", "--format", "json"])]
    #[case::output_without_format(&["ci-tui", "-s", "--output", "r.json"])]
    #[case::output_with_text_format(&["ci-tui", "--format", "text", "--output", "r.txt"])]
    #[case::format_before_validate(&["ci-tui", "--format", "json", "validate"])]
    #[case::output_before_init(&["ci-tui", "--format", "json", "--output", "r", "init"])]
    fn test_conflicting_run_flags(#[case] args: &[&str]) {
        let err = Cli::try_parse_checked(args)
            .err()
            .expect("expected conflict");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn test_schema_subcommand_parsed() {
        let cli = Cli::try_parse_checked(["ci-tui", "schema"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Schema)));
    }

    #[test]
    fn test_base_flag_parsed() {
        let cli = Cli::try_parse_checked(["ci-tui", "--base", "v1.0"]).unwrap();
        assert_eq!(cli.base.as_deref(), Some("v1.0"));
        assert!(Cli::try_parse_checked(["ci-tui"]).unwrap().base.is_none());
    }

    #[rstest::rstest]
    #[case::list(&["ci-tui", "--list"])]
    #[case::dry_run_alias(&["ci-tui", "--dry-run"])]
    #[case::with_files(&["ci-tui", "--list", "-f", "a.rs", "b.rs"])]
    #[case::with_base(&["ci-tui", "--dry-run", "--base", "v1.0"])]
    fn test_list_flag_parsed(#[case] args: &[&str]) {
        let cli = Cli::try_parse_checked(args).unwrap();
        assert!(cli.list);
    }

    #[rstest::rstest]
    #[case::alone(&["ci-tui", "--staged"])]
    #[case::simple(&["ci-tui", "--staged", "-s"])]
    #[case::list(&["ci-tui", "--staged", "--list"])]
    #[case::dry_run(&["ci-tui", "--dry-run", "--staged"])]
    #[case::with_config(&["ci-tui", "-c", "x.yaml", "--staged"])]
    fn test_staged_flag_parsed(#[case] args: &[&str]) {
        let cli = Cli::try_parse_checked(args).unwrap();
        assert!(cli.staged);
        assert!(cli.command.is_none() && cli.files.is_empty() && cli.base.is_none());
    }

    #[test]
    fn test_staged_flag_default_off() {
        assert!(!Cli::try_parse_checked(["ci-tui"]).unwrap().staged);
    }

    #[rstest::rstest]
    #[case::comma(&["ci-tui", "--only", "a,b"], &["a", "b"], &[])]
    #[case::repeated(&["ci-tui", "--only", "a", "--only=b"], &["a", "b"], &[])]
    #[case::group(&["ci-tui", "--group", "g1,g2", "--group", "g3"], &[], &["g1", "g2", "g3"])]
    #[case::both_with_modes(&["ci-tui", "-s", "--only", "a", "--group", "g"], &["a"], &["g"])]
    #[case::with_fix(&["ci-tui", "--fix", "--group", "g"], &[], &["g"])]
    #[case::with_list(&["ci-tui", "--list", "--only", "a"], &["a"], &[])]
    #[case::after_files(&["ci-tui", "-f", "x.rs", "--only", "a"], &["a"], &[])]
    #[case::none(&["ci-tui"], &[], &[])]
    #[case::spaces_trimmed(&["ci-tui", "--only", "a, b", "--group", " g "], &["a", "b"], &["g"])]
    #[case::trailing_comma(&["ci-tui", "--only", "a,"], &["a"], &[])]
    #[case::empty_value_is_no_filter(&["ci-tui", "--only", "", "--group="], &[], &[])]
    #[case::deduped(&["ci-tui", "--only", "a,b,a", "--only", "b"], &["a", "b"], &[])]
    fn test_only_group_flags_parsed(
        #[case] args: &[&str],
        #[case] only: &[&str],
        #[case] groups: &[&str],
    ) {
        let cli = Cli::try_parse_checked(args).unwrap();
        assert_eq!(cli.only, only);
        assert_eq!(cli.groups, groups);
    }

    #[rstest::rstest]
    #[case::long(&["ci-tui", "--jobs", "4"], Some(4))]
    #[case::short(&["ci-tui", "-j", "1"], Some(1))]
    #[case::unset(&["ci-tui"], None)]
    fn test_jobs_flag_parsed(#[case] args: &[&str], #[case] jobs: Option<usize>) {
        let cli = Cli::try_parse_checked(args).unwrap();
        assert_eq!(cli.jobs.map(NonZeroUsize::get), jobs);
    }

    #[rstest::rstest]
    #[case::zero("--jobs=0")]
    fn test_jobs_rejects_invalid(#[case] arg: &str) {
        let err = Cli::try_parse_checked(["ci-tui", arg])
            .err()
            .expect("expected error");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
        assert_eq!(err.exit_code(), crate::exit::CONFIG_ERROR);
    }

    #[rstest::rstest]
    #[case::unset(&["ci-tui"], false)]
    #[case::run(&["ci-tui", "--no-color", "-s"], true)]
    #[case::after_subcommand(&["ci-tui", "validate", "--no-color"], true)]
    fn test_no_color_flag_parsed(#[case] args: &[&str], #[case] expected: bool) {
        assert_eq!(Cli::try_parse_checked(args).unwrap().no_color, expected);
    }

    #[rstest::rstest]
    #[case::unset(&["ci-tui"], false)]
    #[case::set(&["ci-tui", "--no-stats"], true)]
    fn test_no_stats_flag_parsed(#[case] args: &[&str], #[case] expected: bool) {
        assert_eq!(Cli::try_parse_checked(args).unwrap().no_stats, expected);
    }

    #[rstest::rstest]
    #[case::unset(&["ci-tui"], false)]
    #[case::set(&["ci-tui", "--no-cache"], true)]
    #[case::simple(&["ci-tui", "-s", "--no-cache", "-f", "a.rs"], true)]
    fn test_no_cache_flag_parsed(#[case] args: &[&str], #[case] expected: bool) {
        assert_eq!(Cli::try_parse_checked(args).unwrap().no_cache, expected);
    }

    #[rstest::rstest]
    #[case::unset(&["ci-tui"], false, false)]
    #[case::notify(&["ci-tui", "--notify"], true, false)]
    #[case::exit_on_finish(&["ci-tui", "--exit-on-finish"], false, true)]
    #[case::both(&["ci-tui", "--notify", "--exit-on-finish", "-f", "a.rs"], true, true)]
    #[case::notify_with_simple(&["ci-tui", "-s", "--notify"], true, false)]
    fn test_notify_exit_on_finish_flags_parsed(
        #[case] args: &[&str],
        #[case] notify: bool,
        #[case] exit_on_finish: bool,
    ) {
        let cli = Cli::try_parse_checked(args).unwrap();
        assert_eq!((cli.notify, cli.exit_on_finish), (notify, exit_on_finish));
    }

    #[rstest::rstest]
    #[case::unset(&["ci-tui"], false)]
    #[case::fix(&["ci-tui", "--fix", "--no-verify"], true)]
    #[case::simple_fix(&["ci-tui", "-s", "--fix", "--no-verify"], true)]
    #[case::tui(&["ci-tui", "--no-verify", "-f", "a.rs"], true)]
    fn test_no_verify_flag_parsed(#[case] args: &[&str], #[case] expected: bool) {
        assert_eq!(Cli::try_parse_checked(args).unwrap().no_verify, expected);
    }

    #[rstest::rstest]
    #[case::unset(&["ci-tui"], false)]
    #[case::set(&["ci-tui", "--watch"], true)]
    #[case::with_tui_flags(&["ci-tui", "--watch", "--notify", "-f", "a.rs", "--only", "x"], true)]
    fn test_watch_flag_parsed(#[case] args: &[&str], #[case] expected: bool) {
        assert_eq!(Cli::try_parse_checked(args).unwrap().watch, expected);
    }

    #[rstest::rstest]
    #[case::default(&["ci-tui"], Format::Text, None)]
    #[case::text_with_list(&["ci-tui", "--format", "text", "--list"], Format::Text, None)]
    #[case::text_with_watch(&["ci-tui", "--format=text", "--watch"], Format::Text, None)]
    #[case::json(&["ci-tui", "--format", "json"], Format::Json, None)]
    #[case::junit_simple(&["ci-tui", "-s", "--format", "junit"], Format::Junit, None)]
    #[case::json_output(&["ci-tui", "--format", "json", "--output", "r.json"], Format::Json, Some("r.json"))]
    fn test_format_output_flags_parsed(
        #[case] args: &[&str],
        #[case] format: Format,
        #[case] output: Option<&str>,
    ) {
        let cli = Cli::try_parse_checked(args).unwrap();
        assert_eq!(cli.format, format);
        assert_eq!(cli.output, output.map(PathBuf::from));
    }

    #[test]
    fn test_format_rejects_unknown_value() {
        let err = Cli::try_parse_checked(["ci-tui", "--format", "xml"])
            .err()
            .expect("expected error");
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
        assert_eq!(err.exit_code(), crate::exit::CONFIG_ERROR);
    }

    #[test]
    fn test_no_cache_flag_in_help() {
        let help = Cli::command().render_help().to_string();
        assert!(help.contains("--no-cache"), "{help}");
    }

    #[test]
    fn test_list_flag_default_off() {
        assert!(!Cli::try_parse_checked(["ci-tui"]).unwrap().list);
    }

    #[rstest::rstest]
    #[case::empty("--base=")]
    #[case::whitespace("--base=  ")]
    fn test_base_rejects_empty_ref(#[case] arg: &str) {
        let err = Cli::try_parse_checked(["ci-tui", arg])
            .err()
            .expect("expected error");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[rstest::rstest]
    #[case::validate(&["ci-tui", "-c", "x.yaml", "-f", "a.rs", "validate"])]
    #[case::init(&["ci-tui", "-f", "a.rs", "init"])]
    fn test_files_rejects_subcommand_name(#[case] args: &[&str]) {
        let err = Cli::try_parse_checked(args).err().expect("expected error");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[rstest::rstest]
    #[case::config_before_subcommand(&["ci-tui", "-c", "a.yaml", "validate"], "a.yaml")]
    #[case::config_after_subcommand(&["ci-tui", "validate", "--config", "a.yaml"], "a.yaml")]
    #[case::positional_path(&["ci-tui", "validate", "b.yaml"], "b.yaml")]
    #[case::positional_beats_config(&["ci-tui", "-c", "a.yaml", "validate", "b.yaml"], "b.yaml")]
    #[case::validate_default(&["ci-tui", "validate"], "ci-tui.yaml")]
    #[case::init_default(&["ci-tui", "init"], "ci-tui.yaml")]
    #[case::init_honors_config(&["ci-tui", "-c", "a.yaml", "init"], "a.yaml")]
    fn test_subcommand_config_path(#[case] args: &[&str], #[case] expected: &str) {
        assert_eq!(resolved(args), PathBuf::from(expected));
    }

    #[test]
    fn test_run_config_path_none_without_config_or_default_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_config_path(None, dir.path()), None);
    }

    #[test]
    fn test_run_config_path_falls_back_to_default_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci-tui.yaml"), "").unwrap();
        assert_eq!(
            run_config_path(None, dir.path()),
            Some(PathBuf::from("ci-tui.yaml"))
        );
    }

    #[test]
    fn test_run_config_path_prefers_config_flag() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci-tui.yaml"), "").unwrap();
        assert_eq!(
            run_config_path(Some("a.yaml".into()), dir.path()),
            Some(PathBuf::from("a.yaml"))
        );
    }
}
