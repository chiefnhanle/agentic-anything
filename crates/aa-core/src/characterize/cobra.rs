//! Cobra conventions (`cobra-complete@1` candidates).
//!
//! Cobra (gh, kubectl) reports usage errors with exit 1 and `unknown flag` on stderr, which is also its runtime
//! error exit code, so L1 needs the stderr marker as well (gh, observed 2026-11-05).

/// Cobra's usage-error exit code.
pub const USAGE_EXIT_CODE: i32 = 1;
/// The stderr marker that distinguishes a Cobra usage error from a runtime error.
pub const USAGE_STDERR_MARKER: &str = "unknown flag";

/// Cobra's generated helper commands, which are never capabilities of the target itself.
pub fn is_builtin(native_path: &[String]) -> bool {
    matches!(
        native_path.first().map(String::as_str),
        Some("__complete" | "__completeNoDesc")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins() {
        assert!(is_builtin(&["__complete".into()]));
        assert!(!is_builtin(&["alias".into(), "list".into()]));
    }
}
