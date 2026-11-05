//! Acceptance test 1 (build-plan §3.1): properties of the effects lattice and its projections.

use aa_core::effects::{self, ALL_FIELDS, IDEMPOTENT, TARGET_FIELDS};
use aa_core::pb;
use proptest::prelude::*;

const SOURCES: &[&str] = &[
    "verify:vr_1",
    "verify:vr_2",
    "operator:audit_seq=7",
    "rule:cli-verbs@1",
    "rule:compile@1",
    "proposal:p1",
    "harvest:click-info@1",
    "overlay:effects",
];

fn tri() -> impl Strategy<Value = pb::Tri> {
    prop_oneof![Just(pb::Tri::Unknown), Just(pb::Tri::False), Just(pb::Tri::True)]
}

fn confidence() -> impl Strategy<Value = pb::Confidence> {
    (0i32..=5).prop_map(|c| pb::Confidence::try_from(c).unwrap())
}

fn isolation() -> impl Strategy<Value = pb::Isolation> {
    (0i32..=5).prop_map(|c| pb::Isolation::try_from(c).unwrap())
}

fn signal() -> impl Strategy<Value = pb::EffectSignal> {
    (0..ALL_FIELDS.len(), tri(), confidence(), 0..SOURCES.len(), isolation()).prop_map(|(f, v, c, s, i)| {
        pb::EffectSignal {
            field: ALL_FIELDS[f].to_string(),
            value: v as i32,
            confidence: c as i32,
            source: SOURCES[s].to_string(),
            isolation: i as i32,
            ..Default::default()
        }
    })
}

fn signals() -> impl Strategy<Value = Vec<pb::EffectSignal>> {
    prop::collection::vec(signal(), 0..24)
}

fn effects_value() -> impl Strategy<Value = pb::Effects> {
    prop::collection::vec(tri(), ALL_FIELDS.len()).prop_map(|vals| {
        let mut e = pb::Effects::default();
        for (f, v) in ALL_FIELDS.iter().zip(vals) {
            effects::set(&mut e, f, v).unwrap();
        }
        e
    })
}

fn rank_of(e: &pb::Effects, f: &str) -> u8 {
    effects::lattice_rank(f, effects::get(e, f).unwrap())
}

/// `e2` is at least as risky as `e1` on every field.
fn riskier(e1: &pb::Effects, raise: &[(usize, pb::Tri)]) -> pb::Effects {
    let mut e2 = *e1;
    for (i, v) in raise {
        let f = ALL_FIELDS[*i];
        let cur = effects::get(&e2, f).unwrap();
        if effects::risk_rank(f, *v) > effects::risk_rank(f, cur) {
            effects::set(&mut e2, f, *v).unwrap();
        }
    }
    e2
}

