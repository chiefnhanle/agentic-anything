//! Policy evaluator: hard exclusions first, then all matching rules, strictest wins, default deny (§5.5).
//!
//! The file format is `aa-policy/1` (TOML). Unknown keys, unknown level/tier/effect names and duplicate rule ids
//! are rejected, because a typo in a condition would silently widen an allow rule.
//!
//! Matching uses structured descriptors only, never command text. Each `when` condition yields yes / no / maybe
//! (maybe = the descriptor is UNKNOWN on that axis). An `allow` rule matches only if every condition is yes; a
//! `deny` or `ask` rule matches unless some condition is no, so uncertainty always resolves to the stricter rule.

pub mod exclusions;
pub mod paths;

use crate::{AaError, AaResult, effects, pb};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The built-in policy used when no policy file exists: deny everything except observe-tier actions on
/// registered targets.
pub const DEFAULT_POLICY_TOML: &str = r#"format = "aa-policy/1"
default = "deny"

[paths]
deny = ["~/.ssh", "~/.aws", "~/.gnupg", "~/.config/agentic-anything", "~/Library/Keychains"]

[[rule]]
id = "observe-registered"
effect = "allow"
when = { tier = "observe", target_registered = true }
"#;

/// The `format` value this evaluator understands.
pub const FORMAT: &str = "aa-policy/1";

/// Parsed `aa-policy/1`.
#[derive(Debug, Clone)]
pub struct Policy {
    pub format: String,
    /// `policy_digest` of the source text.
    pub digest: String,
    pub paths: PolicyPaths,
    pub rules: Vec<Rule>,
}

/// `[paths]`, with `~` expanded against `$HOME` at parse time (roots are not canonicalized here; see
/// [`paths::check`]).
#[derive(Debug, Clone, Default)]
pub struct PolicyPaths {
    pub write_roots: Vec<PathBuf>,
    pub read_roots: Vec<PathBuf>,
    pub deny: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    /// ALLOW, ASK or DENY.
    pub effect: pb::Verdict,
    pub when: When,
    /// Approval sources acceptable for an `ask` rule (empty = operator socket only).
    pub approval_sources: Vec<pb::ApprovalSource>,
    /// Minimum assurance tier; below it the rule contributes DENY with `TIER_TOO_LOW`.
    pub require_tier: Option<pb::AssuranceTier>,
    pub reason: Option<String>,
}

/// Conjunction of conditions; an absent condition always matches.
#[derive(Debug, Clone, Default)]
pub struct When {
    pub levels: Vec<pb::EffectLevel>,
    pub min_level: Option<pb::EffectLevel>,
    pub max_level: Option<pb::EffectLevel>,
    pub tiers: Vec<pb::AuthorityTier>,
    pub min_tier: Option<pb::AuthorityTier>,
    pub max_tier: Option<pb::AuthorityTier>,
    pub targets: Vec<globset::GlobMatcher>,
    pub capabilities: Vec<globset::GlobMatcher>,
    pub target_registered: Option<bool>,
    pub session_tainted: Option<bool>,
    /// Matches if any listed field is TRUE (UNKNOWN = maybe).
    pub any_effect: Vec<&'static str>,
    /// Matches if every listed field is FALSE (UNKNOWN = maybe).
    pub no_effect: Vec<&'static str>,
    pub assurance: Vec<pb::AssuranceTier>,
}

impl Policy {
    /// The session-independent part of the PathScope (the engine adds the session workspace as a write root).
    pub fn path_scope(&self) -> paths::PathScope {
        paths::PathScope {
            write_roots: self.paths.write_roots.clone(),
            read_roots: self.paths.read_roots.clone(),
            deny: self.paths.deny.clone(),
        }
    }

    pub fn rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// Approval sources acceptable for the decision's deciding `ask` rule (operator socket by default).
    pub fn acceptable_sources(&self, decision: &pb::PolicyDecision) -> Vec<pb::ApprovalSource> {
        match self.rule(&decision.deciding_rule_id) {
            Some(r) if !r.approval_sources.is_empty() => r.approval_sources.clone(),
            _ => vec![pb::ApprovalSource::OperatorSocket],
        }
    }
}

