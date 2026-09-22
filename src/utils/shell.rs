//! Shell command building helpers.

/// Quote a single argument for POSIX shells.
///
/// Arguments made only of safe characters are returned unchanged; anything
/// else is single-quoted with embedded `'` escaped as `'\''`.
pub(crate) fn quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./-+:@,=%^".contains(c));
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// Replace `{files}` with the shell-quoted, space-separated paths, then trim.
pub(crate) fn expand_files(command: &str, files: &[&str]) -> String {
    let files_str = files.iter().map(|f| quote(f)).collect::<Vec<_>>().join(" ");
    command.replace("{files}", &files_str).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_leaves_safe_paths_unchanged() {
        assert_eq!(quote("src/foo_bar-1.rs"), "src/foo_bar-1.rs");
    }

    #[test]
    fn quote_wraps_shell_metacharacters() {
        assert_eq!(quote("(weird).rs"), "'(weird).rs'");
        assert_eq!(quote("a b.rs"), "'a b.rs'");
        assert_eq!(quote("$(curl x|sh).php"), "'$(curl x|sh).php'");
        assert_eq!(quote("it's.rs"), "'it'\\''s.rs'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn expand_files_quotes_each_path_and_trims() {
        assert_eq!(
            expand_files("run {files}", &["a.rs", "b c.rs"]),
            "run a.rs 'b c.rs'"
        );
        assert_eq!(expand_files("run {files} ", &[]), "run");
    }
}
