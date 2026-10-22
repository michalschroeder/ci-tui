//! Machine-readable simple-mode output (#147): `--format json|junit`
//! reports (to stdout, or `--output <file>` next to the text output) and
//! GitHub Actions workflow commands (`::group::` / `::error`, `::warning`
//! for `allow_failure` checks) when `GITHUB_ACTIONS=true` and stdout
//! carries the text output.
//!
//! Reports cover the checks simple mode ran (cached ones included, on-demand
//! ones never run there). Output text is ANSI-stripped; JUnit is hand-rolled
//! XML (escaped, XML-invalid control chars dropped) to avoid a dependency.

use crate::checks::{severity_in, CheckToRun, Severity};
use crate::runner::{CheckResult, CheckStatus};
use anyhow::{Context, Result};
use serde::Serialize;
use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

/// JSON report schema version, bumped on breaking changes
const JSON_VERSION: u32 = 1;

/// `--format` value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Format {
    /// Human-readable console output
    #[default]
    Text,
    /// One JSON document
    Json,
    /// JUnit XML (`<testsuites>`, one `<testsuite>` per group)
    Junit,
}

/// Where simple mode writes what
#[derive(Debug, Clone, Default)]
pub struct ReportOptions {
    /// Report format; `Text` writes no report
    pub format: Format,
    /// Report file (`--output`); `None` puts the report on stdout instead of text
    pub output: Option<PathBuf>,
    /// Running under GitHub Actions (`GITHUB_ACTIONS=true`)
    pub github: bool,
}

impl ReportOptions {
    /// Options for `--format` / `--output`, annotating when the
    /// `GITHUB_ACTIONS` env var is `true`. Removes an existing `--output`
    /// file, so a run that stops before writing (error, Ctrl-C) leaves no
    /// stale report.
    ///
    /// # Errors
    ///
    /// Returns an error if the existing `--output` file cannot be removed.
    pub fn new(format: Format, output: Option<PathBuf>) -> Result<Self> {
        if let Some(path) = &output {
            remove_stale(path)?;
        }
        let github = std::env::var_os("GITHUB_ACTIONS").is_some_and(|v| v == "true");
        Ok(Self {
            format,
            output,
            github,
        })
    }

    /// A json / junit report is asked for
    pub fn writes_report(&self) -> bool {
        self.format != Format::Text
    }

    /// Stdout carries the text output (not a report document)
    pub fn text_on_stdout(&self) -> bool {
        !self.writes_report() || self.output.is_some()
    }

    /// Emit `::group::` / `::error` workflow commands in the text output
    pub fn annotate(&self) -> bool {
        self.github && self.text_on_stdout()
    }

    /// Write `report` to `--output`, else stdout (no-op for `Text`).
    ///
    /// # Errors
    ///
    /// Returns an error if the file or stdout cannot be written.
    pub fn write(&self, report: &Report) -> Result<()> {
        let doc = match self.format {
            Format::Text => return Ok(()),
            Format::Json => report.json(),
            Format::Junit => report.junit(),
        };
        match &self.output {
            Some(path) => std::fs::write(path, doc)
                .with_context(|| format!("Failed to write report to {}", path.display())),
            None => {
                let mut out = std::io::stdout().lock();
                out.write_all(doc.as_bytes())
                    .and_then(|()| out.flush())
                    .context("Failed to write report to stdout")
            }
        }
    }
}

/// Remove an old report at `path` (absent is fine)
fn remove_stale(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("Failed to remove old report {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// One simple-mode run, rendered as JSON or JUnit
pub struct Report<'a> {
    /// Ref changes were compared against
    pub base_ref: &'a str,
    /// Changed file count
    pub changed_files: usize,
    /// Wall time of the whole run
    pub duration_ms: u64,
    /// Selected checks (name, group, fix command lookup)
    pub checks: &'a [CheckToRun],
    /// Results in group order
    pub results: &'a [CheckResult],
    /// Counts of `results` ([`Summary::of`] with `checks`)
    pub summary: Summary,
}

#[derive(Serialize)]
struct JsonReport<'a> {
    version: u32,
    base_ref: &'a str,
    changed_files: usize,
    duration_ms: u64,
    summary: &'a Summary,
    checks: Vec<JsonCheck<'a>>,
}

