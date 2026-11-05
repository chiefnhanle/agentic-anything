//! ADR 0002: every safety-relevant enum's zero value (what an omitted field decodes to) is its most restrictive
//! value, and proto3-JSON (pbjson) round-trips the persisted messages byte-identically after JCS.

use aa_core::{digest::jcs, pb};
use proptest::prelude::*;
use prost::Message as _;
use std::collections::BTreeSet;

/// Asserts `T::default()` and `T::try_from(0)` both decode to `expected`.
macro_rules! zero_is {
    ($ty:ty, $expected:expr) => {{
        assert_eq!(<$ty>::default(), $expected, "{}::default()", stringify!($ty));
        assert_eq!(<$ty>::try_from(0), Ok($expected), "{} from 0", stringify!($ty));
        assert_eq!($expected as i32, 0, "{}", stringify!($expected));
    }};
}

/// Proto enum names whose zero value is deliberately meaningful (not `*_UNSPECIFIED`); each is checked below.
const SAFETY_ENUMS: &[&str] = &[
    "Tri",
    "AdmissionState",
    "Verdict",
    "Role",
    "EffectLevel",
    "AuthorityTier",
    "ContentOrigin",
    "HaltScope",
    "Isolation",
    "Confidence",
    "Outcome",
    "LaneStatus",
    "ApprovalSource",
    "AssuranceTier",
    "RequestState",
];

#[test]
fn safety_enum_zero_values_are_most_restrictive() {
    zero_is!(pb::Tri, pb::Tri::Unknown);
    zero_is!(pb::AdmissionState, pb::AdmissionState::Withheld);
    zero_is!(pb::Verdict, pb::Verdict::Deny);
    zero_is!(pb::Role, pb::Role::None);
    zero_is!(pb::EffectLevel, pb::EffectLevel::Unknown);
    zero_is!(pb::AuthorityTier, pb::AuthorityTier::Unknown);
    zero_is!(pb::ContentOrigin, pb::ContentOrigin::Target);
    zero_is!(pb::HaltScope, pb::HaltScope::Engine);
    zero_is!(pb::Isolation, pb::Isolation::NotProven);
    zero_is!(pb::Confidence, pb::Confidence::Proposed);
    zero_is!(pb::Outcome, pb::Outcome::Unknown);
    zero_is!(pb::LaneStatus, pb::LaneStatus::NotRun);
    zero_is!(pb::ApprovalSource, pb::ApprovalSource::None);
    zero_is!(pb::AssuranceTier, pb::AssuranceTier::SamePrincipal);
    zero_is!(pb::RequestState, pb::RequestState::UnknownId);
}

#[test]
fn omitted_fields_decode_to_restrictive_messages() {
    // An empty wire message is the "everything omitted" case.
    let e = pb::Effects::decode(&[][..]).unwrap();
    let tris = [
        e.mutates_target,
        e.writes_filesystem,
        e.launches_process,
        e.accesses_network,
        e.changes_ui_state,
        e.sends_input,
        e.may_discard_data,
        e.executes_foreign_code,
        e.mutates_device,
        e.external_side_effect,
        e.attaches_process,
        e.reads_screen,
        e.reads_clipboard,
        e.writes_clipboard,
        e.reads_credentials,
        e.irreversible,
        e.mutates_session,
        e.idempotent,
    ];
    assert!(tris.iter().all(|t| *t == pb::Tri::Unknown as i32));

    let a: pb::Admission = serde_json::from_str("{}").unwrap();
    assert_eq!(a.state(), pb::AdmissionState::Withheld);
    assert_eq!(a.required_runtime_isolation(), pb::Isolation::NotProven);

    let d: pb::PolicyDecision = serde_json::from_str("{}").unwrap();
    assert_eq!(d.verdict(), pb::Verdict::Deny);
    assert_eq!(d.approval_source(), pb::ApprovalSource::None);
    assert_eq!(d.assurance(), pb::AssuranceTier::SamePrincipal);

    let c: pb::Capability = serde_json::from_str("{}").unwrap();
    assert_eq!(c.level(), pb::EffectLevel::Unknown);
    assert_eq!(c.tier(), pb::AuthorityTier::Unknown);
    assert!(
        c.admission.is_none(),
        "a capability without admission is never servable"
    );

    let r: pb::ActionRecord = serde_json::from_str("{}").unwrap();
    assert_eq!(r.outcome(), pb::Outcome::Unknown);
    assert_eq!(r.runtime_isolation(), pb::Isolation::NotProven);

    let s: pb::EffectSignal = serde_json::from_str("{}").unwrap();
    assert_eq!(s.confidence(), pb::Confidence::Proposed);
    assert_eq!(s.isolation(), pb::Isolation::NotProven);

    let t: pb::TargetText = serde_json::from_str("{}").unwrap();
    assert_eq!(t.origin(), pb::ContentOrigin::Target);

    let p: pb::Principal = serde_json::from_str("{}").unwrap();
    assert_eq!(p.role(), pb::Role::None);

    let h: pb::HaltState = serde_json::from_str("{}").unwrap();
    assert_eq!(h.scope(), pb::HaltScope::Engine);

    let v: pb::VerificationRecord = serde_json::from_str("{}").unwrap();
    assert_eq!(v.status(), pb::LaneStatus::NotRun);
}

