//! Process exit codes: why a run failed, for scripts and CI callers.
//!
//! | code | meaning                                          |
//! |------|--------------------------------------------------|
//! | 0    | all selected checks passed (or nothing to do)    |
//! | 1    | at least one check / fix command failed          |
//! | 2    | config or usage error                            |
//! | 3    | git / environment error (e.g. Docker unreachable)|
//! | 130  | interrupted (Ctrl-C)                             |

/// All selected checks passed
pub const SUCCESS: i32 = 0;
/// At least one check (or fix command) failed
pub const CHECKS_FAILED: i32 = 1;
/// Config file missing / invalid, or bad flag value (clap's usage code)
pub const CONFIG_ERROR: i32 = 2;
/// Git change detection failed, Docker unreachable, or other runtime error
pub const ENV_ERROR: i32 = 3;
/// Ctrl-C (128 + SIGINT)
pub const INTERRUPTED: i32 = 130;

/// Marks an error as a config error ([`CONFIG_ERROR`]). Transparent: the
/// message and cause chain are the wrapped error's.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ConfigError(#[from] pub anyhow::Error);

/// Exit code for an error escaping the run: [`CONFIG_ERROR`] if it is (or
/// wraps) a [`ConfigError`], else [`ENV_ERROR`].
pub fn code_for(err: &anyhow::Error) -> i32 {
    if err.chain().any(|e| e.is::<ConfigError>()) {
        CONFIG_ERROR
    } else {
        ENV_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn config_error_maps_to_config_code() {
        let err = anyhow::Error::from(ConfigError(anyhow::anyhow!("bad yaml")));
        assert_eq!(code_for(&err), CONFIG_ERROR);
    }

    #[test]
    fn config_error_under_context_maps_to_config_code() {
        let err: anyhow::Error = Err::<(), _>(ConfigError(anyhow::anyhow!("bad yaml")))
            .context("loading")
            .unwrap_err();
        assert_eq!(code_for(&err), CONFIG_ERROR);
    }

    #[test]
    fn other_errors_map_to_env_code() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        assert_eq!(code_for(&anyhow::Error::from(io)), ENV_ERROR);
        assert_eq!(code_for(&anyhow::anyhow!("docker down")), ENV_ERROR);
    }

    #[test]
    fn config_error_keeps_message_and_chain() {
        let inner = anyhow::anyhow!("root cause").context("Failed to parse ci.yaml");
        let err = anyhow::Error::from(ConfigError(inner));
        assert_eq!(
            format!("{err:#}"),
            "Failed to parse ci.yaml: root cause",
            "transparent wrapper adds nothing"
        );
    }

    #[test]
    fn codes_are_distinct() {
        let codes = [SUCCESS, CHECKS_FAILED, CONFIG_ERROR, ENV_ERROR, INTERRUPTED];
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len());
    }
}
