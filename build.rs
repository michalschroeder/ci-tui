use std::process::Command;

fn main() {
    // Capture git commit hash (short, 7 chars)
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Capture build date in YYYY-MM-DD format using date command
    let build_date = Command::new("date")
        .args(["+%Y-%m-%d"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Set environment variables for compile-time access
    println!("cargo:rustc-env=CI_TUI_GIT_HASH={}", git_hash);
    println!("cargo:rustc-env=CI_TUI_BUILD_DATE={}", build_date);

    // Rebuild when git commits change
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
