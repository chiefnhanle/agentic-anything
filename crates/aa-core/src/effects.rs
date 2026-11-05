//! Effects lattice, lowering table and projections (architecture §6.2-§6.3, ADR 0011).
//!
//! - Every field is tri-state. Target-facing fields and `mutates_session` combine by **max** over admissible
//!   signals in the order `UNKNOWN < FALSE < TRUE`; `idempotent` (safe direction TRUE) combines by AND, where an
//!   UNKNOWN-valued signal counts as FALSE and "no admissible signal" stays UNKNOWN.
//! - A FALSE (for `idempotent`: a TRUE) counts only from an admissible source ([`admissible`]).
//! - `level`, `tier` and `hints` are pure, monotone projections; UNKNOWN is always the most restrictive reading.

use crate::{AaError, AaResult, pb};

/// The 16 target-facing `Effects` field names (proto field names, declaration order).
pub const TARGET_FIELDS: &[&str] = &[
    "mutates_target",
    "writes_filesystem",
    "launches_process",
    "accesses_network",
    "changes_ui_state",
    "sends_input",
    "may_discard_data",
    "executes_foreign_code",
    "mutates_device",
    "external_side_effect",
    "attaches_process",
    "reads_screen",
    "reads_clipboard",
    "writes_clipboard",
    "reads_credentials",
    "irreversible",
];

/// The control-plane field (never in `readOnlyHint`).
pub const MUTATES_SESSION: &str = "mutates_session";
/// The safe-direction field (AND-combined).
pub const IDEMPOTENT: &str = "idempotent";
/// All 18 field names: the target-facing ones, then `mutates_session`, then `idempotent`.
pub const ALL_FIELDS: &[&str] = &[
    "mutates_target",
    "writes_filesystem",
    "launches_process",
    "accesses_network",
    "changes_ui_state",
    "sends_input",
    "may_discard_data",
    "executes_foreign_code",
    "mutates_device",
    "external_side_effect",
    "attaches_process",
    "reads_screen",
    "reads_clipboard",
    "writes_clipboard",
    "reads_credentials",
    "irreversible",
    MUTATES_SESSION,
    IDEMPOTENT,
];

/// Any TRUE makes the level DESTRUCTIVE (architecture §6.3).
pub const DESTRUCTIVE_FIELDS: &[&str] = &[
    "may_discard_data",
    "irreversible",
    "executes_foreign_code",
    "mutates_device",
    "external_side_effect",
    "attaches_process",
    "reads_credentials",
];
/// Any TRUE (and no destructive field) makes the level WRITE.
pub const WRITE_FIELDS: &[&str] = &[
    "mutates_target",
    "writes_filesystem",
    "launches_process",
    "accesses_network",
    "writes_clipboard",
];
/// Any TRUE (and nothing above) makes the level UI.
pub const UI_FIELDS: &[&str] = &["changes_ui_state", "sends_input"];
/// Any field here that is not FALSE sets `openWorldHint`.
pub const OPEN_WORLD_FIELDS: &[&str] = &["launches_process", "accesses_network", "external_side_effect"];
/// Fields a path-typed/output-like WRITE parameter makes impossible to lower from lanes (ADR 0011).
const WRITE_PATH_BLOCKED: &[&str] = &["writes_filesystem", "may_discard_data", "irreversible"];

fn slot<'a>(e: &'a pb::Effects, field: &str) -> Option<&'a i32> {
    Some(match field {
        "mutates_target" => &e.mutates_target,
        "writes_filesystem" => &e.writes_filesystem,
        "launches_process" => &e.launches_process,
        "accesses_network" => &e.accesses_network,
        "changes_ui_state" => &e.changes_ui_state,
        "sends_input" => &e.sends_input,
        "may_discard_data" => &e.may_discard_data,
        "executes_foreign_code" => &e.executes_foreign_code,
        "mutates_device" => &e.mutates_device,
        "external_side_effect" => &e.external_side_effect,
        "attaches_process" => &e.attaches_process,
        "reads_screen" => &e.reads_screen,
        "reads_clipboard" => &e.reads_clipboard,
        "writes_clipboard" => &e.writes_clipboard,
        "reads_credentials" => &e.reads_credentials,
        "irreversible" => &e.irreversible,
        "mutates_session" => &e.mutates_session,
        "idempotent" => &e.idempotent,
        _ => return None,
    })
}