/// Result counts shared by the text summary and the reports
#[derive(Serialize, Default)]
pub struct Summary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    /// Failures of `allow_failure` checks (not in `failed`)
    pub warnings: usize,
    pub cached: usize,
    /// JUnit `skipped`, not in the JSON summary
    #[serde(skip)]
    pub cancelled: usize,
}

impl Summary {
    /// Counts of `results`; failures of `allow_failure` checks (looked up
    /// in `checks`) are warnings
    pub fn of(checks: &[CheckToRun], results: &[CheckResult]) -> Self {
        let mut summary = Self {
            total: results.len(),
            ..Self::default()
        };
        for result in results {
            summary.cached += usize::from(result.cached);
            let bucket = match (severity_in(checks, result), &result.status) {
                (Severity::Error, _) => &mut summary.failed,
                (Severity::Warning, _) => &mut summary.warnings,
                (_, CheckStatus::Passed) => &mut summary.passed,
                (_, CheckStatus::Cancelled) => &mut summary.cancelled,
                _ => continue,
            };
            *bucket += 1;
        }
        summary
    }
}

/// `1 warning` / `N warnings`
pub fn warning_count(n: usize) -> String {
    if n == 1 {
        "1 warning".to_string()
    } else {
        format!("{n} warnings")
    }
}

/// Run outcome tail: `, N failed` (left out when there are only warnings)
/// and `, M warning(s)` (when any)
pub fn outcome_tail(failed: usize, warnings: usize) -> String {
    let mut tail = String::new();
    if failed > 0 || warnings == 0 {
        tail += &format!(", {failed} failed");
    }
    if warnings > 0 {
        tail += &format!(", {}", warning_count(warnings));
    }
    tail
}

#[derive(Serialize)]
struct JsonCheck<'a> {
    id: &'a str,
    name: &'a str,
    group: &'a str,
    status: &'static str,
    /// The check's `allow_failure` (a failure is a warning)
    allow_failure: bool,
    duration_ms: u64,
    cached: bool,
    output: Cow<'a, str>,
    error_output: Cow<'a, str>,
    fix_command: Option<&'a str>,
}

impl Report<'_> {
    /// The selected check `result` belongs to
    pub fn check(&self, result: &CheckResult) -> Option<&CheckToRun> {
        self.checks.iter().find(|c| c.id() == result.check_id)
    }

    /// Group key of `result`'s check (empty if unknown)
    fn group(&self, result: &CheckResult) -> &str {
        self.check(result).map_or("", CheckToRun::group)
    }

    /// Pretty-printed JSON document, newline-terminated
    pub fn json(&self) -> String {
        let checks = self.results.iter().map(|r| {
            let check = self.check(r);
            JsonCheck {
                id: &r.check_id,
                name: check.map_or(&r.check_id, |c| c.name()),
                group: check.map_or("", CheckToRun::group),
                status: status_name(&r.status),
                allow_failure: check.is_some_and(CheckToRun::allow_failure),
                duration_ms: r.duration_ms,
                cached: r.cached,
                // Not the TUI's "cached" hint: `cached` says it
                output: match r.cached {
                    true => Cow::Borrowed(""),
                    false => plain(&r.output),
                },
                error_output: plain(&r.error_output),
                fix_command: check.and_then(|c| c.resolved_fix_command.as_deref()),
            }
        });
        let report = JsonReport {
            version: JSON_VERSION,
            base_ref: self.base_ref,
            changed_files: self.changed_files,
            duration_ms: self.duration_ms,
            summary: &self.summary,
            checks: checks.collect(),
        };
        let mut doc = serde_json::to_string_pretty(&report).expect("report serializes");
        doc.push('\n');
        doc
    }

    /// JUnit XML document: one `<testsuite>` per group (in run order), one
    /// `<testcase>` per result; failures / timeouts carry `<failure>` with
    /// the output (`<skipped message="allowed failure: ..">` for
    /// `allow_failure` checks), cancelled checks `<skipped/>`
    pub fn junit(&self) -> String {
        let mut buf = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        let _ = writeln!(
            buf,
            "<testsuites name=\"ci-tui\" {} time=\"{}\">",
            counts(&self.summary),
            secs(self.duration_ms)
        );
        // Results arrive group by group
        for results in self.results.chunk_by(|a, b| self.group(a) == self.group(b)) {
            write_testsuite(&mut buf, self.checks, self.group(&results[0]), results);
        }
        buf.push_str("</testsuites>\n");
        buf
    }
}

