use std::env;
use std::process::Command;

fn main() {
    // Try environment variable first (set by Docker build args), then fall back to git command
    let git_hash = env::var("CI_TUI_GIT_HASH")
        .ok()
        .filter(|s| !s.is_empty() && s != "unknown")
        .or_else(|| {
            Command::new("git")
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
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Try environment variable first (set by Docker build args), then fall back to date command
    let build_date = env::var("CI_TUI_BUILD_DATE")
        .ok()
        .filter(|s| !s.is_empty() && s != "unknown")
        .or_else(|| {
            Command::new("date")
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
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Set environment variables for compile-time access
    println!("cargo:rustc-env=CI_TUI_GIT_HASH={}", git_hash);
    println!("cargo:rustc-env=CI_TUI_BUILD_DATE={}", build_date);

    // Rebuild when git commits change
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