fn slot_mut<'a>(e: &'a mut pb::Effects, field: &str) -> Option<&'a mut i32> {
    Some(match field {
        "mutates_target" => &mut e.mutates_target,
        "writes_filesystem" => &mut e.writes_filesystem,
        "launches_process" => &mut e.launches_process,
        "accesses_network" => &mut e.accesses_network,
        "changes_ui_state" => &mut e.changes_ui_state,
        "sends_input" => &mut e.sends_input,
        "may_discard_data" => &mut e.may_discard_data,
        "executes_foreign_code" => &mut e.executes_foreign_code,
        "mutates_device" => &mut e.mutates_device,
        "external_side_effect" => &mut e.external_side_effect,
        "attaches_process" => &mut e.attaches_process,
        "reads_screen" => &mut e.reads_screen,
        "reads_clipboard" => &mut e.reads_clipboard,
        "writes_clipboard" => &mut e.writes_clipboard,
        "reads_credentials" => &mut e.reads_credentials,
        "irreversible" => &mut e.irreversible,
        "mutates_session" => &mut e.mutates_session,
        "idempotent" => &mut e.idempotent,
        _ => return None,
    })
}

/// A raw enum number as a `Tri`; out-of-range numbers read as UNKNOWN (fail-safe).
pub fn tri_of(raw: i32) -> pb::Tri {
    pb::Tri::try_from(raw).unwrap_or(pb::Tri::Unknown)
}

/// Whether `field` names one of the 18 `Effects` fields.
pub fn is_field(field: &str) -> bool {
    ALL_FIELDS.contains(&field)
}

/// The value of `field`, or `None` if it is not an `Effects` field name.
pub fn get(e: &pb::Effects, field: &str) -> Option<pb::Tri> {
    slot(e, field).map(|v| tri_of(*v))
}

/// Sets `field`; `INVALID_REQUEST` if it is not an `Effects` field name.
pub fn set(e: &mut pb::Effects, field: &str, v: pb::Tri) -> crate::AaResult<()> {
    match slot_mut(e, field) {
        Some(s) => {
            *s = v as i32;
            Ok(())
        }
        None => Err(AaError::invalid(format!("unknown effect field {field:?}"))),
    }
}

/// Lattice rank used by `combine` (adding a signal never lowers it):
/// target fields and `mutates_session`: UNKNOWN 0 < FALSE 1 < TRUE 2;
/// `idempotent`: UNKNOWN 0 < TRUE 1 < FALSE 2 (FALSE is absorbing under AND).
pub fn lattice_rank(field: &str, v: pb::Tri) -> u8 {
    match (field == IDEMPOTENT, v) {
        (_, pb::Tri::Unknown) => 0,
        (false, pb::Tri::False) | (true, pb::Tri::True) => 1,
        (false, pb::Tri::True) | (true, pb::Tri::False) => 2,
    }
}

/// Risk rank used by the projections (higher = riskier; UNKNOWN is evaluated as the worst case):
/// target fields: FALSE 0 < TRUE 1 < UNKNOWN 2; `idempotent`: TRUE 0 < FALSE 1 < UNKNOWN 2.
pub fn risk_rank(field: &str, v: pb::Tri) -> u8 {
    match (field == IDEMPOTENT, v) {
        (_, pb::Tri::Unknown) => 2,
        (false, pb::Tri::False) | (true, pb::Tri::True) => 0,
        (false, pb::Tri::True) | (true, pb::Tri::False) => 1,
    }
}

/// Numeric strength of an isolation level (NOT_PROVEN weakest).
pub fn isolation_rank(i: pb::Isolation) -> u8 {
    match i {
        pb::Isolation::NotProven => 0,
        pb::Isolation::None => 1,
        pb::Isolation::Process => 2,
        pb::Isolation::Confined => 3,
        pb::Isolation::Container => 4,
        pb::Isolation::VirtualMachine => 5,
    }
}