/// JSON `status` value (the report schema, independent of [`CheckStatus`]
/// variant names)
fn status_name(status: &CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pending => "pending",
        CheckStatus::Queued => "queued",
        CheckStatus::Running => "running",
        CheckStatus::Passed => "passed",
        CheckStatus::Failed => "failed",
        CheckStatus::Skipped => "skipped",
        CheckStatus::OnDemand => "on_demand",
        CheckStatus::TimedOut => "timed_out",
        CheckStatus::Cancelled => "cancelled",
    }
}

/// Failure label of a failed / timed-out result, `None` otherwise
fn failure_label(status: &CheckStatus) -> Option<&'static str> {
    match status {
        CheckStatus::Failed => Some("failed"),
        CheckStatus::TimedOut => Some("timed out"),
        _ => None,
    }
}

/// `result`'s ANSI-stripped, trimmed stdout and stderr, empty parts skipped
fn failure_body(result: &CheckResult) -> String {
    let parts = [&result.output, &result.error_output].map(|s| plain(s));
    let parts: Vec<&str> = parts
        .iter()
        .map(|s| s.trim_end())
        .filter(|s| !s.is_empty())
        .collect();
    parts.join("\n")
}

/// One `<testsuite>` element for `group`'s `results` (of `checks`); `time`
/// sums them
fn write_testsuite(buf: &mut String, checks: &[CheckToRun], group: &str, results: &[CheckResult]) {
    let time: u64 = results.iter().map(|r| r.duration_ms).sum();
    let _ = writeln!(
        buf,
        "  <testsuite name=\"{}\" {} time=\"{}\">",
        xml(group),
        counts(&Summary::of(checks, results)),
        secs(time)
    );
    for result in results {
        write_testcase(buf, group, result, severity_in(checks, result));
    }
    buf.push_str("  </testsuite>\n");
}

/// One `<testcase>` element for `result` in `group`; a
/// [`Severity::Warning`] failure is skipped, not failed
fn write_testcase(buf: &mut String, group: &str, result: &CheckResult, severity: Severity) {
    let _ = write!(
        buf,
        "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\"",
        xml(&result.check_id),
        xml(group),
        secs(result.duration_ms)
    );
    if let Some(label) = failure_label(&result.status) {
        let (tag, message) = if severity == Severity::Warning {
            ("skipped", format!("allowed failure: {label}"))
        } else {
            ("failure", label.to_string())
        };
        let _ = writeln!(
            buf,
            ">\n      <{tag} message=\"{message}\">{}</{tag}>\n    </testcase>",
            xml(&failure_body(result))
        );
    } else if result.status == CheckStatus::Cancelled {
        buf.push_str(">\n      <skipped/>\n    </testcase>\n");
    } else {
        buf.push_str("/>\n");
    }
}

/// JUnit `tests` / `failures` / `errors` / `skipped` attributes; no check
/// counts as an error (that is for broken test harnesses), cancelled ones
/// and allowed failures are skipped
fn counts(summary: &Summary) -> String {
    format!(
        "tests=\"{}\" failures=\"{}\" errors=\"0\" skipped=\"{}\"",
        summary.total,
        summary.failed,
        summary.cancelled + summary.warnings
    )
}

/// `ms` as JUnit seconds (`1.234`)
fn secs(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// `s` without ANSI escapes
fn plain(s: &str) -> Cow<'_, str> {
    crate::color::paint(s, false)
}

