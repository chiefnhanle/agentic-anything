//! Rule tables as code (architecture §6.4). Every table only **raises** effects; each emitted signal records the
//! versioned rule id in `source`.

use crate::{effects, pb};

pub const CLI_VERBS: &str = "rule:cli-verbs@1";
pub const CLI_OPTIONS: &str = "rule:cli-options@1";
pub const CLI_ANYTHING: &str = "rule:cli-anything@1";
pub const EXCLUSIONS: &str = "rule:exclusions@1";
pub const COMPILE: &str = "rule:compile@1";

/// `cli-verbs@1`: leaf words that raise `mutates_target`.
pub const MUTATING_VERBS: &[&str] = &[
    "add", "new", "create", "set", "modify", "rename", "apply", "save", "import", "open", "insert", "update", "commit",
    "checkout", "merge", "rebase", "push", "pull",
];
/// `cli-verbs@1`: leaf words that raise `may_discard_data`.
pub const DISCARDING_VERBS: &[&str] = &[
    "remove",
    "delete",
    "rm",
    "clear",
    "reset",
    "purge",
    "drop",
    "prune",
    "wipe",
    "overwrite",
];
/// `cli-verbs@1`: leaf words that raise `external_side_effect` + `accesses_network`.
pub const EXTERNAL_VERBS: &[&str] = &["send", "publish", "post", "pay", "upload", "push"];
/// Read-ish words: emit **nothing**; only a hint in the withheld message.
pub const READISH_VERBS: &[&str] = &["list", "info", "show", "get", "status", "log", "presets"];

/// `cli-options@1` flags/parameter names that designate an output path.
pub const OUTPUT_FLAGS: &[&str] = &["-o", "--output", "--out"];
pub const OUTPUT_NAMES: &[&str] = &["output", "out", "output_path"];
/// `cli-options@1` flags that raise `may_discard_data` through a branch.
pub const DISCARD_FLAGS: &[&str] = &["--force", "-f", "--yes", "--hard", "--clobber", "--overwrite", "-D"];
/// `cli-options@1` flags that launch a browser (the parameter is excluded by default).
pub const BROWSER_FLAGS: &[&str] = &["--web", "--browser"];
/// `cli-options@1` flags whose value switches authority (an unresolved branch: `MIXED_AUTHORITY`).
pub const MIXED_AUTHORITY_FLAGS: &[&str] = &["-X", "--method"];

/// Fields the binding-intrinsic `compile@1` rule declares FALSE for a headless `cli_argv` binding.
pub const HEADLESS_FIELDS: &[&str] = &[
    "changes_ui_state",
    "sends_input",
    "reads_screen",
    "reads_clipboard",
    "writes_clipboard",
    "attaches_process",
    effects::MUTATES_SESSION,
];

pub fn signal(
    field: &str,
    value: pb::Tri,
    confidence: pb::Confidence,
    source: &str,
    rationale: impl Into<String>,
) -> pb::EffectSignal {
    pb::EffectSignal {
        field: field.to_string(),
        value: value as i32,
        confidence: confidence as i32,
        source: source.to_string(),
        rationale: rationale.into(),
        ..Default::default()
    }
}

