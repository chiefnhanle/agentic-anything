//! Characterization: rule tables turn a harvested candidate into effect signals, branches, scopes and
//! withheld reasons (architecture §6.4).
//!
//! Everything here **raises** only. Harvester-native FALSE hints are demoted to INFERRED (recorded and ignored),
//! overlay effect entries are raises, and lowering is left to lanes, trusted declarations and the operator.

pub mod click;
pub mod cobra;
pub mod help_text;
pub mod rules;

use crate::{effects, pb};
use rules::{OptionRule, signal};

/// `{"cli-verbs":"1","cli-options":"1","cli-anything":"1","exclusions":"1","compile":"1"}`.
pub fn rule_versions() -> std::collections::BTreeMap<String, String> {
    ["cli-verbs", "cli-options", "cli-anything", "exclusions", "compile"]
        .into_iter()
        .map(|k| (k.to_string(), "1".to_string()))
        .collect()
}

pub struct Characterized {
    pub signals: Vec<pb::EffectSignal>,
    pub branches: Vec<pb::EffectBranch>,
    pub scopes: Vec<pb::ScopeParam>,
    pub withheld: Vec<pb::Withheld>,
}

/// Source tag of overlay `capabilities.<id>.effects` raises.
pub const OVERLAY_EFFECTS: &str = "overlay:effects";
/// Source tag of overlay `capabilities.<id>.branches` raises.
pub const OVERLAY_BRANCHES: &str = "overlay:branches";

pub(crate) fn cli_of(cap: &pb::Capability) -> Option<&pb::CliArgvBinding> {
    match cap.binding.as_ref()?.kind.as_ref()? {
        pb::binding::Kind::Cli(c) => Some(c),
        _ => None,
    }
}

/// Harvester-native hints may only raise: a FALSE on a target field (or a TRUE on `idempotent`) is capped at
/// INFERRED so it is recorded but never counts.
fn demote_native(s: &pb::EffectSignal) -> pb::EffectSignal {
    let mut s = s.clone();
    let lowering = (s.field == effects::IDEMPOTENT && s.value == pb::Tri::True as i32)
        || (s.field != effects::IDEMPOTENT && s.value == pb::Tri::False as i32);
    let conf = pb::Confidence::try_from(s.confidence).unwrap_or(pb::Confidence::Proposed);
    if lowering && conf != pb::Confidence::Proposed {
        s.confidence = pb::Confidence::Inferred as i32;
    } else if !lowering
        && effects::lattice_rank(&s.field, effects::tri_of(s.value)) > 0
        && conf > pb::Confidence::Declared
    {
        s.confidence = pb::Confidence::Declared as i32;
    }
    if s.source.is_empty() {
        s.source = "harvest".into();
    }
    s
}

/// An overlay `field: bool` entry as a raise-only effect value (`None` = would lower: ignored).
fn overlay_raise(field: &str, v: bool) -> Option<pb::Tri> {
    match (field == effects::IDEMPOTENT, v) {
        (false, true) => Some(pb::Tri::True),
        (true, false) => Some(pb::Tri::False),
        _ => None,
    }
}

fn withheld(reason: pb::WithheldReason, hint: impl Into<String>) -> pb::Withheld {
    pb::Withheld {
        reason: reason as i32,
        hint: hint.into(),
        evidence_ids: Vec::new(),
    }
}

fn text_of(t: &Option<pb::TargetText>) -> &str {
    t.as_ref().map_or("", |t| t.text.as_str())
}

/// Deterministic order + dedup for signals.
pub(crate) fn sort_signals(v: &mut Vec<pb::EffectSignal>) {
    v.sort_by(|a, b| {
        (
            &a.field,
            &a.source,
            a.value,
            a.confidence,
            a.isolation,
            &a.rationale,
            &a.evidence_ids,
        )
            .cmp(&(
                &b.field,
                &b.source,
                b.value,
                b.confidence,
                b.isolation,
                &b.rationale,
                &b.evidence_ids,
            ))
    });
    v.dedup();
}