/// `s` escaped for XML text and attribute values; chars XML 1.0 forbids
/// (C0 controls except tab / LF / CR, U+FFFE, U+FFFF) are dropped
fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            '\0'..='\x1f' | '\u{fffe}' | '\u{ffff}' => {}
            _ => out.push(c),
        }
    }
    out
}

/// `::group::<name>` opening a collapsible GitHub Actions log group
pub fn github_group(name: &str) -> String {
    format!("::group::{}", github_data(name))
}

/// `::endgroup::` closing [`github_group`]
pub const GITHUB_ENDGROUP: &str = "::endgroup::";

/// `::error title=<id>::<message>` annotation for a failed check
/// (`::warning` for a [`Severity::Warning`] one): status line plus its
/// ANSI-stripped output
pub fn github_annotation(result: &CheckResult, severity: Severity) -> String {
    let command = if severity == Severity::Warning {
        "warning"
    } else {
        "error"
    };
    let status = failure_label(&result.status).unwrap_or("failed");
    let mut message = format!("{} {status}", result.check_id);
    let body = failure_body(result);
    if !body.is_empty() {
        message.push('\n');
        message.push_str(&body);
    }
    format!(
        "::{command} title={}::{}",
        github_property(&result.check_id),
        github_data(&message)
    )
}

