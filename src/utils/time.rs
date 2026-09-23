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
/// ```ignore
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

/// Parse a config duration: a positive integer followed by `s`, `m` or `h`
/// (e.g. `30s`, `10m`, `1h`).
///
/// Hand-rolled instead of `humantime`: config only needs these three units and
/// one fewer dependency beats compound forms (`1h30m`) nobody asked for.
pub(crate) fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    let invalid = || format!("invalid duration '{s}': expected <n>s, <n>m or <n>h (e.g. 30s, 10m)");
    let unit = s.chars().last().ok_or_else(invalid)?;
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        _ => return Err(invalid()),
    };
    let digits = &s[..s.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let n: u64 = digits.parse().map_err(|_| invalid())?;
    if n == 0 {
        return Err(format!("invalid duration '{s}': must be greater than zero"));
    }
    n.checked_mul(multiplier)
        .map(std::time::Duration::from_secs)
        .ok_or_else(invalid)
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

    #[rstest]
    #[case("1s", 1)]
    #[case("30s", 30)]
    #[case("10m", 600)]
    #[case("1h", 3600)]
    fn test_parse_duration_valid(#[case] input: &str, #[case] secs: u64) {
        assert_eq!(
            parse_duration(input),
            Ok(std::time::Duration::from_secs(secs))
        );
    }

    #[rstest]
    #[case("")]
    #[case("abc")]
    #[case("10x")]
    #[case("0s")]
    #[case("s")]
    #[case("-1s")]
    #[case("1.5m")]
    #[case(" 1s")]
    #[case("99999999999999999999h")]
    fn test_parse_duration_invalid(#[case] input: &str) {
        let err = parse_duration(input).unwrap_err();
        assert!(err.contains("duration"), "got: {err}");
    }
}
