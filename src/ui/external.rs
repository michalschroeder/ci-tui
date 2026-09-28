//! Check output outside the TUI: `o` / `O` open it in `$PAGER` / `$EDITOR`
//! (the TUI is suspended meanwhile, see [`super::run`]), `w` saves it to
//! `.ci-tui/logs/<check>.log`. Both use the same plain-text log
//! ([`check_log_text`]): ANSI escapes stripped, so pagers without `-R` and
//! editors show clean text.

use crate::runner::CheckResult;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// Log directory for `w`, relative to the project root (git cwd)
pub const LOG_DIR: &str = ".ci-tui/logs";

/// `sh` exit code for "command not found"
const EXIT_NOT_FOUND: i32 = 127;

/// External program the output opens in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Viewer {
    /// `$PAGER`, else `less` (`o`)
    Pager,
    /// `$EDITOR`, else `vi` (`O`)
    Editor,
}

impl Viewer {
    /// Env var naming the program
    fn env_var(self) -> &'static str {
        match self {
            Self::Pager => "PAGER",
            Self::Editor => "EDITOR",
        }
    }

    /// Program used when the env var is unset or blank
    fn fallback(self) -> &'static str {
        match self {
            Self::Pager => "less",
            Self::Editor => "vi",
        }
    }

    /// Name for status messages
    pub fn label(self) -> &'static str {
        match self {
            Self::Pager => "Pager",
            Self::Editor => "Editor",
        }
    }

    /// Command line from the environment ([`resolve_command`])
    pub fn command(self) -> String {
        let value = std::env::var(self.env_var()).ok();
        resolve_command(value.as_deref(), self.fallback())
    }
}

/// `value` (an env var, may carry args like `less -R`) trimmed, or
/// `fallback` when unset / blank
pub fn resolve_command(value: Option<&str>, fallback: &str) -> String {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Plain-text log of a check: its command, status, stdout, then stderr
/// under a separator. `None` while it has no output (pending, on-demand,
/// never ran), so the keys have nothing to show.
pub fn check_log_text(command: &str, result: &CheckResult) -> Option<String> {
    if result.output.is_empty() && result.error_output.is_empty() {
        return None;
    }
    // Strip each part straight into one buffer (outputs can be large)
    let plain = |s: &str| crate::color::paint(s, false).into_owned();
    let mut text =
        String::with_capacity(command.len() + result.output.len() + result.error_output.len() + 64);
    text.push_str(&format!("$ {command}\nstatus: {:?}\n\n", result.status));
    text.push_str(&plain(&result.output));
    if !result.error_output.is_empty() {
        push_newline(&mut text);
        text.push_str("── stderr ──\n");
        text.push_str(&plain(&result.error_output));
    }
    push_newline(&mut text);
    Some(text)
}

/// End `text` with a newline unless it already does
fn push_newline(text: &mut String) {
    if !text.ends_with('\n') {
        text.push('\n');
    }
}

/// `id` safe as a file name: anything but ASCII alphanumerics, `-`, `_`
/// and `.` becomes `_` (no `/`, so it cannot leave the log dir; the `.log`
/// suffix keeps `.` / `..` from naming a directory)
pub fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Log file of `check_id` under `root`: `<root>/.ci-tui/logs/<id>.log`
pub fn log_path(root: &Path, check_id: &str) -> PathBuf {
    root.join(LOG_DIR)
        .join(format!("{}.log", sanitize_id(check_id)))
}

/// Write `text` to [`log_path`], creating the log dir and overwriting any
/// previous log; returns the path written
pub fn save_log(root: &Path, check_id: &str, text: &str) -> io::Result<PathBuf> {
    let path = log_path(root, check_id);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, text)?;
    Ok(path)
}

/// `command` (a shell command line, may carry args) run by `sh` with `path`
/// appended as its last argument. `sh -c` handles quoting in `$PAGER` /
/// `$EDITOR` (e.g. `code --wait`) the way git and man do; `path` is passed
/// as `$1`, never spliced into the script, so it needs no escaping.
pub fn viewer_process(command: &str, path: &Path) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(format!("{command} \"$1\""))
        .arg("ci-tui")
        .arg(path);
    cmd
}

/// Write `text` to a temp file, run `command` on it with the terminal
/// inherited (blocks until it exits), then delete the file. `Err` holds a
/// status message: spawn failure, command not found, or non-zero exit.
pub fn run_viewer(command: &str, check_id: &str, text: &str) -> Result<(), String> {
    let path = temp_path(check_id);
    write_new(&path, text).map_err(|e| format!("Temp file {}: {e}", path.display()))?;
    let status = viewer_process(command, &path).status();
    let _ = fs::remove_file(&path);
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(exit_message(command, status)),
        Err(e) => Err(format!("Failed to run `{command}`: {e}")),
    }
}

/// Temp file `o` / `O` hand to the viewer
fn temp_path(check_id: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ci-tui-{}-{}.log",
        std::process::id(),
        sanitize_id(check_id)
    ))
}

