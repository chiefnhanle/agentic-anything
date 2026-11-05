//! Acceptance test 2 (build-plan §3.1): the policy evaluator is order-free, strictest-wins, default-deny, puts
//! hard exclusions first, and every denial reason has a remediation.

use aa_core::policy::{self, ActionDescriptor, Policy};
use aa_core::{effects, pb};
use proptest::prelude::*;
use std::path::PathBuf;

const LEVELS: &[&str] = &["read", "ui", "write", "destructive"];
const TIERS: &[&str] = &["observe", "launch", "drive", "write", "destructive", "attach"];
const VERDICTS: &[&str] = &["allow", "ask", "deny"];

#[derive(Debug, Clone)]
struct RuleSpec {
    effect: usize,
    level: Option<usize>,
    tier: Option<usize>,
    targets: Option<bool>,
    registered: Option<bool>,
    tainted: Option<bool>,
    any_effect: Option<usize>,
}

fn rule_spec() -> impl Strategy<Value = RuleSpec> {
    (
        0..3usize,
        prop::option::of(0..LEVELS.len()),
        prop::option::of(0..TIERS.len()),
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        prop::option::of(0..effects::TARGET_FIELDS.len()),
    )
        .prop_map(
            |(effect, level, tier, targets, registered, tainted, any_effect)| RuleSpec {
                effect,
                level,
                tier,
                targets,
                registered,
                tainted,
                any_effect,
            },
        )
}

fn render(rules: &[(String, RuleSpec)]) -> String {
    let mut s = String::from("format = \"aa-policy/1\"\ndefault = \"deny\"\n");
    for (id, r) in rules {
        let mut when = Vec::new();
        if let Some(l) = r.level {
            when.push(format!("level = \"{}\"", LEVELS[l]));
        }
        if let Some(t) = r.tier {
            when.push(format!("tier = \"{}\"", TIERS[t]));
        }
        if let Some(t) = r.targets {
            when.push(format!("targets = [\"{}\"]", if t { "git*" } else { "gh" }));
        }
        if let Some(b) = r.registered {
            when.push(format!("target_registered = {b}"));
        }
        if let Some(b) = r.tainted {
            when.push(format!("session_tainted = {b}"));
        }
        if let Some(f) = r.any_effect {
            when.push(format!("any_effect = [\"{}\"]", effects::TARGET_FIELDS[f]));
        }
        s.push_str(&format!(
            "\n[[rule]]\nid = \"{id}\"\neffect = \"{}\"\nwhen = {{ {} }}\n",
            VERDICTS[r.effect],
            when.join(", ")
        ));
    }
    s
}

fn effects_value() -> impl Strategy<Value = pb::Effects> {
    prop::collection::vec(0i32..=2, effects::ALL_FIELDS.len()).prop_map(|vals| {
        let mut e = pb::Effects::default();
        for (f, v) in effects::ALL_FIELDS.iter().zip(vals) {
            effects::set(&mut e, f, pb::Tri::try_from(v).unwrap()).unwrap();
        }
        e
    })
}

#[derive(Debug, Clone)]
struct Desc {
    level: pb::EffectLevel,
    tier: pb::AuthorityTier,
    effects: pb::Effects,
    registered: bool,
    tainted: bool,
    target: &'static str,
}

fn desc() -> impl Strategy<Value = Desc> {
    (
        0i32..=4,
        0i32..=8,
        effects_value(),
        any::<bool>(),
        any::<bool>(),
        prop::sample::select(vec!["git", "gh", "cli-anything.lo"]),
    )
        .prop_map(|(l, t, effects, registered, tainted, target)| Desc {
            level: pb::EffectLevel::try_from(l).unwrap(),
            tier: pb::AuthorityTier::try_from(t).unwrap(),
            effects,
            registered,
            tainted,
            target,
        })
}

fn eval(p: &Policy, d: &Desc) -> pb::PolicyDecision {
    let cap = format!("{}.thing.show", d.target);
    policy::evaluate(
        p,
        &ActionDescriptor {
            target_id: d.target,
            capability_id: &cap,
            level: d.level,
            tier: d.tier,
            effects: &d.effects,
            read_paths: &[],
            write_paths: &[],
            target_registered: d.registered,
            session_tainted: d.tainted,
            assurance: pb::AssuranceTier::SamePrincipal,
        },
    )
}

