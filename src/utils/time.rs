//! Time formatting utilities for CI-TUI.
//!
//! Provides functions for formatting durations in human-readable format.

/// Format duration in human-readable format.
///
/// Returns a formatted string representing the duration in milliseconds:
/// - `< 1000ms`: Shows as "Xms" (e.g., "500ms")
/// - `< 60s`: Shows as "X.Xs" with one decimal place (e.g., "1.5s", "45.2s")
/// - `>= 60s`: Shows as "Xm Ys" format (e.g., "1m 30s", "2m 15s")
///
/// # Arguments
///
/// * `ms` - Duration in milliseconds
///
/// # Examples
///
/// ```
/// # use ci_tui::utils::time::format;
/// assert_eq!(format(500), "500ms");
/// assert_eq!(format(1500), "1.5s");
/// assert_eq!(format(90000), "1m 30s");
/// ```
pub(crate) fn format(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        format!("{}m {}s", mins, secs)
    }
}

/// Format a std::time::Duration to human-readable string.
///
/// Returns:
/// - "Xs" for <1 minute
/// - "Xm Ys" for <1 hour
/// - "Xh Ym" for >=1 hour
pub(crate) fn format_from_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        let mins = secs / 60;
        let remaining_secs = secs % 60;
        format!("{}m {}s", mins, remaining_secs)
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{}h {}m", hours, mins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(0, "0ms")]
    #[case(500, "500ms")]
    #[case(999, "999ms")]
    #[case(1000, "1.0s")]
    #[case(1500, "1.5s")]
    #[case(59999, "60.0s")]
    #[case(60000, "1m 0s")]
    #[case(90000, "1m 30s")]
    #[case(3600000, "60m 0s")]
    fn test_format_duration(#[case] ms: u64, #[case] expected: &str) {
        assert_eq!(format(ms), expected);
    }

    #[rstest]
    #[case(0, "0s")]
    #[case(30, "30s")]
    #[case(59, "59s")]
    #[case(60, "1m 0s")]
    #[case(90, "1m 30s")]
    #[case(3599, "59m 59s")]
    #[case(3600, "1h 0m")]
    #[case(7200, "2h 0m")]
    fn test_format_from_duration(#[case] secs: u64, #[case] expected: &str) {
        assert_eq!(
            format_from_duration(std::time::Duration::from_secs(secs)),
            expected
        );
    }
}