#[test]
fn zero_is_the_bottom_of_ordered_safety_ladders() {
    // Ladders where a higher number means more trust/containment: zero must be the weakest.
    assert!(
        (0..=5)
            .filter_map(|n| pb::Isolation::try_from(n).ok())
            .all(|i| i >= pb::Isolation::NotProven)
    );
    assert!(
        (0..=5)
            .filter_map(|n| pb::Confidence::try_from(n).ok())
            .all(|c| c >= pb::Confidence::Proposed)
    );
    assert!(
        (0..=2)
            .filter_map(|n| pb::AssuranceTier::try_from(n).ok())
            .all(|t| t >= pb::AssuranceTier::SamePrincipal)
    );
}

/// Guard: any enum added to the protos with a meaningful (non-UNSPECIFIED) zero value must be listed above.
#[test]
fn every_meaningful_zero_enum_is_covered() {
    let fds = pbjson_types::FileDescriptorSet::decode(aa_proto::FILE_DESCRIPTOR_SET).expect("descriptor set");
    let mut meaningful = BTreeSet::new();
    fn walk(
        prefix: &str,
        msgs: &[pbjson_types::DescriptorProto],
        enums: &[pbjson_types::EnumDescriptorProto],
        out: &mut BTreeSet<String>,
    ) {
        for e in enums {
            let zero = e
                .value
                .iter()
                .find(|v| v.number == Some(0))
                .and_then(|v| v.name.clone())
                .unwrap_or_default();
            if !zero.ends_with("_UNSPECIFIED") {
                out.insert(format!("{prefix}{}", e.name.clone().unwrap_or_default()));
            }
        }
        for m in msgs {
            let p = format!("{prefix}{}.", m.name.clone().unwrap_or_default());
            walk(&p, &m.nested_type, &m.enum_type, out);
        }
    }
    for f in fds
        .file
        .iter()
        .filter(|f| f.package.as_deref() == Some("agentic_anything.v1"))
    {
        walk("", &f.message_type, &f.enum_type, &mut meaningful);
    }
    let listed: BTreeSet<String> = SAFETY_ENUMS.iter().map(|s| s.to_string()).collect();
    assert_eq!(meaningful, listed, "safety enums in protos vs SAFETY_ENUMS");
}

// ---------------------------------------------------------------------------
// pbjson round-trip proptests
// ---------------------------------------------------------------------------

fn text() -> impl Strategy<Value = String> {
    "(?s).{0,12}"
}

fn e(max: i32) -> impl Strategy<Value = i32> {
    0..=max
}

fn timestamp() -> impl Strategy<Value = Option<pbjson_types::Timestamp>> {
    // RFC 3339 range: 0001-01-01T00:00:00Z ..= 9999-12-31T23:59:59Z.
    proptest::option::of(
        (-62_135_596_800i64..=253_402_300_799, 0i32..1_000_000_000)
            .prop_map(|(seconds, nanos)| pbjson_types::Timestamp { seconds, nanos }),
    )
}

fn target_text() -> impl Strategy<Value = Option<pb::TargetText>> {
    proptest::option::of(
        (text(), e(4), text()).prop_map(|(text, origin, source)| pb::TargetText { text, origin, source }),
    )
}

fn effects() -> impl Strategy<Value = pb::Effects> {
    prop::array::uniform18(e(2)).prop_map(|t| pb::Effects {
        mutates_target: t[0],
        writes_filesystem: t[1],
        launches_process: t[2],
        accesses_network: t[3],
        changes_ui_state: t[4],
        sends_input: t[5],
        may_discard_data: t[6],
        executes_foreign_code: t[7],
        mutates_device: t[8],
        external_side_effect: t[9],
        attaches_process: t[10],
        reads_screen: t[11],
        reads_clipboard: t[12],
        writes_clipboard: t[13],
        reads_credentials: t[14],
        irreversible: t[15],
        mutates_session: t[16],
        idempotent: t[17],
    })
}