// ---------------------------------------------------------------------------
// File format
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    format: String,
    #[serde(default = "default_deny")]
    default: String,
    #[serde(default)]
    paths: PathsFile,
    #[serde(default, rename = "rule")]
    rules: Vec<RuleFile>,
}

fn default_deny() -> String {
    "deny".into()
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PathsFile {
    #[serde(default)]
    write_roots: Vec<String>,
    #[serde(default)]
    read_roots: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    id: String,
    effect: String,
    #[serde(default)]
    when: WhenFile,
    #[serde(default)]
    approval_sources: Vec<String>,
    require_tier: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct WhenFile {
    level: Option<OneOrMany>,
    min_level: Option<String>,
    max_level: Option<String>,
    tier: Option<OneOrMany>,
    min_tier: Option<String>,
    max_tier: Option<String>,
    targets: Option<OneOrMany>,
    capabilities: Option<OneOrMany>,
    target_registered: Option<bool>,
    session_tainted: Option<bool>,
    any_effect: Option<Vec<String>>,
    no_effect: Option<Vec<String>>,
    assurance: Option<OneOrMany>,
}

fn bad(msg: impl Into<String>) -> AaError {
    AaError::new(
        pb::ErrorCode::ConfigurationInvalid,
        msg,
        "fix the operator policy file (format aa-policy/1) and restart the engine: aa status",
    )
}

/// `read`, `READ` or `EFFECT_LEVEL_READ`.
pub fn parse_level(s: &str) -> AaResult<pb::EffectLevel> {
    let name = format!(
        "EFFECT_LEVEL_{}",
        s.trim().to_ascii_uppercase().trim_start_matches("EFFECT_LEVEL_")
    );
    pb::EffectLevel::from_str_name(&name).ok_or_else(|| bad(format!("unknown level {s:?}")))
}

/// `observe`, `OBSERVE` or `AUTHORITY_TIER_OBSERVE`.
pub fn parse_tier(s: &str) -> AaResult<pb::AuthorityTier> {
    let up = s.trim().to_ascii_uppercase().replace('-', "_");
    let name = format!("AUTHORITY_TIER_{}", up.trim_start_matches("AUTHORITY_TIER_"));
    pb::AuthorityTier::from_str_name(&name).ok_or_else(|| bad(format!("unknown tier {s:?}")))
}

/// `same_principal`, `SEPARATED_AGENT`, `ASSURANCE_TIER_ISOLATED_TARGET`, or `0`/`1`/`2`.
pub fn parse_assurance(s: &str) -> AaResult<pb::AssuranceTier> {
    let up = s.trim().to_ascii_uppercase().replace('-', "_");
    let name = match up.as_str() {
        "0" | "TIER0" | "TIER_0" => "ASSURANCE_TIER_SAME_PRINCIPAL".to_string(),
        "1" | "TIER1" | "TIER_1" => "ASSURANCE_TIER_SEPARATED_AGENT".to_string(),
        "2" | "TIER2" | "TIER_2" => "ASSURANCE_TIER_ISOLATED_TARGET".to_string(),
        _ => format!("ASSURANCE_TIER_{}", up.trim_start_matches("ASSURANCE_TIER_")),
    };
    pb::AssuranceTier::from_str_name(&name).ok_or_else(|| bad(format!("unknown assurance tier {s:?}")))
}

fn parse_source(s: &str) -> AaResult<pb::ApprovalSource> {
    let up = s.trim().to_ascii_uppercase();
    let name = format!("APPROVAL_SOURCE_{}", up.trim_start_matches("APPROVAL_SOURCE_"));
    match pb::ApprovalSource::from_str_name(&name) {
        Some(pb::ApprovalSource::None) | None => Err(bad(format!("unknown approval source {s:?}"))),
        Some(src) => Ok(src),
    }
}

fn parse_verdict(s: &str) -> AaResult<pb::Verdict> {
    match s.trim().to_ascii_lowercase().as_str() {
        "allow" => Ok(pb::Verdict::Allow),
        "ask" => Ok(pb::Verdict::Ask),
        "deny" => Ok(pb::Verdict::Deny),
        _ => Err(bad(format!("unknown rule effect {s:?} (allow | ask | deny)"))),
    }
}

fn globs(v: Option<OneOrMany>) -> AaResult<Vec<globset::GlobMatcher>> {
    v.map(OneOrMany::into_vec)
        .unwrap_or_default()
        .iter()
        .map(|g| {
            globset::Glob::new(g)
                .map(|g| g.compile_matcher())
                .map_err(|e| bad(format!("bad glob {g:?}: {e}")))
        })
        .collect()
}

fn fields(v: Option<Vec<String>>) -> AaResult<Vec<&'static str>> {
    v.unwrap_or_default()
        .iter()
        .map(|f| effects::field_name(f).map_err(|_| bad(format!("unknown effect field {f:?}"))))
        .collect()
}

fn expand(p: &str, home: Option<&Path>) -> AaResult<PathBuf> {
    let path = if p == "~" || p.starts_with("~/") {
        let home = home.ok_or_else(|| bad(format!("path {p:?} uses ~ but $HOME is not set")))?;
        home.join(p.trim_start_matches('~').trim_start_matches('/'))
    } else {
        PathBuf::from(p)
    };
    if !path.is_absolute() {
        return Err(bad(format!("policy path {p:?} must be absolute or start with ~/")));
    }
    Ok(path)
}

fn parse_when(w: WhenFile) -> AaResult<When> {
    let many = |v: Option<OneOrMany>| v.map(OneOrMany::into_vec).unwrap_or_default();
    Ok(When {
        levels: many(w.level).iter().map(|s| parse_level(s)).collect::<AaResult<_>>()?,
        min_level: w.min_level.as_deref().map(parse_level).transpose()?,
        max_level: w.max_level.as_deref().map(parse_level).transpose()?,
        tiers: many(w.tier).iter().map(|s| parse_tier(s)).collect::<AaResult<_>>()?,
        min_tier: w.min_tier.as_deref().map(parse_tier).transpose()?,
        max_tier: w.max_tier.as_deref().map(parse_tier).transpose()?,
        targets: globs(w.targets)?,
        capabilities: globs(w.capabilities)?,
        target_registered: w.target_registered,
        session_tainted: w.session_tainted,
        any_effect: fields(w.any_effect)?,
        no_effect: fields(w.no_effect)?,
        assurance: many(w.assurance)
            .iter()
            .map(|s| parse_assurance(s))
            .collect::<AaResult<_>>()?,
    })
}

/// Parses `aa-policy/1`. `~` in `[paths]` is expanded against `$HOME`.
pub fn parse_policy(toml_text: &str) -> crate::AaResult<Policy> {
    let file: PolicyFile =
        toml::from_str(toml_text).map_err(|e| bad(format!("policy is not valid aa-policy/1: {e}")))?;
    if file.format != FORMAT {
        return Err(bad(format!("policy format {:?} is not {FORMAT:?}", file.format)));
    }
    if file.default != "deny" {
        return Err(bad(format!(
            "policy default {:?} is not allowed: the default is always \"deny\"",
            file.default
        )));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let exp = |v: &[String]| {
        v.iter()
            .map(|p| expand(p, home.as_deref()))
            .collect::<AaResult<Vec<_>>>()
    };
    let paths = PolicyPaths {
        write_roots: exp(&file.paths.write_roots)?,
        read_roots: exp(&file.paths.read_roots)?,
        deny: exp(&file.paths.deny)?,
    };
    let mut rules = Vec::with_capacity(file.rules.len());
    let mut seen = std::collections::BTreeSet::new();
    for r in file.rules {
        if r.id.trim().is_empty() {
            return Err(bad("a policy rule has an empty id"));
        }
        if !seen.insert(r.id.clone()) {
            return Err(bad(format!("duplicate policy rule id {:?}", r.id)));
        }
        rules.push(Rule {
            effect: parse_verdict(&r.effect)?,
            when: parse_when(r.when)?,
            approval_sources: r
                .approval_sources
                .iter()
                .map(|s| parse_source(s))
                .collect::<AaResult<_>>()?,
            require_tier: r.require_tier.as_deref().map(parse_assurance).transpose()?,
            reason: r.reason,
            id: r.id,
        });
    }
    Ok(Policy {
        format: file.format,
        digest: policy_digest(toml_text),
        paths,
        rules,
    })
}

/// `sha256_hex` of the policy file bytes.
pub fn policy_digest(toml_text: &str) -> String {
    crate::digest::sha256_hex(toml_text.as_bytes())
}

#[derive(Debug, Clone)]
pub struct ActionDescriptor<'a> {
    pub target_id: &'a str,
    pub capability_id: &'a str,
    pub level: pb::EffectLevel,
    pub tier: pb::AuthorityTier,
    pub effects: &'a pb::Effects,
    pub read_paths: &'a [std::path::PathBuf],
    pub write_paths: &'a [std::path::PathBuf],
    pub target_registered: bool,
    pub session_tainted: bool,
    pub assurance: pb::AssuranceTier,
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum M {
    Yes,
    No,
    Maybe,
}

fn all(ms: impl IntoIterator<Item = M>) -> M {
    let mut out = M::Yes;
    for m in ms {
        match m {
            M::No => return M::No,
            M::Maybe => out = M::Maybe,
            M::Yes => {}
        }
    }
    out
}

fn yes(b: bool) -> M {
    if b { M::Yes } else { M::No }
}

fn level_cond(d: pb::EffectLevel, f: impl Fn(pb::EffectLevel) -> bool) -> M {
    if d == pb::EffectLevel::Unknown {
        M::Maybe
    } else {
        yes(f(d))
    }
}

fn tier_cond(d: pb::AuthorityTier, f: impl Fn(pb::AuthorityTier) -> bool) -> M {
    if d == pb::AuthorityTier::Unknown {
        M::Maybe
    } else {
        yes(f(d))
    }
}

fn assurance_rank(a: pb::AssuranceTier) -> u8 {
    match a {
        pb::AssuranceTier::SamePrincipal => 0,
        pb::AssuranceTier::SeparatedAgent => 1,
        pb::AssuranceTier::IsolatedTarget => 2,
    }
}

impl When {
    fn eval(&self, d: &ActionDescriptor<'_>) -> M {
        let lr = effects::level_rank;
        let tr = effects::tier_rank;
        let mut conds = Vec::new();
        if !self.levels.is_empty() {
            conds.push(level_cond(d.level, |l| self.levels.contains(&l)));
        }
        if let Some(min) = self.min_level {
            conds.push(level_cond(d.level, |l| lr(l) >= lr(min)));
        }
        if let Some(max) = self.max_level {
            conds.push(level_cond(d.level, |l| lr(l) <= lr(max)));
        }
        if !self.tiers.is_empty() {
            conds.push(tier_cond(d.tier, |t| self.tiers.contains(&t)));
        }
        if let Some(min) = self.min_tier {
            conds.push(tier_cond(d.tier, |t| tr(t) >= tr(min)));
        }
        if let Some(max) = self.max_tier {
            conds.push(tier_cond(d.tier, |t| tr(t) <= tr(max)));
        }
        if !self.targets.is_empty() {
            conds.push(yes(self.targets.iter().any(|g| g.is_match(d.target_id))));
        }
        if !self.capabilities.is_empty() {
            conds.push(yes(self.capabilities.iter().any(|g| g.is_match(d.capability_id))));
        }
        if let Some(b) = self.target_registered {
            conds.push(yes(d.target_registered == b));
        }
        if let Some(b) = self.session_tainted {
            conds.push(yes(d.session_tainted == b));
        }
        if !self.any_effect.is_empty() {
            let vals: Vec<pb::Tri> = self
                .any_effect
                .iter()
                .map(|f| effects::get(d.effects, f).unwrap_or_default())
                .collect();
            conds.push(if vals.contains(&pb::Tri::True) {
                M::Yes
            } else if vals.contains(&pb::Tri::Unknown) {
                M::Maybe
            } else {
                M::No
            });
        }
        if !self.no_effect.is_empty() {
            let vals: Vec<pb::Tri> = self
                .no_effect
                .iter()
                .map(|f| effects::get(d.effects, f).unwrap_or_default())
                .collect();
            conds.push(if vals.contains(&pb::Tri::True) {
                M::No
            } else if vals.contains(&pb::Tri::Unknown) {
                M::Maybe
            } else {
                M::Yes
            });
        }
        if !self.assurance.is_empty() {
            conds.push(yes(self.assurance.contains(&d.assurance)));
        }
        all(conds)
    }
}

fn strictness(v: pb::Verdict) -> u8 {
    match v {
        pb::Verdict::Allow => 1,
        pb::Verdict::Ask => 2,
        pb::Verdict::Deny => 3,
    }
}

/// Id recorded as `deciding_rule_id` when nothing matched.
pub const DEFAULT_DENY_ID: &str = "default:deny";
/// Prefix of `deciding_rule_id` for a hard exclusion.
pub const HARD_EXCLUSION_PREFIX: &str = "exclusions@1:";
/// `deciding_rule_id` for a path under `[paths].deny`.
pub const PATHS_DENY_ID: &str = "paths:deny";

/// Hard exclusions first; then ALL matching rules; strictest wins regardless of order; no match => DENY.
pub fn evaluate(policy: &Policy, d: &ActionDescriptor<'_>) -> pb::PolicyDecision {
    let mut dec = pb::PolicyDecision {
        verdict: pb::Verdict::Deny as i32,
        assurance: d.assurance as i32,
        policy_digest: policy.digest.clone(),
        ..Default::default()
    };
    let deny_with = |mut dec: pb::PolicyDecision, id: String, reason: pb::DenialReason, why: String| {
        dec.verdict = pb::Verdict::Deny as i32;
        dec.deciding_rule_id = id;
        dec.denial_reason = reason as i32;
        dec.reasons.push(why);
        dec
    };

    // 1. Hard exclusions (configuration cannot remove them).
    let target = pb::Target {
        id: d.target_id.to_string(),
        ..Default::default()
    };
    if let Some(cat) = exclusions::excluded(&target, d.capability_id, "") {
        return deny_with(
            dec,
            format!("{HARD_EXCLUSION_PREFIX}{cat}"),
            pb::DenialReason::HardExclusion,
            format!("{} is a hard exclusion (category {cat})", d.capability_id),
        );
    }

    // 2. [paths].deny always wins.
    let deny = &policy.paths.deny;
    for p in d.read_paths.iter().chain(d.write_paths) {
        if deny
            .iter()
            .any(|r| p.starts_with(r) || p.starts_with(paths::canonical_root(r)))
        {
            return deny_with(
                dec,
                PATHS_DENY_ID.into(),
                pb::DenialReason::OutsidePathScope,
                format!("path {} is under a denied root", p.display()),
            );
        }
    }

    // 3. Every matching rule contributes; the strictest wins (ties: smallest rule id).
    let mut best: Option<(u8, &str, pb::Verdict, pb::DenialReason)> = None;
    let mut matched = Vec::new();
    for r in &policy.rules {
        let m = r.when.eval(d);
        let hit = match r.effect {
            pb::Verdict::Allow => m == M::Yes,
            pb::Verdict::Ask | pb::Verdict::Deny => m != M::No,
        };
        if !hit {
            continue;
        }
        matched.push(r.id.clone());
        let (verdict, reason) = match (r.effect, r.require_tier) {
            (pb::Verdict::Deny, _) => (pb::Verdict::Deny, pb::DenialReason::DenyRule),
            (_, Some(req)) if assurance_rank(d.assurance) < assurance_rank(req) => {
                (pb::Verdict::Deny, pb::DenialReason::TierTooLow)
            }
            (v, _) => (v, pb::DenialReason::Unspecified),
        };
        let cand = (strictness(verdict), r.id.as_str(), verdict, reason);
        best = match best {
            Some(b) if b.0 > cand.0 || (b.0 == cand.0 && b.1 <= cand.1) => Some(b),
            _ => Some(cand),
        };
    }
    matched.sort();
    dec.matched_rule_ids = matched;
    match best {
        None => deny_with(
            dec,
            DEFAULT_DENY_ID.into(),
            pb::DenialReason::NoMatchingAllow,
            format!(
                "no policy rule allows {} (level {}, tier {})",
                d.capability_id,
                d.level.as_str_name(),
                d.tier.as_str_name()
            ),
        ),
        Some((_, id, verdict, reason)) => {
            dec.verdict = verdict as i32;
            dec.deciding_rule_id = id.to_string();
            dec.denial_reason = reason as i32;
            match verdict {
                pb::Verdict::Allow => {
                    dec.approval_source = pb::ApprovalSource::PolicyAllow as i32;
                    dec.reasons.push(format!("allowed by rule {id}"));
                }
                pb::Verdict::Ask => dec.reasons.push(format!("rule {id} requires a confirmation")),
                pb::Verdict::Deny if reason == pb::DenialReason::TierTooLow => {
                    dec.reasons.push(format!("rule {id} requires a higher assurance tier"));
                }
                pb::Verdict::Deny => dec.reasons.push(format!("denied by rule {id}")),
            }
            if let Some(why) = policy.rule(id).and_then(|r| r.reason.clone()) {
                dec.reasons.push(why);
            }
            dec
        }
    }
}

/// One remediation command per denial reason (a test asserts each command exists in aa-cli's command list).
///
/// Every string ends with exactly one slice-1 `aa` command (`describe` or `status`) after a colon.
pub fn remediation_for(reason: pb::DenialReason, d: &ActionDescriptor<'_>) -> String {
    let cap = d.capability_id;
    match reason {
        pb::DenialReason::Unspecified => format!("inspect the capability and its admission: aa describe {cap}"),
        pb::DenialReason::NoMatchingAllow => format!(
            "no policy rule allows this action; ask the operator to add an allow rule to the policy file, and inspect \
             the capability's level and tier: aa describe {cap}"
        ),
        pb::DenialReason::DenyRule => {
            format!("a deny rule in the operator policy matched; review the capability's effects: aa describe {cap}")
        }
        pb::DenialReason::OutsidePathScope => format!(
            "pass paths inside the policy read/write roots or the session workspace (deny always wins); see the \
             path parameters: aa describe {cap}"
        ),
        pb::DenialReason::GrantAbsent => {
            "no standing grant covers this action; an operator must approve it on operator.sock (grants are M3), \
             check the engine state: aa status"
                .into()
        }
        pb::DenialReason::GrantExpired => {
            "the standing grant expired; an operator must issue a new one on operator.sock, check the engine \
             state: aa status"
                .into()
        }
        pb::DenialReason::GrantRevokedOrConsumed => {
            "the standing grant was revoked or used up; an operator must issue a new one on operator.sock, check \
             the engine state: aa status"
                .into()
        }
        pb::DenialReason::EnforcementUnavailable => {
            "this action must run confined and this host cannot provide confinement; check confinement \
             availability: aa status"
                .into()
        }
        pb::DenialReason::TierTooLow => {
            "the rule requires a higher assurance tier than this setup proves; check the current tier: aa status".into()
        }
        pb::DenialReason::HardExclusion => {
            format!("this capability is a hard exclusion and can never be served: aa describe {cap}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d<'a>(e: &'a pb::Effects, level: pb::EffectLevel, tier: pb::AuthorityTier) -> ActionDescriptor<'a> {
        ActionDescriptor {
            target_id: "git",
            capability_id: "git.status",
            level,
            tier,
            effects: e,
            read_paths: &[],
            write_paths: &[],
            target_registered: true,
            session_tainted: false,
            assurance: pb::AssuranceTier::SamePrincipal,
        }
    }

    #[test]
    fn default_policy_allows_observe_only() {
        let p = parse_policy(DEFAULT_POLICY_TOML).unwrap();
        let e = pb::Effects::default();
        let dec = evaluate(&p, &d(&e, pb::EffectLevel::Read, pb::AuthorityTier::Observe));
        assert_eq!(dec.verdict, pb::Verdict::Allow as i32);
        assert_eq!(dec.approval_source, pb::ApprovalSource::PolicyAllow as i32);
        let dec = evaluate(&p, &d(&e, pb::EffectLevel::Write, pb::AuthorityTier::Write));
        assert_eq!(dec.verdict, pb::Verdict::Deny as i32);
        assert_eq!(dec.denial_reason, pb::DenialReason::NoMatchingAllow as i32);
        let dec = evaluate(&p, &d(&e, pb::EffectLevel::Unknown, pb::AuthorityTier::Unknown));
        assert_eq!(dec.verdict, pb::Verdict::Deny as i32);
    }

    #[test]
    fn rejects_typos_and_bad_defaults() {
        assert!(parse_policy("format = \"aa-policy/1\"\ndefault = \"allow\"\n").is_err());
        assert!(parse_policy("format = \"aa-policy/2\"\n").is_err());
        let typo =
            "format = \"aa-policy/1\"\n[[rule]]\nid = \"a\"\neffect = \"allow\"\nwhen = { teir = \"observe\" }\n";
        assert!(parse_policy(typo).is_err());
        let field =
            "format = \"aa-policy/1\"\n[[rule]]\nid = \"a\"\neffect = \"ask\"\nwhen = { any_effect = [\"nope\"] }\n";
        assert!(parse_policy(field).is_err());
        let dup = "format = \"aa-policy/1\"\n[[rule]]\nid = \"a\"\neffect = \"ask\"\n[[rule]]\nid = \"a\"\neffect = \"deny\"\n";
        assert!(parse_policy(dup).is_err());
    }

    #[test]
    fn unknown_effects_escalate_to_ask() {
        let text = r#"format = "aa-policy/1"
[[rule]]
id = "allow-all-read"
effect = "allow"
when = { level = "read" }
[[rule]]
id = "privacy"
effect = "ask"
when = { any_effect = ["reads_screen"] }
approval_sources = ["operator_socket"]
"#;
        let p = parse_policy(text).unwrap();
        let mut e = pb::Effects::default();
        let dec = evaluate(&p, &d(&e, pb::EffectLevel::Read, pb::AuthorityTier::Observe));
        assert_eq!(dec.verdict, pb::Verdict::Ask as i32);
        assert_eq!(dec.deciding_rule_id, "privacy");
        assert_eq!(p.acceptable_sources(&dec), vec![pb::ApprovalSource::OperatorSocket]);
        e.reads_screen = pb::Tri::False as i32;
        let dec = evaluate(&p, &d(&e, pb::EffectLevel::Read, pb::AuthorityTier::Observe));
        assert_eq!(dec.verdict, pb::Verdict::Allow as i32);
        assert_eq!(dec.matched_rule_ids, vec!["allow-all-read".to_string()]);
    }

    #[test]
    fn deny_paths_and_require_tier() {
        let text = r#"format = "aa-policy/1"
[paths]
deny = ["/etc"]
[[rule]]
id = "a"
effect = "allow"
require_tier = "separated_agent"
"#;
        let p = parse_policy(text).unwrap();
        let e = pb::Effects::default();
        let dec = evaluate(&p, &d(&e, pb::EffectLevel::Read, pb::AuthorityTier::Observe));
        assert_eq!(dec.denial_reason, pb::DenialReason::TierTooLow as i32);
        let reads = [PathBuf::from("/etc/passwd")];
        let mut desc = d(&e, pb::EffectLevel::Read, pb::AuthorityTier::Observe);
        desc.read_paths = &reads;
        let dec = evaluate(&p, &desc);
        assert_eq!(dec.denial_reason, pb::DenialReason::OutsidePathScope as i32);
    }
}