/// Lowering table (ADR 0011). None => lanes can never lower this field.
///
/// | field | minimum lane isolation for an OBSERVED FALSE |
/// |---|---|
/// | `mutates_target` | PROCESS (private-root tree diff + state-handle hash) |
/// | `writes_filesystem` | CONFINED (out-of-root writes are denied); never with a WRITE path param |
/// | `may_discard_data`, `irreversible` | CONFINED (no write anywhere succeeded); never with a WRITE path param |
/// | `accesses_network`, `external_side_effect` | CONFINED (network denied) |
/// | `launches_process`, `executes_foreign_code` | CONFINED (exec allow-list = pinned binding files) |
/// | `mutates_device` | CONFINED (device writes are denied) |
/// | `reads_credentials` | CONFINED (private HOME, scrubbed env, keychain helpers not on the exec allow-list) |
/// | UI, screen, clipboard, `attaches_process`, `mutates_session`, `idempotent` | never |
pub fn min_isolation_to_lower(field: &str, has_write_path_param: bool) -> Option<pb::Isolation> {
    if has_write_path_param && WRITE_PATH_BLOCKED.contains(&field) {
        return None;
    }
    match field {
        "mutates_target" => Some(pb::Isolation::Process),
        "writes_filesystem"
        | "may_discard_data"
        | "irreversible"
        | "accesses_network"
        | "external_side_effect"
        | "launches_process"
        | "executes_foreign_code"
        | "mutates_device"
        | "reads_credentials" => Some(pb::Isolation::Confined),
        _ => None,
    }
}

/// Whether a signal may count in [`combine`] (architecture §6.2).
///
/// - Raises always count: TRUE (and UNKNOWN, a no-op) on target fields and `mutates_session`; FALSE or UNKNOWN on
///   `idempotent`.
/// - A lowering FALSE counts only from `OBSERVED` (source `verify:*`, at an isolation the lowering table accepts),
///   `DECLARED`, or `OPERATOR` (source `operator:*`). INFERRED, DERIVED and PROPOSED FALSE are recorded and ignored.
///   With a WRITE path parameter, `writes_filesystem` is never lowered by any source.
/// - An `idempotent` TRUE counts only from `OBSERVED` (≥ PROCESS, `verify:*`), `DECLARED` or `OPERATOR`.
pub fn admissible(sig: &pb::EffectSignal, has_write_path_param: bool) -> bool {
    if !is_field(&sig.field) {
        return false;
    }
    let value = tri_of(sig.value);
    let confidence = pb::Confidence::try_from(sig.confidence).unwrap_or(pb::Confidence::Proposed);
    let isolation = pb::Isolation::try_from(sig.isolation).unwrap_or(pb::Isolation::NotProven);
    let lowering = matches!(
        (sig.field == IDEMPOTENT, value),
        (true, pb::Tri::True) | (false, pb::Tri::False)
    );
    if !lowering {
        return true;
    }
    if sig.field == "writes_filesystem" && has_write_path_param {
        return false;
    }
    match confidence {
        pb::Confidence::Observed => {
            if !sig.source.starts_with("verify:") {
                return false;
            }
            let min = if sig.field == IDEMPOTENT {
                Some(pb::Isolation::Process)
            } else {
                min_isolation_to_lower(&sig.field, has_write_path_param)
            };
            min.is_some_and(|m| isolation_rank(isolation) >= isolation_rank(m))
        }
        pb::Confidence::Operator => sig.source.starts_with("operator:"),
        pb::Confidence::Declared => true,
        pb::Confidence::Proposed | pb::Confidence::Inferred | pb::Confidence::Derived => false,
    }
}

fn max_tri(field: &str, a: pb::Tri, b: pb::Tri) -> pb::Tri {
    if lattice_rank(field, b) > lattice_rank(field, a) {
        b
    } else {
        a
    }
}

/// Pointwise lattice join: max for target fields and `mutates_session`; for `idempotent` UNKNOWN is the identity,
/// FALSE is absorbing and TRUE ∧ TRUE = TRUE. `combine(a ++ b) == join(combine(a), combine(b))`.
pub fn join(a: &pb::Effects, b: &pb::Effects) -> pb::Effects {
    let mut out = *a;
    for f in ALL_FIELDS {
        let (x, y) = (get(a, f).unwrap_or_default(), get(b, f).unwrap_or_default());
        // Both orders agree because the lattice is total per field.
        let _ = set(&mut out, f, max_tri(f, x, y));
    }
    out
}