fn signal() -> impl Strategy<Value = pb::EffectSignal> {
    (
        text(),
        e(2),
        e(5),
        text(),
        text(),
        prop::collection::vec(text(), 0..2),
        e(5),
    )
        .prop_map(
            |(field, value, confidence, source, rationale, evidence_ids, isolation)| pb::EffectSignal {
                field,
                value,
                confidence,
                source,
                rationale,
                evidence_ids,
                isolation,
            },
        )
}

fn binding() -> impl Strategy<Value = Option<pb::Binding>> {
    let arg = (text(), e(5), text(), any::<u32>(), any::<bool>()).prop_map(
        |(pointer, style, flag, position, no_equals_form)| pb::ArgMapping {
            pointer,
            style,
            flag,
            position,
            no_equals_form,
        },
    );
    let cli = (
        (
            text(),
            prop::collection::vec(text(), 0..3),
            prop::collection::vec(text(), 0..3),
            prop::collection::vec(arg, 0..3),
        ),
        (e(3), e(2), any::<u32>(), any::<u64>(), e(3), any::<i32>()),
        (prop::collection::hash_map(text(), text(), 0..3), text(), any::<bool>()),
    )
        .prop_map(
            |(
                (executable_role, global_args, command_path, args),
                (output, errors, deadline_ms, max_output_bytes, kill_scope, usage_exit_code),
                (env, cwd_pointer, engine),
            )| {
                pb::Binding {
                    kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding {
                        executable_role,
                        global_args,
                        command_path,
                        args,
                        output,
                        errors,
                        deadline_ms,
                        max_output_bytes,
                        kill_scope,
                        usage_exit_code,
                        env,
                        cwd_pointer,
                        state_handle_engine_supplied: engine,
                        ..Default::default()
                    })),
                }
            },
        );
    let other = (text(), text()).prop_map(|(a, b)| pb::Binding {
        kind: Some(pb::binding::Kind::Adapter(pb::AdapterOperationBinding {
            adapter_id: a,
            operation: b,
        })),
    });
    proptest::option::of(prop_oneof![cli, other])
}

fn capability() -> impl Strategy<Value = pb::Capability> {
    let branch = (text(), text(), effects(), prop::collection::vec(signal(), 0..2)).prop_map(
        |(pointer, equals_json, eff, signals)| pb::EffectBranch {
            pointer,
            equals_json,
            effects: Some(eff),
            signals,
        },
    );
    let withheld = (e(14), text()).prop_map(|(reason, hint)| pb::Withheld {
        reason,
        hint,
        evidence_ids: vec![],
    });
    let admission = (
        e(2),
        prop::collection::vec(withheld, 0..2),
        prop::collection::vec(text(), 0..2),
        e(5),
    )
        .prop_map(
            |(state, withheld, verification_record_ids, required_runtime_isolation)| pb::Admission {
                state,
                withheld,
                verification_record_ids,
                required_runtime_isolation,
            },
        );
    let provenance = (
        prop::collection::vec(text(), 0..2),
        prop::collection::hash_map(text(), e(5), 0..3),
    )
        .prop_map(|(native_ids, field_confidence)| pb::Provenance {
            native_ids,
            field_confidence,
            ..Default::default()
        });
    let oracle = (
        text(),
        text(),
        prop::collection::vec((text(), e(5), text()), 0..2),
        any::<u64>(),
        prop::collection::vec(any::<u8>(), 0..6),
    )
        .prop_map(|(cap, args, preds, min_size, magic)| {
            let predicates = preds
                .into_iter()
                .map(|(pointer, op, value_json)| pb::Predicate {
                    pointer,
                    op,
                    value_json,
                })
                .collect();
            if min_size % 2 == 0 {
                pb::Oracle {
                    kind: Some(pb::oracle::Kind::ReadBack(pb::ReadBackOracle {
                        capability_id: cap,
                        args_template_json: args,
                        predicates,
                    })),
                }
            } else {
                pb::Oracle {
                    kind: Some(pb::oracle::Kind::File(pb::FileOracle {
                        path_pointer: cap,
                        magic_prefix: magic,
                        min_size,
                    })),
                }
            }
        });
    (
        (text(), text(), text(), target_text(), target_text(), text(), text()),
        (
            proptest::option::of(effects()),
            prop::collection::vec(signal(), 0..3),
            prop::collection::vec(branch, 0..2),
            e(4),
            e(8),
        ),
        (
            binding(),
            proptest::option::of(admission),
            proptest::option::of(provenance),
            prop::collection::vec(oracle, 0..2),
        ),
        (any::<bool>(), e(3), any::<bool>(), text(), text()),
    )
        .prop_map(
            |(
                (id, target_id, family, title, description, input_schema_json, output_schema_json),
                (effects, signals, branches, level, tier),
                (binding, admission, provenance, oracles),
                (returns_target_content, lifetime, core, mcp_name, capability_digest),
            )| pb::Capability {
                id,
                target_id,
                family,
                title,
                description,
                input_schema_json,
                output_schema_json,
                effects,
                signals,
                branches,
                level,
                tier,
                binding,
                oracles,
                returns_target_content,
                lifetime,
                provenance,
                admission,
                core,
                projections: Some(pb::Projections {
                    mcp_name,
                    ..Default::default()
                }),
                examples: vec![pb::Example {
                    input_json: "{}".into(),
                    fixture_id: String::new(),
                    source: "overlay".into(),
                }],
                capability_digest,
                ..Default::default()
            },
        )
}