fn strictness(v: i32) -> u8 {
    match pb::Verdict::try_from(v).unwrap() {
        pb::Verdict::Allow => 1,
        pb::Verdict::Ask => 2,
        pb::Verdict::Deny => 3,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn decision_is_invariant_under_rule_shuffles(specs in prop::collection::vec(rule_spec(), 0..8), d in desc(), seed in any::<u64>()) {
        let rules: Vec<(String, RuleSpec)> = specs.into_iter().enumerate().map(|(i, r)| (format!("r{i}"), r)).collect();
        let a = policy::parse_policy(&render(&rules)).unwrap();
        let mut shuffled = rules.clone();
        let n = shuffled.len().max(1);
        for i in 0..shuffled.len() {
            let j = (seed as usize).wrapping_mul(31).wrapping_add(i * 17) % n;
            shuffled.swap(i, j);
        }
        let b = policy::parse_policy(&render(&shuffled)).unwrap();
        let (da, mut db) = (eval(&a, &d), eval(&b, &d));
        db.policy_digest = da.policy_digest.clone();
        prop_assert_eq!(da, db);
    }

    #[test]
    fn deny_beats_ask_beats_allow(specs in prop::collection::vec(rule_spec(), 1..8), d in desc()) {
        let rules: Vec<(String, RuleSpec)> = specs.into_iter().enumerate().map(|(i, r)| (format!("r{i}"), r)).collect();
        let p = policy::parse_policy(&render(&rules)).unwrap();
        let dec = eval(&p, &d);
        // The verdict is the strictest of the rules that matched (each evaluated alone).
        let mut strictest = 0u8;
        for (id, r) in &rules {
            let alone = policy::parse_policy(&render(&[(id.clone(), r.clone())])).unwrap();
            let one = eval(&alone, &d);
            if !one.matched_rule_ids.is_empty() {
                strictest = strictest.max(strictness(one.verdict));
            }
        }
        if strictest == 0 {
            prop_assert_eq!(dec.verdict, pb::Verdict::Deny as i32);
            prop_assert_eq!(dec.denial_reason, pb::DenialReason::NoMatchingAllow as i32);
        } else {
            prop_assert_eq!(strictness(dec.verdict), strictest);
        }
        if dec.verdict == pb::Verdict::Allow as i32 {
            prop_assert_eq!(dec.approval_source, pb::ApprovalSource::PolicyAllow as i32);
        }
        prop_assert!(dec.matched_rule_ids.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn unknown_never_matches_allow(d in desc()) {
        let p = policy::parse_policy(&render(&[("all-read".into(), RuleSpec {
            effect: 0, level: Some(0), tier: None, targets: None, registered: None, tainted: None, any_effect: None,
        })])).unwrap();
        let mut d = d;
        d.level = pb::EffectLevel::Unknown;
        prop_assert_eq!(eval(&p, &d).verdict, pb::Verdict::Deny as i32);
    }

    #[test]
    fn exclusions_beat_any_allow(d in desc()) {
        let p = policy::parse_policy("format = \"aa-policy/1\"\n[[rule]]\nid = \"everything\"\neffect = \"allow\"\n").unwrap();
        let e = d.effects;
        let dec = policy::evaluate(&p, &ActionDescriptor {
            target_id: "app",
            capability_id: "app.license.activate",
            level: d.level,
            tier: d.tier,
            effects: &e,
            read_paths: &[],
            write_paths: &[],
            target_registered: true,
            session_tainted: false,
            assurance: pb::AssuranceTier::IsolatedTarget,
        });
        prop_assert_eq!(dec.verdict, pb::Verdict::Deny as i32);
        prop_assert_eq!(dec.denial_reason, pb::DenialReason::HardExclusion as i32);
        prop_assert!(dec.matched_rule_ids.is_empty());
    }
}

#[test]
fn empty_policy_denies_with_no_matching_allow() {
    let p = policy::parse_policy("format = \"aa-policy/1\"\n").unwrap();
    let e = pb::Effects::default();
    let d = Desc {
        level: pb::EffectLevel::Read,
        tier: pb::AuthorityTier::Observe,
        effects: e,
        registered: true,
        tainted: false,
        target: "git",
    };
    let dec = eval(&p, &d);
    assert_eq!(dec.verdict, pb::Verdict::Deny as i32);
    assert_eq!(dec.denial_reason, pb::DenialReason::NoMatchingAllow as i32);
    assert_eq!(dec.deciding_rule_id, policy::DEFAULT_DENY_ID);
    assert_eq!(dec.policy_digest, policy::policy_digest("format = \"aa-policy/1\"\n"));
}

#[test]
fn every_denial_reason_has_a_remediation_command() {
    let e = pb::Effects::default();
    let d = ActionDescriptor {
        target_id: "git",
        capability_id: "git.status",
        level: pb::EffectLevel::Read,
        tier: pb::AuthorityTier::Observe,
        effects: &e,
        read_paths: &[],
        write_paths: &[],
        target_registered: true,
        session_tainted: false,
        assurance: pb::AssuranceTier::SamePrincipal,
    };
    // Slice-1 `aa` commands (build-plan §3.6).
    let commands = [
        "daemon", "status", "halt", "unhalt", "target", "import", "discover", "verify", "list", "describe", "invoke",
        "x", "audit",
    ];
    for v in 0..=9 {
        let reason = pb::DenialReason::try_from(v).unwrap();
        let r = policy::remediation_for(reason, &d);
        assert!(!r.trim().is_empty(), "{reason:?}");
        let cmd = r
            .rsplit(": aa ")
            .next()
            .filter(|_| r.contains(": aa "))
            .unwrap_or_else(|| panic!("{reason:?}: {r}"));
        let sub = cmd.split_whitespace().next().unwrap();
        assert!(commands.contains(&sub), "{reason:?} names unknown command {sub:?}");
    }
    assert!(
        pb::DenialReason::try_from(10).is_err(),
        "a new DenialReason needs a remediation"
    );
}

#[test]
fn architecture_example_policy_parses_and_decides() {
    let text = r#"format = "aa-policy/1"
default = "deny"

[paths]
write_roots = ["~/Documents/aa-out", "~/src"]
deny        = ["~/.ssh", "~/.aws", "~/.config/agentic-anything", "~/Library/Keychains"]

[[rule]]
id = "observe-registered"
effect = "allow"
when = { tier = "observe", target_registered = true }

[[rule]]
id = "write-in-roots"
effect = "allow"
when = { level = "write", targets = ["git", "cli-anything.*"] }

[[rule]]
id = "destructive"
effect = "ask"
when = { level = "destructive" }
approval_sources = ["operator_socket"]

[[rule]]
id = "tainted-exfil"
effect = "ask"
when = { session_tainted = true, any_effect = ["external_side_effect", "executes_foreign_code", "reads_credentials", "writes_clipboard"] }
approval_sources = ["operator_socket"]

[[rule]]
id = "privacy-reads"
effect = "ask"
when = { any_effect = ["reads_screen", "reads_clipboard", "reads_credentials"] }
"#;
    let p = policy::parse_policy(text).unwrap();
    assert_eq!(p.rules.len(), 5);
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    assert!(p.paths.deny.contains(&home.join(".ssh")));
    let mut e = pb::Effects::default();
    for f in effects::ALL_FIELDS {
        effects::set(&mut e, f, pb::Tri::False).unwrap();
    }
    let read = Desc {
        level: pb::EffectLevel::Read,
        tier: pb::AuthorityTier::Observe,
        effects: e,
        registered: true,
        tainted: true,
        target: "git",
    };
    let dec = eval(&p, &read);
    assert_eq!(dec.verdict, pb::Verdict::Allow as i32);
    assert_eq!(dec.deciding_rule_id, "observe-registered");
    let mut destructive = read.clone();
    destructive.level = pb::EffectLevel::Destructive;
    destructive.tier = pb::AuthorityTier::Destructive;
    destructive.effects.may_discard_data = pb::Tri::True as i32;
    let dec = eval(&p, &destructive);
    assert_eq!(dec.verdict, pb::Verdict::Ask as i32);
    assert_eq!(dec.deciding_rule_id, "destructive");
    // A read under a denied path is refused even though a rule allows it.
    let ssh = [home.join(".ssh/id_ed25519")];
    let dec = policy::evaluate(
        &p,
        &ActionDescriptor {
            target_id: "git",
            capability_id: "git.status",
            level: pb::EffectLevel::Read,
            tier: pb::AuthorityTier::Observe,
            effects: &e,
            read_paths: &ssh,
            write_paths: &[],
            target_registered: true,
            session_tainted: false,
            assurance: pb::AssuranceTier::SamePrincipal,
        },
    );
    assert_eq!(dec.denial_reason, pb::DenialReason::OutsidePathScope as i32);
}