/// Max-combine over admissible signals; idempotent combines by AND (UNKNOWN => FALSE).
pub fn combine(signals: &[pb::EffectSignal], has_write_path_param: bool) -> pb::Effects {
    let mut out = pb::Effects::default();
    for sig in signals.iter().filter(|s| admissible(s, has_write_path_param)) {
        let mut v = tri_of(sig.value);
        if sig.field == IDEMPOTENT && v == pb::Tri::Unknown {
            v = pb::Tri::False;
        }
        let cur = get(&out, &sig.field).unwrap_or_default();
        let _ = set(&mut out, &sig.field, max_tri(&sig.field, cur, v));
    }
    out
}

/// Whether `v` is "present and truthy" (not null/false/0/""/[]/{}).
fn truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
    }
}

/// Whether a branch's condition matches `input`: `equals_json == ""` means "present and truthy", otherwise the
/// value at `pointer` must equal the parsed `equals_json` (an unparsable `equals_json` matches whenever the
/// pointer is present, the conservative reading).
pub fn branch_matches(branch: &pb::EffectBranch, input: &serde_json::Value) -> bool {
    let Some(v) = input.pointer(&branch.pointer) else {
        return false;
    };
    if branch.equals_json.is_empty() {
        return truthy(v);
    }
    match serde_json::from_str::<serde_json::Value>(&branch.equals_json) {
        Ok(expected) => &expected == v,
        Err(_) => true,
    }
}

/// An unresolved branch (no effect vector: the value switches authority, e.g. `gh api -X`).
pub fn branch_unresolved(branch: &pb::EffectBranch) -> bool {
    branch.effects.is_none()
}

/// The worst case: every target field and `mutates_session` TRUE, `idempotent` FALSE.
pub fn worst_case() -> pb::Effects {
    let mut e = pb::Effects::default();
    for f in TARGET_FIELDS.iter().chain([&MUTATES_SESSION]) {
        let _ = set(&mut e, f, pb::Tri::True);
    }
    e.idempotent = pb::Tri::False as i32;
    e
}

/// Joins the effects of every branch whose condition matches `input` into `base` (raise only). A matching
/// unresolved branch raises to [`worst_case`].
pub fn apply_branches(base: &pb::Effects, branches: &[pb::EffectBranch], input: &serde_json::Value) -> pb::Effects {
    branches
        .iter()
        .filter(|b| branch_matches(b, input))
        .fold(*base, |acc, b| match &b.effects {
            Some(e) => join(&acc, e),
            None => join(&acc, &worst_case()),
        })
}

fn is(e: &pb::Effects, field: &str, v: pb::Tri) -> bool {
    get(e, field) == Some(v)
}

/// The target-facing fields that are still UNKNOWN.
pub fn unknown_target_fields(e: &pb::Effects) -> Vec<&'static str> {
    TARGET_FIELDS
        .iter()
        .copied()
        .filter(|f| is(e, f, pb::Tri::Unknown))
        .collect()
}

pub fn level(e: &pb::Effects) -> pb::EffectLevel {
    let any_true = |fields: &[&str]| fields.iter().any(|f| is(e, f, pb::Tri::True));
    if TARGET_FIELDS.iter().any(|f| is(e, f, pb::Tri::Unknown)) {
        pb::EffectLevel::Unknown
    } else if any_true(DESTRUCTIVE_FIELDS) {
        pb::EffectLevel::Destructive
    } else if any_true(WRITE_FIELDS) {
        pb::EffectLevel::Write
    } else if any_true(UI_FIELDS) {
        pb::EffectLevel::Ui
    } else {
        pb::EffectLevel::Read
    }
}

/// Risk rank of a level (UNKNOWN is the highest: never servable).
pub fn level_rank(l: pb::EffectLevel) -> u8 {
    match l {
        pb::EffectLevel::Read => 1,
        pb::EffectLevel::Ui => 2,
        pb::EffectLevel::Write => 3,
        pb::EffectLevel::Destructive => 4,
        pb::EffectLevel::Unknown => 5,
    }
}