pub fn characterize(
    target: &pb::Target,
    cand: &crate::api::Candidate,
    overlay: &crate::manifest::Overlay,
) -> Characterized {
    let cap = &cand.capability;
    let cli = cli_of(cap);
    let overlay_cap = overlay.capabilities.get(&cap.id);
    let leaf = cand
        .native_path
        .last()
        .cloned()
        .unwrap_or_else(|| cap.id.rsplit('.').next().unwrap_or_default().to_string());
    let mut out = Characterized {
        signals: cand.native_signals.iter().map(demote_native).collect(),
        branches: Vec::new(),
        scopes: Vec::new(),
        withheld: Vec::new(),
    };

    // cli-verbs@1 on the leaf token.
    out.signals.extend(rules::verb_signals(&leaf));

    // cli-options@1 on the mapped arguments.
    let overlay_branch_pointers: Vec<&str> = overlay_cap
        .map(|c| c.branches.iter().map(|b| b.pointer.as_str()).collect())
        .unwrap_or_default();
    for arg in cli.map(|c| c.args.as_slice()).unwrap_or_default() {
        let label = if arg.flag.is_empty() {
            arg.pointer.clone()
        } else {
            arg.flag.clone()
        };
        match rules::option_rule(arg) {
            Some(OptionRule::OutputPath) => {
                out.signals.push(signal(
                    "writes_filesystem",
                    pb::Tri::True,
                    pb::Confidence::Inferred,
                    rules::CLI_OPTIONS,
                    format!("output parameter {label}"),
                ));
                out.scopes.push(pb::ScopeParam {
                    pointer: arg.pointer.clone(),
                    kind: pb::ScopeKind::WritePath as i32,
                });
            }
            Some(OptionRule::Discard) => {
                let s = signal(
                    "may_discard_data",
                    pb::Tri::True,
                    pb::Confidence::Inferred,
                    rules::CLI_OPTIONS,
                    format!("{label} may discard data"),
                );
                out.branches.push(pb::EffectBranch {
                    pointer: arg.pointer.clone(),
                    equals_json: String::new(),
                    effects: Some(pb::Effects {
                        may_discard_data: pb::Tri::True as i32,
                        ..Default::default()
                    }),
                    signals: vec![s],
                });
            }
            Some(OptionRule::Browser) => {}
            Some(OptionRule::MixedAuthority) if !overlay_branch_pointers.contains(&arg.pointer.as_str()) => {
                out.branches.push(pb::EffectBranch {
                    pointer: arg.pointer.clone(),
                    equals_json: String::new(),
                    effects: None,
                    signals: vec![signal(
                        "mutates_target",
                        pb::Tri::Unknown,
                        pb::Confidence::Inferred,
                        rules::CLI_OPTIONS,
                        format!("{label} switches authority by value (unresolved branch)"),
                    )],
                });
                out.withheld.push(withheld(
                        pb::WithheldReason::MixedAuthority,
                        format!(
                            "{label} switches authority by value; resolve it with overlay branches on {} or a mode split in target.aa.yaml",
                            arg.pointer
                        ),
                    ));
            }
            Some(OptionRule::MixedAuthority) | None => {}
        }
    }

    // Overlay raises (effects and branches are raise-only).
    if let Some(oc) = overlay_cap {
        for (field, v) in &oc.effects {
            match overlay_raise(field, *v) {
                Some(t) => out.signals.push(signal(
                    field,
                    t,
                    pb::Confidence::Inferred,
                    OVERLAY_EFFECTS,
                    "overlay raise",
                )),
                None => out.signals.push(signal(
                    field,
                    if *v { pb::Tri::True } else { pb::Tri::False },
                    pb::Confidence::Inferred,
                    OVERLAY_EFFECTS,
                    "overlay entries are raise-only; lowering needs lane evidence or AssumeEffects (ignored)",
                )),
            }
        }
        for b in &oc.branches {
            let mut e = pb::Effects::default();
            let mut sigs = Vec::new();
            for (field, v) in &b.effects {
                if let Some(t) = overlay_raise(field, *v) {
                    let _ = effects::set(&mut e, field, t);
                    sigs.push(signal(
                        field,
                        t,
                        pb::Confidence::Inferred,
                        OVERLAY_BRANCHES,
                        "overlay branch raise",
                    ));
                }
            }
            out.branches.push(pb::EffectBranch {
                pointer: b.pointer.clone(),
                equals_json: b
                    .equals
                    .as_ref()
                    .and_then(|v| crate::digest::jcs(v).ok())
                    .and_then(|v| String::from_utf8(v).ok())
                    .unwrap_or_default(),
                effects: Some(e),
                signals: sigs,
            });
        }
    }

    // cli-anything@1 autosave.
    if let Some(s) = click::autosave(cli, &out.signals) {
        out.signals.push(s);
    }

    // compile@1 binding-intrinsic declarations for headless cli_argv.
    out.signals
        .extend(rules::binding_intrinsic_signals(target, cap.binding.as_ref()));

    // Interactive surfaces.
    if click::interactive(&cand.native_path, cand.interactive) {
        out.withheld.push(withheld(
            pb::WithheldReason::Interactive,
            "REPL/TTY-only surface; never served one-shot (rule:cli-anything@1)",
        ));
    }
    if cobra::is_builtin(&cand.native_path) {
        out.withheld.push(withheld(
            pb::WithheldReason::OperatorExcluded,
            "Cobra completion helper, not a capability of the target",
        ));
    }

    // exclusions@1.
    let text = format!("{}\n{}", text_of(&cap.title), text_of(&cap.description));
    if let Some(cat) = crate::policy::exclusions::excluded(target, &cap.id, &text) {
        out.withheld.push(withheld(
            pb::WithheldReason::ExcludedByPolicy,
            format!("hard exclusion '{cat}' ({}); never servable", rules::EXCLUSIONS),
        ));
    }

    // help-text@1: several usage forms of the same native command = mode-switching authority.
    let native = cap.provenance.as_ref().is_some_and(|p| !p.native_ids.is_empty());
    if native && help_text::multi_mode(text_of(&cap.description), &cand.native_path) {
        out.withheld.push(withheld(
            pb::WithheldReason::MixedAuthority,
            "the usage block shows several modes of this command; split it by mode in target.aa.yaml \
             (capabilities.<id>.binding.command_path + fixed_args)",
        ));
    }

    sort_signals(&mut out.signals);
    out.branches
        .sort_by(|a, b| (&a.pointer, &a.equals_json).cmp(&(&b.pointer, &b.equals_json)));
    out.scopes
        .sort_by(|a, b| (&a.pointer, a.kind).cmp(&(&b.pointer, b.kind)));
    out.scopes.dedup();
    out.withheld
        .sort_by(|a, b| (a.reason, &a.hint).cmp(&(b.reason, &b.hint)));
    out.withheld.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Overlay;

    fn cand(path: &[&str], args: Vec<pb::ArgMapping>) -> crate::api::Candidate {
        let target = "t";
        let p: Vec<String> = path.iter().map(|s| s.to_string()).collect();
        crate::api::Candidate {
            capability: pb::Capability {
                id: crate::ids::capability_id(target, path),
                binding: Some(pb::Binding {
                    kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding {
                        args,
                        kill_scope: pb::KillScope::OwnedGroup as i32,
                        ..Default::default()
                    })),
                }),
                provenance: Some(pb::Provenance {
                    native_ids: vec![p.join(" ")],
                    ..Default::default()
                }),
                ..Default::default()
            },
            native_path: p,
            native_signals: vec![signal(
                "writes_filesystem",
                pb::Tri::False,
                pb::Confidence::Declared,
                "harvest:x",
                "",
            )],
            interactive: false,
        }
    }

    #[test]
    fn rule_versions_are_the_five_tables() {
        let v = rule_versions();
        assert_eq!(v.len(), 5);
        assert!(v.values().all(|x| x == "1"));
    }

    #[test]
    fn options_branches_and_native_demotion() {
        let target = pb::Target {
            id: "t".into(),
            ..Default::default()
        };
        let args = vec![
            pb::ArgMapping {
                pointer: "/force".into(),
                flag: "--force".into(),
                style: pb::ArgStyle::Flag as i32,
                ..Default::default()
            },
            pb::ArgMapping {
                pointer: "/output".into(),
                flag: "--output".into(),
                style: pb::ArgStyle::Option as i32,
                ..Default::default()
            },
            pb::ArgMapping {
                pointer: "/method".into(),
                flag: "-X".into(),
                style: pb::ArgStyle::Option as i32,
                ..Default::default()
            },
        ];
        let c = characterize(&target, &cand(&["api"], args), &Overlay::default());
        assert_eq!(c.branches.len(), 2);
        assert!(c.branches.iter().any(|b| b.effects.is_none()));
        assert_eq!(c.scopes[0].kind, pb::ScopeKind::WritePath as i32);
        assert!(
            c.withheld
                .iter()
                .any(|w| w.reason == pb::WithheldReason::MixedAuthority as i32)
        );
        let native = c.signals.iter().find(|s| s.source == "harvest:x").unwrap();
        assert_eq!(native.confidence, pb::Confidence::Inferred as i32);
        let e = effects::combine(&c.signals, true);
        assert_eq!(e.writes_filesystem, pb::Tri::True as i32);
        assert_eq!(e.changes_ui_state, pb::Tri::False as i32);
    }

    #[test]
    fn repl_and_exclusion() {
        let target = pb::Target {
            id: "t".into(),
            ..Default::default()
        };
        let c = characterize(&target, &cand(&["repl"], vec![]), &Overlay::default());
        assert_eq!(c.withheld[0].reason, pb::WithheldReason::Interactive as i32);
        let c = characterize(&target, &cand(&["license", "activate"], vec![]), &Overlay::default());
        assert!(
            c.withheld
                .iter()
                .any(|w| w.reason == pb::WithheldReason::ExcludedByPolicy as i32)
        );
    }
}