/// Lowercase words of a native leaf token (`add-paragraph` -> `[add, paragraph]`).
pub fn leaf_words(leaf: &str) -> Vec<String> {
    leaf.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// `cli-verbs@1` over every word of the leaf token.
pub fn verb_signals(leaf: &str) -> Vec<pb::EffectSignal> {
    let mut out = Vec::new();
    let raise = |out: &mut Vec<pb::EffectSignal>, field: &str, w: &str| {
        out.push(signal(
            field,
            pb::Tri::True,
            pb::Confidence::Inferred,
            CLI_VERBS,
            format!("leaf verb '{w}'"),
        ));
    };
    for w in leaf_words(leaf) {
        if MUTATING_VERBS.contains(&w.as_str()) {
            raise(&mut out, "mutates_target", &w);
        }
        if DISCARDING_VERBS.contains(&w.as_str()) {
            raise(&mut out, "may_discard_data", &w);
        }
        if EXTERNAL_VERBS.contains(&w.as_str()) {
            raise(&mut out, "external_side_effect", &w);
            raise(&mut out, "accesses_network", &w);
        }
    }
    out
}

/// The read-ish word of a leaf token, if any (used only in hints).
pub fn readish(leaf: &str) -> Option<String> {
    leaf_words(leaf)
        .into_iter()
        .find(|w| READISH_VERBS.contains(&w.as_str()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionRule {
    /// `writes_filesystem` TRUE + a `WRITE_PATH` scope on the parameter.
    OutputPath,
    /// A branch raising `may_discard_data` when the parameter is present and truthy.
    Discard,
    /// A browser launcher: the parameter is excluded from the schema.
    Browser,
    /// An unresolved branch: `MIXED_AUTHORITY` until the overlay resolves it.
    MixedAuthority,
}

/// The last segment of a JSON pointer (`/output_path` -> `output_path`).
pub fn pointer_name(pointer: &str) -> &str {
    pointer.rsplit('/').next().unwrap_or(pointer)
}

/// `cli-options@1` for one argument mapping.
pub fn option_rule(arg: &pb::ArgMapping) -> Option<OptionRule> {
    let flag = arg.flag.as_str();
    let name = pointer_name(&arg.pointer);
    if MIXED_AUTHORITY_FLAGS.contains(&flag) || name == "method" {
        Some(OptionRule::MixedAuthority)
    } else if BROWSER_FLAGS.contains(&flag) || name == "web" || name == "browser" {
        Some(OptionRule::Browser)
    } else if OUTPUT_FLAGS.contains(&flag) || OUTPUT_NAMES.contains(&name) {
        Some(OptionRule::OutputPath)
    } else if DISCARD_FLAGS.contains(&flag) || ["force", "yes", "hard", "clobber", "overwrite"].contains(&name) {
        Some(OptionRule::Discard)
    } else {
        None
    }
}

/// Whether `compile` must drop this parameter from the schema and binding (browser launchers).
pub fn excluded_param(arg: &pb::ArgMapping) -> bool {
    option_rule(arg) == Some(OptionRule::Browser)
}

/// Whether a binding is a headless `cli_argv` binding the engine runs in an owned process group.
pub fn headless_cli(target: &pb::Target, binding: Option<&pb::Binding>) -> bool {
    let Some(pb::binding::Kind::Cli(cli)) = binding.and_then(|b| b.kind.as_ref()) else {
        return false;
    };
    cli.kill_scope == pb::KillScope::OwnedGroup as i32 && target.kind != pb::TargetKind::InstalledApp as i32
}

/// `compile@1` binding-intrinsic rule: a headless `cli_argv` binding runs with no TTY, stdin `/dev/null`, a private
/// HOME/TMPDIR and a scrubbed env in an engine-owned process group, so the binding contract gives it no UI, input,
/// screen, clipboard or foreign-process surface. DECLARED FALSE (lanes can never lower these fields; ADR 0011).
pub fn binding_intrinsic_signals(target: &pb::Target, binding: Option<&pb::Binding>) -> Vec<pb::EffectSignal> {
    if !headless_cli(target, binding) {
        return Vec::new();
    }
    HEADLESS_FIELDS
        .iter()
        .map(|f| {
            signal(
                f,
                pb::Tri::False,
                pb::Confidence::Declared,
                COMPILE,
                "binding-intrinsic: headless cli_argv in an owned process group (no TTY, stdin /dev/null, private \
                 HOME); no UI, input, screen, clipboard or attach surface",
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_raise_only() {
        let s = verb_signals("add-paragraph");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].field, "mutates_target");
        assert_eq!(verb_signals("push").len(), 3);
        assert!(verb_signals("list").is_empty());
        assert_eq!(readish("list-sheets").as_deref(), Some("list"));
        assert!(verb_signals("status").iter().all(|s| s.value == pb::Tri::True as i32));
    }

    #[test]
    fn option_rules() {
        let a = |flag: &str, ptr: &str| pb::ArgMapping {
            flag: flag.into(),
            pointer: ptr.into(),
            ..Default::default()
        };
        assert_eq!(option_rule(&a("--output", "/output")), Some(OptionRule::OutputPath));
        assert_eq!(option_rule(&a("", "/output_path")), Some(OptionRule::OutputPath));
        assert_eq!(option_rule(&a("-D", "/d")), Some(OptionRule::Discard));
        assert_eq!(option_rule(&a("-d", "/d")), None);
        assert_eq!(option_rule(&a("--web", "/web")), Some(OptionRule::Browser));
        assert_eq!(option_rule(&a("--method", "/method")), Some(OptionRule::MixedAuthority));
        assert_eq!(option_rule(&a("--untracked-files", "/untracked_files")), None);
    }
}