/// Whether an adapter binding is a dynamic-instrumentation adapter (tier INJECT).
fn is_instrumentation(binding: &pb::Binding) -> bool {
    match &binding.kind {
        Some(pb::binding::Kind::Adapter(a)) => {
            let s = format!("{} {}", a.adapter_id, a.operation).to_ascii_lowercase();
            ["instrument", "inject", "frida", "dtrace"]
                .iter()
                .any(|k| s.contains(k))
        }
        _ => false,
    }
}

pub fn tier(e: &pb::Effects, binding: &pb::Binding) -> pb::AuthorityTier {
    let lvl = level(e);
    if lvl == pb::EffectLevel::Unknown {
        return pb::AuthorityTier::Unknown;
    }
    if is_instrumentation(binding) {
        return pb::AuthorityTier::Inject;
    }
    if is(e, "attaches_process", pb::Tri::True) {
        return pb::AuthorityTier::Attach;
    }
    match lvl {
        pb::EffectLevel::Destructive => pb::AuthorityTier::Destructive,
        pb::EffectLevel::Write => {
            let only_launch = TARGET_FIELDS
                .iter()
                .all(|f| *f == "launches_process" || is(e, f, pb::Tri::False));
            if only_launch {
                pb::AuthorityTier::Launch
            } else {
                pb::AuthorityTier::Write
            }
        }
        pb::EffectLevel::Ui => pb::AuthorityTier::Drive,
        pb::EffectLevel::Read => pb::AuthorityTier::Observe,
        pb::EffectLevel::Unknown => pb::AuthorityTier::Unknown,
    }
}

/// REA's `annotationsFromEffects` over target-facing fields (UNKNOWN counts as TRUE).
pub fn hints(e: &pb::Effects) -> pb::McpHints {
    let not_false = |fields: &[&str]| fields.iter().any(|f| !is(e, f, pb::Tri::False));
    pb::McpHints {
        read_only: level(e) == pb::EffectLevel::Read,
        destructive: not_false(DESTRUCTIVE_FIELDS),
        idempotent: is(e, IDEMPOTENT, pb::Tri::True),
        open_world: not_false(OPEN_WORLD_FIELDS),
    }
}

/// Evaluation-time conservatism: UNKNOWN tier => highest, UNKNOWN level => never servable.
pub fn tier_rank(t: pb::AuthorityTier) -> u8 {
    match t {
        pb::AuthorityTier::Observe => 1,
        pb::AuthorityTier::Launch => 2,
        pb::AuthorityTier::Drive => 3,
        pb::AuthorityTier::Write => 4,
        pb::AuthorityTier::Destructive => 5,
        pb::AuthorityTier::Attach => 6,
        pb::AuthorityTier::Inject => 7,
        pb::AuthorityTier::MutateSystem => 8,
        pb::AuthorityTier::Unknown => 9,
    }
}