fn target() -> impl Strategy<Value = pb::Target> {
    let file = (text(), text(), text(), text()).prop_map(|(role, realpath, sha256, version_banner)| pb::FileIdentity {
        role,
        realpath,
        sha256,
        version_banner,
    });
    let source = (text(), text(), text(), e(4), timestamp(), text()).prop_map(
        |(harvester, uri, sha256, authority, harvested_at, action_id)| pb::SourceRef {
            harvester,
            uri,
            sha256,
            authority,
            harvested_at,
            action_id,
        },
    );
    let step = prop_oneof![
        (text(), prop::collection::vec(any::<u8>(), 0..16)).prop_map(|(path, content)| pb::FixtureStep {
            step: Some(pb::fixture_step::Step::File(pb::FileStep { path, content }))
        }),
        (prop::collection::vec(text(), 1..3), text()).prop_map(|(argv, cwd)| pb::FixtureStep {
            step: Some(pb::fixture_step::Step::Argv(pb::ArgvStep { argv, cwd }))
        }),
        text().prop_map(|id| pb::FixtureStep {
            step: Some(pb::fixture_step::Step::IncludeFixture(id))
        }),
    ];
    let fixture =
        (text(), prop::collection::vec(step, 0..3), text()).prop_map(|(id, steps, ready_capability_id)| pb::Fixture {
            id,
            steps,
            ready_capability_id,
        });
    (
        (text(), text(), e(5), prop::collection::vec(file, 0..2), text()),
        (
            prop::collection::vec(source, 0..2),
            e(3),
            text(),
            prop::collection::vec(fixture, 0..2),
            target_text(),
        ),
    )
        .prop_map(
            |(
                (id, display_name, kind, files, tree),
                (sources, status, authorization_statement, fixtures, description),
            )| {
                pb::Target {
                    id,
                    display_name,
                    kind,
                    identity: Some(pb::TargetIdentity {
                        files,
                        source_tree_sha: tree,
                        ..Default::default()
                    }),
                    license: None,
                    preconditions: vec![],
                    sources,
                    status,
                    authorization_statement,
                    fixtures,
                    mcp_prefix: String::new(),
                    description,
                    families: vec![],
                }
            },
        )
}

fn lock() -> impl Strategy<Value = pb::CapabilityLock> {
    (
        text(),
        proptest::option::of(target()),
        prop::collection::vec(capability(), 0..3),
        prop::collection::hash_map(text(), text(), 0..4),
        text(),
        text(),
    )
        .prop_map(
            |(schema, target, capabilities, rule_versions, overlay_sha256, lock_digest)| pb::CapabilityLock {
                schema,
                target,
                capabilities,
                rule_versions,
                overlay_sha256,
                lock_digest,
            },
        )
}

fn principal() -> impl Strategy<Value = pb::Principal> {
    (e(3), e(5), any::<u32>(), any::<i32>(), text()).prop_map(|(role, channel, uid, pid, exe_path)| pb::Principal {
        role,
        channel,
        uid,
        pid,
        exe_path,
        ..Default::default()
    })
}

