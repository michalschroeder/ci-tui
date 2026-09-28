//! Check output outside the TUI: `o` / `O` open it in `$PAGER` / `$EDITOR`
//! (the TUI is suspended meanwhile, see [`super::run`]), `w` saves it to
//! `.ci-tui/logs/<check>.log`. Both use the same plain-text log
//! ([`check_log_text`]): ANSI escapes stripped, so pagers without `-R` and
//! editors show clean text.

use crate::runner::CheckResult;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// Log directory for `w`, relative to the project root (git cwd)
pub const LOG_DIR: &str = ".ci-tui/logs";

/// Ignores everything under `.ci-tui/` (written by [`save_log`])
const GITIGNORE: &str = ".ci-tui/.gitignore";

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
    // One buffer for all parts (outputs can be large)
    let mut text =
        String::with_capacity(command.len() + result.output.len() + result.error_output.len() + 64);
    text.push_str(&format!("$ {command}\nstatus: {:?}\n\n", result.status));
    push_plain(&mut text, &result.output);
    if !result.error_output.is_empty() {
        push_newline(&mut text);
        text.push_str("── stderr ──\n");
        push_plain(&mut text, &result.error_output);
    }
    push_newline(&mut text);
    Some(text)
}

/// Append `s` to `text` as plain text: ANSI CSI ([`crate::color::paint`])
/// and OSC (`ESC ]` up to BEL / `ESC \`: hyperlinks, titles) sequences
/// dropped. A bare `\r` (progress redraw) makes the next char start its
/// line over, so only the last redraw stays; `\r\n` is a newline.
fn push_plain(text: &mut String, s: &str) {
    let s = crate::color::paint(s, false);
    let mut line_start = text.rfind('\n').map_or(0, |i| i + 1);
    let mut carriage_return = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' if chars.next_if_eq(&']').is_some() => skip_osc(&mut chars),
            '\r' => carriage_return = true,
            '\n' => {
                carriage_return = false;
                text.push('\n');
                line_start = text.len();
            }
            c if carriage_return => {
                carriage_return = false;
                text.truncate(line_start);
                text.push(c);
            }
            c => text.push(c),
        }
    }
}

/// Skip an OSC body through its BEL / `ESC \` terminator
fn skip_osc(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(c) = chars.next() {
        if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
            return;
        }
    }
}

/// End `text` with a newline unless it already does
fn push_newline(text: &mut String) {
    if !text.ends_with('\n') {
        text.push('\n');
    }
}

/// `id` safe as a file name: anything but ASCII alphanumerics, `-`, `_`
/// and `.` becomes `_` (no `/`, so it cannot leave the log dir; the `.log`
/// suffix keeps `.` / `..` from naming a directory). A changed id gets a
/// hash of the original appended, so ids that sanitize alike (`a/b`,
/// `a_b`) keep separate files.
pub fn sanitize_id(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe == id {
        safe
    } else {
        format!("{safe}-{:08x}", fnv1a(id))
    }
}

/// 32-bit FNV-1a: stable across builds, unlike `DefaultHasher`
fn fnv1a(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// Log file of `check_id` under `root`: `<root>/.ci-tui/logs/<id>.log`
pub fn log_path(root: &Path, check_id: &str) -> PathBuf {
    root.join(LOG_DIR)
        .join(format!("{}.log", sanitize_id(check_id)))
}

/// Write `text` to [`log_path`], creating the log dir and overwriting any
/// previous log; returns the path written. Also writes `.ci-tui/.gitignore`
/// (`*`) if missing, so git change detection never sees saved logs.
pub fn save_log(root: &Path, check_id: &str, text: &str) -> io::Result<PathBuf> {
    let path = log_path(root, check_id);
    fs::create_dir_all(root.join(LOG_DIR))?;
    let gitignore = root.join(GITIGNORE);
    if !gitignore.exists() {
        fs::write(gitignore, "*\n")?;
    }
    fs::write(&path, text)?;
    Ok(path)
}

/// Private (0700) temp dir for viewer files, removed when dropped (TUI exit)
pub fn viewer_dir() -> io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("ci-tui-");
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    builder.tempdir()
}