/// Parses an effect field name, rejecting anything outside the 18 fields.
pub fn field_name(field: &str) -> AaResult<&'static str> {
    ALL_FIELDS
        .iter()
        .copied()
        .find(|f| *f == field)
        .ok_or_else(|| AaError::invalid(format!("unknown effect field {field:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(field: &str, v: pb::Tri, c: pb::Confidence, source: &str, iso: pb::Isolation) -> pb::EffectSignal {
        pb::EffectSignal {
            field: field.into(),
            value: v as i32,
            confidence: c as i32,
            source: source.into(),
            isolation: iso as i32,
            ..Default::default()
        }
    }

    fn all(v: pb::Tri) -> pb::Effects {
        let mut e = pb::Effects::default();
        for f in ALL_FIELDS {
            set(&mut e, f, v).unwrap();
        }
        e
    }

    #[test]
    fn get_set_round_trip_and_unknown_field() {
        let mut e = pb::Effects::default();
        for f in ALL_FIELDS {
            assert_eq!(get(&e, f), Some(pb::Tri::Unknown));
            set(&mut e, f, pb::Tri::True).unwrap();
            assert_eq!(get(&e, f), Some(pb::Tri::True));
        }
        assert_eq!(get(&e, "nope"), None);
        assert!(set(&mut e, "nope", pb::Tri::True).is_err());
        assert_eq!(TARGET_FIELDS.len(), 16);
        assert_eq!(ALL_FIELDS.len(), 18);
    }

    #[test]
    fn projections_basic() {
        assert_eq!(level(&pb::Effects::default()), pb::EffectLevel::Unknown);
        let mut e = all(pb::Tri::False);
        assert_eq!(level(&e), pb::EffectLevel::Read);
        let b = pb::Binding::default();
        assert_eq!(tier(&e, &b), pb::AuthorityTier::Observe);
        assert!(hints(&e).read_only);
        e.launches_process = pb::Tri::True as i32;
        assert_eq!(level(&e), pb::EffectLevel::Write);
        assert_eq!(tier(&e, &b), pb::AuthorityTier::Launch);
        e.writes_filesystem = pb::Tri::True as i32;
        assert_eq!(tier(&e, &b), pb::AuthorityTier::Write);
        e.may_discard_data = pb::Tri::True as i32;
        assert_eq!(level(&e), pb::EffectLevel::Destructive);
        e.attaches_process = pb::Tri::True as i32;
        assert_eq!(tier(&e, &b), pb::AuthorityTier::Attach);
        assert!(hints(&e).destructive);
        assert!(hints(&e).open_world);
    }

    #[test]
    fn lowering_needs_isolation() {
        let lo = |f: &str, iso| sig(f, pb::Tri::False, pb::Confidence::Observed, "verify:vr_1", iso);
        assert!(!admissible(&lo("accesses_network", pb::Isolation::Process), false));
        assert!(admissible(&lo("accesses_network", pb::Isolation::Confined), false));
        assert!(admissible(&lo("mutates_target", pb::Isolation::Process), false));
        assert!(!admissible(&lo("writes_filesystem", pb::Isolation::Confined), true));
        assert!(!admissible(&lo("reads_screen", pb::Isolation::VirtualMachine), false));
        let forged = sig(
            "accesses_network",
            pb::Tri::False,
            pb::Confidence::Observed,
            "proposal:x",
            pb::Isolation::Confined,
        );
        assert!(!admissible(&forged, false));
        let inferred = sig(
            "accesses_network",
            pb::Tri::False,
            pb::Confidence::Inferred,
            "rule:x",
            pb::Isolation::Confined,
        );
        assert!(!admissible(&inferred, false));
    }

    #[test]
    fn idempotent_and() {
        let t = sig(
            IDEMPOTENT,
            pb::Tri::True,
            pb::Confidence::Observed,
            "verify:a",
            pb::Isolation::Process,
        );
        let f = sig(
            IDEMPOTENT,
            pb::Tri::False,
            pb::Confidence::Inferred,
            "rule:x",
            pb::Isolation::NotProven,
        );
        let u = sig(
            IDEMPOTENT,
            pb::Tri::Unknown,
            pb::Confidence::Inferred,
            "rule:x",
            pb::Isolation::NotProven,
        );
        assert_eq!(combine(&[], false).idempotent, pb::Tri::Unknown as i32);
        assert_eq!(
            combine(std::slice::from_ref(&t), false).idempotent,
            pb::Tri::True as i32
        );
        assert_eq!(combine(&[t.clone(), f], false).idempotent, pb::Tri::False as i32);
        assert_eq!(combine(&[t, u], false).idempotent, pb::Tri::False as i32);
    }

    #[test]
    fn branches_raise() {
        let base = all(pb::Tri::False);
        let br = pb::EffectBranch {
            pointer: "/force".into(),
            effects: Some(pb::Effects {
                may_discard_data: pb::Tri::True as i32,
                ..Default::default()
            }),
            ..Default::default()
        };
        let out = apply_branches(&base, std::slice::from_ref(&br), &serde_json::json!({"force": true}));
        assert_eq!(level(&out), pb::EffectLevel::Destructive);
        let out = apply_branches(&base, std::slice::from_ref(&br), &serde_json::json!({"force": false}));
        assert_eq!(level(&out), pb::EffectLevel::Read);
        let eq = pb::EffectBranch {
            pointer: "/m".into(),
            equals_json: "\"POST\"".into(),
            effects: None,
            ..Default::default()
        };
        assert_eq!(
            apply_branches(&base, std::slice::from_ref(&eq), &serde_json::json!({"m": "GET"})),
            base
        );
        let out = apply_branches(&base, &[eq], &serde_json::json!({"m": "POST"}));
        assert_eq!(out, worst_case());
    }
}
