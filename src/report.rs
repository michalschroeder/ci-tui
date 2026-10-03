//! Machine-readable simple-mode output (#147): `--format json|junit`
//! reports (to stdout, or `--output <file>` next to the text output) and
//! GitHub Actions workflow commands (`::group::` / `::error`) when
//! `GITHUB_ACTIONS=true` and stdout carries the text output.
//!
//! Reports cover the checks simple mode ran (cached ones included, on-demand
//! ones never run there). Output text is ANSI-stripped; JUnit is hand-rolled
//! XML (escaped, XML-invalid control chars dropped) to avoid a dependency.

use crate::checks::CheckToRun;
use crate::runner::{CheckResult, CheckStatus};
use anyhow::{Context, Result};
use serde::Serialize;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;

/// Env var GitHub Actions sets to `true` on its runners
pub const GITHUB_ACTIONS_ENV: &str = "GITHUB_ACTIONS";

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

/// Whether GitHub Actions annotations are on: `GITHUB_ACTIONS` is `true`
pub fn github_actions(env: Option<&OsStr>) -> bool {
    env == Some(OsStr::new("true"))
}

/// Where simple mode writes what
#[derive(Debug, Clone, Default)]
pub struct ReportOptions {
    /// Report format; `Text` writes no report
    pub format: Format,
    /// Report file (`--output`); `None` puts the report on stdout instead of text
    pub output: Option<PathBuf>,
    /// Running under GitHub Actions ([`github_actions`])
    pub github: bool,
}

impl ReportOptions {
    /// Options for `--format` / `--output`, annotating when the
    /// `GITHUB_ACTIONS` env var is `true`
    pub fn new(format: Format, output: Option<PathBuf>) -> Self {
        let github = github_actions(std::env::var_os(GITHUB_ACTIONS_ENV).as_deref());
        Self {
            format,
            output,
            github,
        }
    }

    /// Stdout carries the text output (not a report document)
    pub fn text_on_stdout(&self) -> bool {
        self.format == Format::Text || self.output.is_some()
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
                // Flushed here: simple mode may `process::exit` right after
                let mut out = std::io::stdout().lock();
                out.write_all(doc.as_bytes())
                    .and_then(|()| out.flush())
                    .context("Failed to write report to stdout")
            }
        }
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
}

#[derive(Serialize)]
struct JsonReport<'a> {
    version: u32,
    base_ref: &'a str,
    changed_files: usize,
    duration_ms: u64,
    summary: Summary,
    checks: Vec<JsonCheck<'a>>,
}

#[derive(Serialize)]
struct Summary {
    total: usize,
    passed: usize,
    failed: usize,
    cached: usize,
}

#[derive(Serialize)]
struct JsonCheck<'a> {
    id: &'a str,
    name: &'a str,
    group: &'a str,
    status: &'a CheckStatus,
    duration_ms: u64,
    cached: bool,
    output: String,
    error_output: String,
    fix_command: Option<&'a str>,
}

impl Report<'_> {
    /// The selected check `result` belongs to
    fn check(&self, result: &CheckResult) -> Option<&CheckToRun> {
        self.checks.iter().find(|c| c.id() == result.check_id)
    }

    /// Group key of `result`'s check (empty if unknown)
    fn group(&self, result: &CheckResult) -> &str {
        self.check(result).map_or("", |c| c.group())
    }

    fn summary(&self) -> Summary {
        let count = |f: fn(&CheckResult) -> bool| self.results.iter().filter(|r| f(r)).count();
        Summary {
            total: self.results.len(),
            passed: count(|r| r.status == CheckStatus::Passed),
            failed: count(|r| r.status.is_failure()),
            cached: count(|r| r.cached),
        }
    }

    /// Pretty-printed JSON document, newline-terminated
    pub fn json(&self) -> String {
        let checks = self.results.iter().map(|r| {
            let check = self.check(r);
            JsonCheck {
                id: &r.check_id,
                name: check.map_or(&r.check_id, |c| c.name()),
                group: self.group(r),
                status: &r.status,
                duration_ms: r.duration_ms,
                cached: r.cached,
                output: plain(&r.output),
                error_output: plain(&r.error_output),
                fix_command: check.and_then(|c| c.resolved_fix_command.as_deref()),
            }
        });
        let report = JsonReport {
            version: JSON_VERSION,
            base_ref: self.base_ref,
            changed_files: self.changed_files,
            duration_ms: self.duration_ms,
            summary: self.summary(),
            checks: checks.collect(),
        };
        let mut doc = serde_json::to_string_pretty(&report).expect("report serializes");
        doc.push('\n');
        doc
    }

    /// JUnit XML document: one `<testsuite>` per group (in run order), one
    /// `<testcase>` per result; failures / timeouts carry `<failure>` with
    /// the output, cancelled checks `<skipped/>`
    pub fn junit(&self) -> String {
        let mut groups: indexmap::IndexMap<&str, Vec<&CheckResult>> = indexmap::IndexMap::new();
        for result in self.results {
            groups.entry(self.group(result)).or_default().push(result);
        }
        let summary = self.summary();
        let mut buf = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        let _ = writeln!(
            buf,
            "<testsuites name=\"ci-tui\" tests=\"{}\" failures=\"{}\" time=\"{}\">",
            summary.total,
            summary.failed,
            secs(self.duration_ms)
        );
        for (group, results) in groups {
            write_testsuite(&mut buf, group, &results);
        }
        buf.push_str("</testsuites>\n");
        buf
    }
}

