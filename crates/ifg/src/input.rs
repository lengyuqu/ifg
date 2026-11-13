//! Non-interactive mode: `--no-input` or `IFG_NO_INPUT=1`.
//!
//! With the switch on, ifg never waits for a human. Anything that would ask
//! a question (a confirmation, the token prompt, `$EDITOR`, the pager) fails
//! at once with a usage error (exit 2) that names the flag to pass instead,
//! and git subprocesses are told not to prompt. The switch is not `--yes`:
//! it never answers a confirmation on the caller's behalf.

use std::sync::atomic::{AtomicBool, Ordering};

/// Environment variable that turns the switch on (`1` or `true`).
pub const ENV_VAR: &str = "IFG_NO_INPUT";

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Decide the switch: the flag wins, then the environment, then off. Only
/// `1` and `true` (any case, surrounding spaces ignored) count as on.
pub fn resolve(flag: bool, env: Option<&str>) -> bool {
    flag || env
        .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true"))
}

/// Record the decision for the rest of the process. `main` calls this once,
/// right after parsing.
pub fn set(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether the switch is on.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn flag_or_truthy_environment_turns_the_switch_on() {
        assert!(resolve(true, None));
        assert!(
            resolve(true, Some("0")),
            "the flag wins over the environment"
        );
        assert!(resolve(false, Some("1")));
        assert!(resolve(false, Some("true")));
        assert!(resolve(false, Some(" TRUE ")));
    }

    #[test]
    fn default_and_other_environment_values_leave_it_off() {
        assert!(!resolve(false, None));
        assert!(!resolve(false, Some("")));
        assert!(!resolve(false, Some("0")));
        assert!(!resolve(false, Some("false")));
        assert!(!resolve(false, Some("yes")));
    }
}