fn decision() -> impl Strategy<Value = pb::PolicyDecision> {
    (e(2), prop::collection::vec(text(), 0..2), text(), e(9), e(4), e(2)).prop_map(
        |(verdict, matched_rule_ids, deciding_rule_id, denial_reason, approval_source, assurance)| pb::PolicyDecision {
            verdict,
            matched_rule_ids,
            deciding_rule_id,
            denial_reason,
            approval_source,
            assurance,
            ..Default::default()
        },
    )
}

fn audit_entry() -> impl Strategy<Value = pb::AuditEntry> {
    let intent = (text(), text(), e(4), proptest::option::of(decision()), text()).prop_map(
        |(action_id, capability_id, level, decision, idempotency_key)| {
            pb::audit_entry::Body::Intent(pb::ActionIntent {
                action_id,
                capability_id,
                level,
                decision,
                idempotency_key,
                ..Default::default()
            })
        },
    );
    let error = (e(29), e(9), text(), any::<bool>(), text(), target_text()).prop_map(
        |(code, category, message, retryable, remediation, upstream)| pb::Error {
            code,
            category,
            message,
            retryable,
            remediation,
            upstream,
            ..Default::default()
        },
    );
    let process = (
        any::<i32>(),
        any::<u64>(),
        target_text(),
        any::<u32>(),
        e(3),
        prop::collection::vec(text(), 0..2),
    )
        .prop_map(
            |(exit_code, stdout_bytes, stderr_excerpt, wall_ms, cleanup, confinement_denials)| pb::ProcessReport {
                exit_code,
                stdout_bytes,
                stderr_excerpt,
                wall_ms,
                cleanup,
                confinement_denials,
                ..Default::default()
            },
        );
    let observed =
        (text(), text(), e(3), text()).prop_map(|(path, root, change, after_sha256)| pb::ObservedFileEffect {
            path,
            root,
            change,
            after_sha256,
            ..Default::default()
        });
    let action = (
        (
            text(),
            text(),
            text(),
            any::<bool>(),
            proptest::option::of(effects()),
            e(4),
            e(8),
        ),
        (
            proptest::option::of(decision()),
            e(5),
            proptest::option::of(principal()),
            prop::collection::vec(observed, 0..2),
            e(5),
        ),
        (
            proptest::option::of(process),
            proptest::option::of(error),
            timestamp(),
            timestamp(),
            any::<bool>(),
        ),
    )
        .prop_map(
            |(
                (action_id, capability_id, args_commitment, dry_run, declared_effects, level, tier),
                (decision, runtime_isolation, principal, observed_files, outcome),
                (process, error, requested_at, completed_at, recovered),
            )| {
                pb::audit_entry::Body::Action(pb::ActionRecord {
                    action_id,
                    capability_id,
                    args_commitment,
                    dry_run,
                    declared_effects,
                    level,
                    tier,
                    decision,
                    runtime_isolation,
                    principal,
                    observed_files,
                    outcome,
                    process,
                    error,
                    requested_at,
                    completed_at,
                    recovered,
                    ..Default::default()
                })
            },
        );
    let admin = (e(13), text(), text(), target_text()).prop_map(|(kind, subject, details_json, operator_note)| {
        pb::audit_entry::Body::Admin(pb::AdminEvent {
            kind,
            subject,
            details_json,
            operator_note,
        })
    });
    (
        any::<u64>(),
        text(),
        text(),
        timestamp(),
        proptest::option::of(principal()),
        any::<u64>(),
        proptest::option::of(prop_oneof![intent, action, admin]),
    )
        .prop_map(
            |(seq, prev_sha256, entry_sha256, written_at, principal, halt_epoch, body)| pb::AuditEntry {
                seq,
                prev_sha256,
                entry_sha256,
                written_at,
                principal,
                halt_epoch,
                body,
            },
        )
}

fn assert_round_trip<T>(msg: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let canonical = jcs(msg).expect("jcs");
    let back: T = serde_json::from_slice(&canonical).expect("proto3-JSON parses back");
    assert_eq!(&back, msg, "message equality after round trip");
    assert_eq!(
        jcs(&back).expect("jcs"),
        canonical,
        "JCS bytes identical after round trip"
    );
    // Non-canonical (serde_json) bytes parse to the same canonical form too.
    let loose: T = serde_json::from_slice(&serde_json::to_vec_pretty(msg).expect("json")).expect("parse");
    assert_eq!(jcs(&loose).expect("jcs"), canonical);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    #[test]
    fn capability_lock_round_trips(l in lock()) {
        assert_round_trip(&l);
    }

    #[test]
    fn audit_entry_round_trips(a in audit_entry()) {
        assert_round_trip(&a);
    }
}