/// Write `text` to `<dir>/<id>.log` for the viewer. It lives until `dir`
/// is dropped, so viewers that return at once (`code` without `--wait`)
/// still find it.
pub fn write_viewer_file(dir: &Path, check_id: &str, text: &str) -> io::Result<PathBuf> {
    let path = dir.join(format!("{}.log", sanitize_id(check_id)));
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

/// Run `command` on `path` with the terminal inherited (blocks until it
/// exits). `Err` holds a status message: spawn failure, command not found,
/// or non-zero exit.
pub fn run_viewer(command: &str, path: &Path) -> Result<(), String> {
    match viewer_process(command, path).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(exit_message(command, status)),
        Err(e) => Err(format!("Failed to run `{command}`: {e}")),
    }
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
    #[case::slash("../../etc/passwd", ".._.._etc_passwd-")]
    #[case::spaces_unicode("my check ✓", "my_check__-")]
    fn test_sanitize_id(#[case] id: &str, #[case] expected: &str) {
        let safe = sanitize_id(id);
        if expected.ends_with('-') {
            // Changed ids: prefix plus an 8-hex-digit hash
            let hash = safe.strip_prefix(expected).expect(&safe);
            assert!(hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
        } else {
            assert_eq!(safe, expected);
        }
    }

    #[test]
    fn test_sanitize_id_keeps_alike_ids_apart() {
        assert_ne!(sanitize_id("unit/api"), sanitize_id("unit_api"));
        assert_ne!(sanitize_id("lint:php"), sanitize_id("lint_php"));
    }

    #[rstest::rstest]
    #[case::osc_hyperlink_bel("see \x1b]8;;https://x.y\x07link\x1b]8;;\x07!", "see link!")]
    #[case::osc_title_st("\x1b]0;title\x1b\\ok", "ok")]
    #[case::progress_redraw("10%\r50%\r100%\ndone", "100%\ndone")]
    #[case::crlf("a\r\nb", "a\nb")]
    #[case::trailing_cr("Done\r", "Done")]
    #[case::csi("\x1b[1mbold\x1b[0m", "bold")]
    fn test_push_plain(#[case] input: &str, #[case] expected: &str) {
        let mut text = String::from("header\n");
        push_plain(&mut text, input);
        assert_eq!(text, format!("header\n{expected}"));
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
        let path = save_log(dir.path(), "a-b", "first").unwrap();
        assert_eq!(path, dir.path().join(".ci-tui/logs/a-b.log"));
        save_log(dir.path(), "a-b", "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let gitignore = dir.path().join(".ci-tui/.gitignore");
        assert_eq!(fs::read_to_string(gitignore).unwrap(), "*\n");
    }

    #[test]
    fn test_save_log_keeps_existing_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".ci-tui")).unwrap();
        fs::write(dir.path().join(".ci-tui/.gitignore"), "logs/\n").unwrap();
        save_log(dir.path(), "php-lint", "x").unwrap();
        let kept = fs::read_to_string(dir.path().join(".ci-tui/.gitignore")).unwrap();
        assert_eq!(kept, "logs/\n");
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

    /// `text` in a viewer file under a fresh private dir (dir kept alive)
    fn viewer_file(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = viewer_dir().unwrap();
        let path = write_viewer_file(dir.path(), "php-lint", text).unwrap();
        (dir, path)
    }

    #[test]
    fn test_run_viewer_passes_args_and_file_content() {
        // `grep -q` stands in for `$PAGER` with args: succeeds only if the
        // file it gets holds the text
        let (_dir, path) = viewer_file("hay needle\n");
        assert_eq!(run_viewer("grep -q needle", &path), Ok(()));
        let (_dir, path) = viewer_file("hay\n");
        let err = run_viewer("grep -q needle", &path).unwrap_err();
        assert!(err.contains("exited with status 1"), "{err}");
    }

    #[test]
    fn test_run_viewer_reports_missing_program() {
        let (_dir, path) = viewer_file("x");
        let err = run_viewer("ci-tui-no-such-pager", &path).unwrap_err();
        assert!(err.contains("command not found"), "{err}");
    }

    #[test]
    fn test_viewer_file_outlives_viewer_until_dir_dropped() {
        let (dir, path) = viewer_file("x");
        run_viewer("true", &path).unwrap();
        assert!(path.exists(), "kept for viewers that return at once");
        let root = dir.path().to_path_buf();
        drop(dir);
        assert!(!root.exists(), "removed with the dir");
    }

    #[cfg(unix)]
    #[test]
    fn test_viewer_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = viewer_dir().unwrap();
        let mode = fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}
