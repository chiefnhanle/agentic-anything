//! Click / CLI-Anything conventions: `cli-anything@1` (architecture §6.4, §11).
//!
//! - **Autosave:** a CLI-Anything command that mutates its project autosaves it, so `mutates_target` with a state
//!   handle raises `writes_filesystem`.
//! - **`repl`** and bare groups are `INTERACTIVE` (never served one-shot).
//! - **`{error, type}`** on stdout is the error convention of every CLI-Anything harness.

use super::rules::{self, CLI_ANYTHING};
use crate::pb;

/// Click's usage-error exit code (L1 expects it).
pub const USAGE_EXIT_CODE: i32 = 2;

/// Whether a candidate is REPL/TTY-only.
pub fn interactive(native_path: &[String], flagged: bool) -> bool {
    flagged || native_path.last().is_some_and(|l| l.eq_ignore_ascii_case("repl"))
}

/// Whether the target follows the CLI-Anything conventions.
pub fn is_cli_anything(target: &pb::Target) -> bool {
    target.kind == pb::TargetKind::CliAnythingHarness as i32 || target.id.starts_with("cli-anything.")
}

/// The error convention compile applies when the harvester left it unspecified.
pub fn default_errors(target: &pb::Target) -> pb::ErrorConvention {
    if is_cli_anything(target) {
        pb::ErrorConvention::CliAnythingJson
    } else {
        pb::ErrorConvention::ExitCode
    }
}

/// The autosave rule: `mutates_target` TRUE (in `signals`) plus a state handle raises `writes_filesystem`.
pub fn autosave(cli: Option<&pb::CliArgvBinding>, signals: &[pb::EffectSignal]) -> Option<pb::EffectSignal> {
    let has_handle = cli.is_some_and(|c| !c.state_handle_flag.is_empty());
    let mutates = signals
        .iter()
        .any(|s| s.field == "mutates_target" && s.value == pb::Tri::True as i32);
    (has_handle && mutates).then(|| {
        rules::signal(
            "writes_filesystem",
            pb::Tri::True,
            pb::Confidence::Inferred,
            CLI_ANYTHING,
            "autosave: a mutating command rewrites its state handle (--project file)",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repl_and_autosave() {
        assert!(interactive(&["repl".into()], false));
        assert!(!interactive(&["writer".into(), "list".into()], false));
        let cli = pb::CliArgvBinding {
            state_handle_flag: "--project".into(),
            ..Default::default()
        };
        let m = rules::verb_signals("add");
        assert!(autosave(Some(&cli), &m).is_some());
        assert!(autosave(Some(&pb::CliArgvBinding::default()), &m).is_none());
        assert!(autosave(Some(&cli), &[]).is_none());
    }
}