/// Workflow command message escaping: `%`, `\r`, `\n`
fn github_data(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Workflow command property escaping: message escaping plus `:` and `,`
fn github_property(s: &str) -> String {
    github_data(s).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckFiles;
    use crate::config::CheckDefinition;
    use std::collections::HashMap;

    fn check(id: &str, group: &str, fix: Option<&str>) -> CheckToRun {
        CheckToRun {
            id: id.into(),
            group: group.into(),
            definition: CheckDefinition {
                name: format!("{id} name"),
                command: "true".into(),
                service: None,
                fix_command: None,
                triggers: None,
                on_demand: false,
                allow_failure: false,
                env: HashMap::new(),
                timeout: None,
                error_pattern: None,
            },
            files: CheckFiles::Files(vec![]),
            resolved_command: "true".into(),
            resolved_fix_command: fix.map(Into::into),
            cache_key: None,
        }
    }

    fn result(id: &str, status: CheckStatus, output: &str) -> CheckResult {
        CheckResult {
            status,
            output: output.into(),
            duration_ms: 1234,
            ..CheckResult::pending(id)
        }
    }

    /// lint (fix cmd) failed, test timed out, fmt cached
    fn fixture() -> (Vec<CheckToRun>, Vec<CheckResult>) {
        let checks = vec![
            check("lint", "quality", Some("cargo fmt")),
            check("fmt", "quality", None),
            check("test", "tests", None),
        ];
        let results = vec![
            result(
                "lint",
                CheckStatus::Failed,
                "\x1b[31merror\x1b[0m: a < b & \"c\"\x01",
            ),
            CheckResult::cached("fmt"),
            result("test", CheckStatus::TimedOut, ""),
        ];
        (checks, results)
    }

    fn report<'a>(checks: &'a [CheckToRun], results: &'a [CheckResult]) -> Report<'a> {
        Report {
            base_ref: "origin/main",
            changed_files: 3,
            duration_ms: 5000,
            checks,
            results,
            summary: Summary::of(checks, results),
        }
    }

    #[test]
    fn test_json_report_fields() {
        let (checks, results) = fixture();
        let doc: serde_json::Value =
            serde_json::from_str(&report(&checks, &results).json()).unwrap();
        assert_eq!(doc["version"], 1);
        assert_eq!(doc["base_ref"], "origin/main");
        assert_eq!(doc["changed_files"], 3);
        assert_eq!(doc["duration_ms"], 5000);
        assert_eq!(
            doc["summary"],
            serde_json::json!({"total": 3, "passed": 1, "failed": 2, "warnings": 0, "cached": 1})
        );
        let lint = &doc["checks"][0];
        assert_eq!(lint["id"], "lint");
        assert_eq!(lint["name"], "lint name");
        assert_eq!(lint["group"], "quality");
        assert_eq!(lint["status"], "failed");
        assert_eq!(lint["duration_ms"], 1234);
        assert_eq!(lint["cached"], false);
        assert_eq!(lint["output"], "error: a < b & \"c\"\x01");
        assert_eq!(lint["error_output"], "");
        assert_eq!(lint["fix_command"], "cargo fmt");
        assert_eq!(doc["checks"][1]["cached"], true);
        assert_eq!(doc["checks"][1]["output"], "");
        assert_eq!(doc["checks"][1]["status"], "passed");
        assert_eq!(doc["checks"][1]["fix_command"], serde_json::Value::Null);
        assert_eq!(doc["checks"][2]["status"], "timed_out");
    }

    /// `warn` (allow_failure) failed, `slow` (allow_failure) timed out, `bad` failed
    fn allowed_fixture() -> (Vec<CheckToRun>, Vec<CheckResult>) {
        let mut checks = vec![
            check("warn", "g", None),
            check("slow", "g", None),
            check("bad", "g", None),
        ];
        checks[0].definition.allow_failure = true;
        checks[1].definition.allow_failure = true;
        let results = vec![
            result("warn", CheckStatus::Failed, "warn out"),
            result("slow", CheckStatus::TimedOut, ""),
            result("bad", CheckStatus::Failed, ""),
        ];
        (checks, results)
    }

    #[test]
    fn test_allowed_failures_count_as_warnings() {
        let (checks, results) = allowed_fixture();
        let doc: serde_json::Value =
            serde_json::from_str(&report(&checks, &results).json()).unwrap();
        assert_eq!(doc["version"], 1);
        assert_eq!(
            doc["summary"],
            serde_json::json!({"total": 3, "passed": 0, "failed": 1, "warnings": 2, "cached": 0})
        );
        assert_eq!(doc["checks"][0]["allow_failure"], true);
        assert_eq!(doc["checks"][0]["status"], "failed");
        assert_eq!(doc["checks"][1]["status"], "timed_out");
        assert_eq!(doc["checks"][2]["allow_failure"], false);
    }

    #[test]
    fn test_junit_allowed_failures_are_skipped() {
        let (checks, results) = allowed_fixture();
        let doc = report(&checks, &results).junit();
        assert!(
            doc.contains(
                "<testsuites name=\"ci-tui\" tests=\"3\" failures=\"1\" errors=\"0\" skipped=\"2\""
            ),
            "{doc}"
        );
        assert!(
            doc.contains(
                "<testsuite name=\"g\" tests=\"3\" failures=\"1\" errors=\"0\" skipped=\"2\""
            ),
            "{doc}"
        );
        assert!(
            doc.contains("<testcase name=\"warn\" classname=\"g\" time=\"1.234\">\n      <skipped message=\"allowed failure: failed\">warn out</skipped>\n    </testcase>"),
            "{doc}"
        );
        assert!(
            doc.contains("<skipped message=\"allowed failure: timed out\"></skipped>"),
            "{doc}"
        );
        assert!(
            doc.contains("<failure message=\"failed\"></failure>"),
            "{doc}"
        );
    }

    #[test]
    fn test_github_warning() {
        let warned = result("w:1", CheckStatus::Failed, "out\n");
        assert_eq!(
            github_annotation(&warned, Severity::Warning),
            "::warning title=w%3A1::w:1 failed%0Aout"
        );
        let timed_out = result("t", CheckStatus::TimedOut, "");
        assert_eq!(
            github_annotation(&timed_out, Severity::Warning),
            "::warning title=t::t timed out"
        );
    }

    #[test]
    fn test_json_report_empty() {
        let doc: serde_json::Value = serde_json::from_str(&report(&[], &[]).json()).unwrap();
        assert_eq!(doc["checks"], serde_json::json!([]));
        assert_eq!(doc["summary"]["total"], 0);
    }

    #[test]
    fn test_junit_report() {
        let (checks, results) = fixture();
        let doc = report(&checks, &results).junit();
        assert!(doc.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"ci-tui\" tests=\"3\" failures=\"2\" errors=\"0\" skipped=\"0\" time=\"5.000\">\n"), "{doc}");
        assert!(
            doc.contains("<testsuite name=\"quality\" tests=\"2\" failures=\"1\" errors=\"0\" skipped=\"0\" time=\"1.234\">"),
            "{doc}"
        );
        assert!(
            doc.contains("<testsuite name=\"tests\" tests=\"1\" failures=\"1\" errors=\"0\" skipped=\"0\" time=\"1.234\">"),
            "{doc}"
        );
        // Escaped, ANSI stripped, \x01 dropped
        assert!(doc.contains(
            "<testcase name=\"lint\" classname=\"quality\" time=\"1.234\">\n      <failure message=\"failed\">error: a &lt; b &amp; &quot;c&quot;</failure>\n    </testcase>"
        ), "{doc}");
        assert!(
            doc.contains("<testcase name=\"fmt\" classname=\"quality\" time=\"0.000\"/>"),
            "{doc}"
        );
        assert!(
            doc.contains("<failure message=\"timed out\"></failure>"),
            "{doc}"
        );
        assert!(doc.ends_with("  </testsuite>\n</testsuites>\n"), "{doc}");
    }

    #[test]
    fn test_junit_cancelled_is_skipped() {
        let checks = [check("c", "g", None)];
        let results = [result("c", CheckStatus::Cancelled, "")];
        let doc = report(&checks, &results).junit();
        assert!(doc.contains("<skipped/>"), "{doc}");
        assert!(
            doc.contains("tests=\"1\" failures=\"0\" errors=\"0\" skipped=\"1\""),
            "{doc}"
        );
    }

    #[test]
    fn test_junit_report_empty() {
        assert_eq!(
            report(&[], &[]).junit(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"ci-tui\" tests=\"0\" failures=\"0\" errors=\"0\" skipped=\"0\" time=\"5.000\">\n</testsuites>\n"
        );
    }

    #[test]
    fn test_xml_escape() {
        assert_eq!(
            xml("<a href='x'>&\"</a>"),
            "&lt;a href=&apos;x&apos;&gt;&amp;&quot;&lt;/a&gt;"
        );
        assert_eq!(xml("a\0b\x07c\x1bd\te\nf\rg\u{fffe}✓"), "abcd\te\nf\rg✓");
    }

    #[test]
    fn test_github_error_escapes() {
        let mut failed = result(
            "a:b,c",
            CheckStatus::Failed,
            "\x1b[31m100%\x1b[0m\r\nline 2\n",
        );
        failed.error_output = "err".into();
        assert_eq!(
            github_annotation(&failed, Severity::Error),
            "::error title=a%3Ab%2Cc::a:b,c failed%0A100%25%0D%0Aline 2%0Aerr"
        );
        let timed_out = result("t", CheckStatus::TimedOut, "");
        assert_eq!(
            github_annotation(&timed_out, Severity::Error),
            "::error title=t::t timed out"
        );
    }

    #[test]
    fn test_github_group() {
        assert_eq!(github_group("LINT 100%\n"), "::group::LINT 100%25%0A");
    }

    #[rstest::rstest]
    #[case::text(Format::Text, false, true)]
    #[case::json_stdout(Format::Json, false, false)]
    #[case::junit_file(Format::Junit, true, true)]
    fn test_text_on_stdout_and_annotate(
        #[case] format: Format,
        #[case] file: bool,
        #[case] text: bool,
    ) {
        let options = ReportOptions {
            format,
            output: file.then(|| "r.xml".into()),
            github: true,
        };
        assert_eq!((options.text_on_stdout(), options.annotate()), (text, text));
        let off = ReportOptions {
            github: false,
            ..options
        };
        assert!(!off.annotate());
    }

    #[test]
    fn test_write_report_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.json");
        let options = ReportOptions {
            format: Format::Json,
            output: Some(path.clone()),
            github: false,
        };
        options.write(&report(&[], &[])).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["version"], 1);
    }

    #[test]
    fn test_write_report_failure_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let options = ReportOptions {
            format: Format::Junit,
            output: Some(dir.path().join("missing/r.xml")),
            github: false,
        };
        let err = options.write(&report(&[], &[])).unwrap_err();
        assert!(
            format!("{err:#}").contains("Failed to write report"),
            "{err:#}"
        );
    }
}