/// One `<testsuite>` element for `group`'s `results`; `time` sums them
fn write_testsuite(buf: &mut String, group: &str, results: &[&CheckResult]) {
    let failures = results.iter().filter(|r| r.status.is_failure()).count();
    let time: u64 = results.iter().map(|r| r.duration_ms).sum();
    let _ = writeln!(
        buf,
        "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" time=\"{}\">",
        xml(group),
        results.len(),
        secs(time)
    );
    for result in results {
        write_testcase(buf, group, result);
    }
    buf.push_str("  </testsuite>\n");
}

/// One `<testcase>` element for `result` in `group`
fn write_testcase(buf: &mut String, group: &str, result: &CheckResult) {
    let _ = write!(
        buf,
        "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\"",
        xml(&result.check_id),
        xml(group),
        secs(result.duration_ms)
    );
    match result.status {
        CheckStatus::Failed | CheckStatus::TimedOut => {
            let message = match result.status {
                CheckStatus::TimedOut => "timed out",
                _ => "failed",
            };
            let body = [result.output.as_str(), result.error_output.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let _ = writeln!(
                buf,
                ">\n      <failure message=\"{message}\">{}</failure>\n    </testcase>",
                xml(&plain(&body))
            );
        }
        CheckStatus::Cancelled => buf.push_str(">\n      <skipped/>\n    </testcase>\n"),
        _ => buf.push_str("/>\n"),
    }
}

/// `ms` as JUnit seconds (`1.234`)
fn secs(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// `s` without ANSI escapes
fn plain(s: &str) -> String {
    crate::color::paint(s, false).into_owned()
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

/// `::error title=<id>::<message>` annotation for a failed check: status
/// line plus its ANSI-stripped output
pub fn github_error(result: &CheckResult) -> String {
    let status = match result.status {
        CheckStatus::TimedOut => "timed out",
        _ => "failed",
    };
    let mut message = format!("{} {status}", result.check_id);
    for out in [&result.output, &result.error_output] {
        let out = plain(out);
        let out = out.trim_end();
        if !out.is_empty() {
            message.push('\n');
            message.push_str(out);
        }
    }
    format!(
        "::error title={}::{}",
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
                container: None,
                fix_command: None,
                triggers: None,
                on_demand: false,
                env: HashMap::new(),
                timeout: None,
                error_pattern: None,
            },
            service: None,
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
            serde_json::json!({"total": 3, "passed": 1, "failed": 2, "cached": 1})
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
        assert_eq!(doc["checks"][1]["status"], "passed");
        assert_eq!(doc["checks"][1]["fix_command"], serde_json::Value::Null);
        assert_eq!(doc["checks"][2]["status"], "timed_out");
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
        assert!(doc.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"ci-tui\" tests=\"3\" failures=\"2\" time=\"5.000\">\n"), "{doc}");
        assert!(
            doc.contains("<testsuite name=\"quality\" tests=\"2\" failures=\"1\" time=\"1.234\">"),
            "{doc}"
        );
        assert!(
            doc.contains("<testsuite name=\"tests\" tests=\"1\" failures=\"1\" time=\"1.234\">"),
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
        assert!(doc.contains("failures=\"0\""), "{doc}");
    }

    #[test]
    fn test_junit_report_empty() {
        assert_eq!(
            report(&[], &[]).junit(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"ci-tui\" tests=\"0\" failures=\"0\" time=\"5.000\">\n</testsuites>\n"
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
            github_error(&failed),
            "::error title=a%3Ab%2Cc::a:b,c failed%0A100%25%0D%0Aline 2%0Aerr"
        );
        let timed_out = result("t", CheckStatus::TimedOut, "");
        assert_eq!(github_error(&timed_out), "::error title=t::t timed out");
    }

    #[test]
    fn test_github_group() {
        assert_eq!(github_group("LINT 100%\n"), "::group::LINT 100%25%0A");
    }

    #[rstest::rstest]
    #[case::unset(None, false)]
    #[case::true_(Some("true"), true)]
    #[case::other(Some("1"), false)]
    #[case::empty(Some(""), false)]
    fn test_github_actions(#[case] env: Option<&str>, #[case] expected: bool) {
        assert_eq!(github_actions(env.map(OsStr::new)), expected);
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