fn binding() -> pb::Binding {
    pb::Binding {
        kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding::default())),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn combine_is_commutative_associative_idempotent(a in signals(), b in signals(), c in signals(), wpp: bool) {
        let cat = |x: &[pb::EffectSignal], y: &[pb::EffectSignal]| [x, y].concat();
        // Commutative (order-free) and a join-homomorphism.
        prop_assert_eq!(effects::combine(&cat(&a, &b), wpp), effects::combine(&cat(&b, &a), wpp));
        let ab = effects::join(&effects::combine(&a, wpp), &effects::combine(&b, wpp));
        prop_assert_eq!(effects::combine(&cat(&a, &b), wpp), ab);
        // Associative.
        let left = effects::join(&ab, &effects::combine(&c, wpp));
        let bc = effects::join(&effects::combine(&b, wpp), &effects::combine(&c, wpp));
        let right = effects::join(&effects::combine(&a, wpp), &bc);
        prop_assert_eq!(left, right);
        // Idempotent.
        prop_assert_eq!(effects::combine(&cat(&a, &a), wpp), effects::combine(&a, wpp));
        let mut shuffled = a.clone();
        shuffled.reverse();
        prop_assert_eq!(effects::combine(&shuffled, wpp), effects::combine(&a, wpp));
    }

    #[test]
    fn adding_a_signal_never_lowers_a_rank(a in signals(), s in signal(), wpp: bool) {
        let before = effects::combine(&a, wpp);
        let mut more = a.clone();
        more.push(s);
        let after = effects::combine(&more, wpp);
        for f in ALL_FIELDS {
            prop_assert!(rank_of(&after, f) >= rank_of(&before, f), "{} lowered", f);
        }
    }

    #[test]
    fn proposed_and_inferred_false_never_lower(a in signals(), wpp: bool) {
        let weak: Vec<pb::EffectSignal> = a
            .into_iter()
            .map(|mut s| {
                if s.confidence != pb::Confidence::Inferred as i32 {
                    s.confidence = pb::Confidence::Proposed as i32;
                }
                s
            })
            .collect();
        let e = effects::combine(&weak, wpp);
        for f in TARGET_FIELDS.iter().chain([&effects::MUTATES_SESSION]) {
            prop_assert_ne!(effects::get(&e, f).unwrap(), pb::Tri::False, "{} lowered by a weak source", f);
        }
        prop_assert_ne!(effects::get(&e, IDEMPOTENT).unwrap(), pb::Tri::True);
    }

    #[test]
    fn observed_false_at_process_cannot_lower_confined_only_fields(a in signals(), wpp: bool) {
        let at_process: Vec<pb::EffectSignal> = a
            .into_iter()
            .map(|mut s| {
                s.confidence = pb::Confidence::Observed as i32;
                s.source = "verify:vr_p".into();
                s.isolation = pb::Isolation::Process as i32;
                s
            })
            .collect();
        let e = effects::combine(&at_process, wpp);
        for f in ["accesses_network", "launches_process", "writes_filesystem"] {
            prop_assert_ne!(effects::get(&e, f).unwrap(), pb::Tri::False, "{} lowered at PROCESS", f);
        }
    }

    #[test]
    fn write_path_param_blocks_lowering_writes_filesystem(a in signals()) {
        let mut a = a;
        for c in [pb::Confidence::Observed, pb::Confidence::Operator, pb::Confidence::Declared] {
            a.push(pb::EffectSignal {
                field: "writes_filesystem".into(),
                value: pb::Tri::False as i32,
                confidence: c as i32,
                source: if c == pb::Confidence::Operator { "operator:audit_seq=1" } else { "verify:vr_c" }.into(),
                isolation: pb::Isolation::VirtualMachine as i32,
                ..Default::default()
            });
        }
        prop_assert_ne!(effects::combine(&a, true).writes_filesystem, pb::Tri::False as i32);
        prop_assert_eq!(effects::min_isolation_to_lower("writes_filesystem", true), None);
    }

    #[test]
    fn projections_are_monotone(e1 in effects_value(), raise in prop::collection::vec((0..ALL_FIELDS.len(), tri()), 0..8)) {
        let e2 = riskier(&e1, &raise);
        let b = binding();
        prop_assert!(effects::level_rank(effects::level(&e2)) >= effects::level_rank(effects::level(&e1)));
        prop_assert!(effects::tier_rank(effects::tier(&e2, &b)) >= effects::tier_rank(effects::tier(&e1, &b)));
        let (h1, h2) = (effects::hints(&e1), effects::hints(&e2));
        prop_assert!(h2.destructive >= h1.destructive);
        prop_assert!(h2.open_world >= h1.open_world);
        prop_assert!(h2.read_only <= h1.read_only);
        prop_assert!(h2.idempotent <= h1.idempotent);
    }

    #[test]
    fn any_unknown_target_field_means_unknown_level(e in effects_value(), i in 0..TARGET_FIELDS.len()) {
        let mut e = e;
        effects::set(&mut e, TARGET_FIELDS[i], pb::Tri::Unknown).unwrap();
        prop_assert_eq!(effects::level(&e), pb::EffectLevel::Unknown);
        prop_assert_eq!(effects::tier(&e, &binding()), pb::AuthorityTier::Unknown);
        prop_assert!(!effects::hints(&e).read_only);
        prop_assert_eq!(effects::tier_rank(pb::AuthorityTier::Unknown), 9);
    }

    #[test]
    fn branches_only_raise(e in effects_value(), b in effects_value(), on: bool) {
        let branch = pb::EffectBranch {
            pointer: "/x".into(),
            effects: Some(b),
            ..Default::default()
        };
        let input = serde_json::json!({"x": on});
        let out = effects::apply_branches(&e, &[branch], &input);
        for f in ALL_FIELDS {
            prop_assert!(rank_of(&out, f) >= rank_of(&e, f));
        }
        if !on {
            prop_assert_eq!(out, e);
        }
    }
}

#[test]
fn default_effects_are_most_restrictive() {
    let e = pb::Effects::default();
    assert_eq!(effects::level(&e), pb::EffectLevel::Unknown);
    assert_eq!(effects::tier(&e, &binding()), pb::AuthorityTier::Unknown);
    let h = effects::hints(&e);
    assert!(!h.read_only && h.destructive && h.open_world && !h.idempotent);
}