/// Create `path` afresh (a stale file from a crashed run is replaced;
/// `create_new` never writes through a pre-planted symlink)
fn write_new(path: &Path, text: &str) -> io::Result<()> {
    let _ = fs::remove_file(path);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(text.as_bytes())
}

/// Status message for a viewer that exited unsuccessfully
fn exit_message(command: &str, status: ExitStatus) -> String {
    match status.code() {
        Some(EXIT_NOT_FOUND) => format!("`{command}`: command not found"),
        Some(code) => format!("`{command}` exited with status {code}"),
        None => format!("`{command}` killed by a signal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::CheckStatus;

    fn result(status: CheckStatus, output: &str, error_output: &str) -> CheckResult {
        let mut result = CheckResult::pending("php-lint");
        result.status = status;
        result.output = output.to_string();
        result.error_output = error_output.to_string();
        result
    }

    #[rstest::rstest]
    #[case::unset(None, "less")]
    #[case::empty(Some(""), "less")]
    #[case::blank(Some("  "), "less")]
    #[case::program(Some("most"), "most")]
    #[case::with_args(Some(" less -R "), "less -R")]
    fn test_resolve_command(#[case] value: Option<&str>, #[case] expected: &str) {
        assert_eq!(resolve_command(value, "less"), expected);
    }

    #[test]
    fn test_viewer_fallbacks() {
        assert_eq!(Viewer::Pager.fallback(), "less");
        assert_eq!(Viewer::Editor.fallback(), "vi");
        assert_eq!(Viewer::Pager.env_var(), "PAGER");
        assert_eq!(Viewer::Editor.env_var(), "EDITOR");
    }

    #[test]
    fn test_check_log_text_strips_ansi_and_appends_stderr() {
        let r = result(
            CheckStatus::Failed,
            "\x1b[31merror\x1b[0m in Foo.php",
            "warn\n",
        );
        assert_eq!(
            check_log_text("php -l src/Foo.php", &r).unwrap(),
            "$ php -l src/Foo.php\nstatus: Failed\n\nerror in Foo.php\n── stderr ──\nwarn\n"
        );
    }

    #[test]
    fn test_check_log_text_none_without_output() {
        assert!(check_log_text("cmd", &result(CheckStatus::OnDemand, "", "")).is_none());
        assert!(check_log_text("cmd", &result(CheckStatus::Running, "", "e")).is_some());
    }

    #[rstest::rstest]
    #[case::plain("php-lint", "php-lint")]
    #[case::dotted("unit.v2_x", "unit.v2_x")]
    #[case::slash("../../etc/passwd", ".._.._etc_passwd")]
    #[case::spaces_unicode("my check ✓", "my_check__")]
    fn test_sanitize_id(#[case] id: &str, #[case] expected: &str) {
        assert_eq!(sanitize_id(id), expected);
    }

    #[test]
    fn test_log_path_stays_in_log_dir() {
        let root = Path::new("/repo");
        assert_eq!(
            log_path(root, "php-lint"),
            PathBuf::from("/repo/.ci-tui/logs/php-lint.log")
        );
        assert_eq!(
            log_path(root, "..").parent(),
            Some(Path::new("/repo/.ci-tui/logs"))
        );
    }

    #[test]
    fn test_save_log_creates_dirs_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = save_log(dir.path(), "a/b", "first").unwrap();
        assert_eq!(path, dir.path().join(".ci-tui/logs/a_b.log"));
        save_log(dir.path(), "a/b", "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn test_save_log_error_when_root_unwritable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        fs::write(&file, "").unwrap();
        assert!(save_log(&file, "php-lint", "x").is_err());
    }

    #[test]
    fn test_viewer_process_passes_path_as_last_arg() {
        let cmd = viewer_process("less -R", Path::new("/tmp/x y.log"));
        assert_eq!(cmd.get_program(), "sh");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, ["-c", "less -R \"$1\"", "ci-tui", "/tmp/x y.log"]);
    }

    #[test]
    fn test_run_viewer_passes_args_and_file_content() {
        // `grep -q` stands in for `$PAGER` with args: succeeds only if the
        // temp file it gets holds the text
        assert_eq!(
            run_viewer("grep -q needle", "php-lint", "hay needle\n"),
            Ok(())
        );
        let err = run_viewer("grep -q needle", "php-lint", "hay\n").unwrap_err();
        assert!(err.contains("exited with status 1"), "{err}");
    }

    #[test]
    fn test_run_viewer_reports_missing_program() {
        let err = run_viewer("ci-tui-no-such-pager", "php-lint", "x").unwrap_err();
        assert!(err.contains("command not found"), "{err}");
    }

    #[test]
    fn test_run_viewer_removes_temp_file() {
        let path = temp_path("rm-check");
        run_viewer("true", "rm-check", "x").unwrap();
        assert!(!path.exists());
    }
}
