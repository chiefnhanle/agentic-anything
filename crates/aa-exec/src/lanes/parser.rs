//! L1 parser lane: the target's own argument parser must reject what the binding never sends.
//!
//! Runs, each in a fresh root built from the first example's fixture:
//! - a **control** run of the first example's input (or `{}`);
//! - the **unknown-flag probe**: the control argv plus `--aa-probe-unknown` (before the positional `--`);
//! - one **wrong-typed probe** per typed option (integer/number/boolean/enum in the input schema): the control
//!   input without that key, plus `--opt=aa-probe-…` (or `--opt aa-probe-…` for `no_equals_form`).
//!
//! A probe is *rejected* when it exits with `usage_exit_code` (any non-zero code when the binding sets none)
//! and, if `usage_stderr_marker` is set, its stderr contains the marker. A probe that exits with another
//! non-zero code also counts as rejected, but only if the control run succeeded (exit 0), so the rejection is
//! attributable to the probe (git's revision parser, e.g. `git log --aa-probe-unknown`, exits 128, not 129).
//! A probe that exits 0 was **accepted** → `PARSER_MISMATCH`. Any file change during a probe →
//! `UNEXPLAINED_CHANGE`.
//!
//! `no_equals_form` detection: when the control run itself is rejected as a usage error, each option present in
//! the input is retried as `--opt value`; if that is accepted, the record carries a `PARSER_MISMATCH` finding
//! with hint `no_equals_form:<pointer>` and the evidence id `no_equals_form:<pointer>` (the binding must be
//! recompiled with `no_equals_form` before the capability can pass).

use super::{LaneCtx, finding, finish, parse_input, record, run_example};
use crate::cli_argv::{cli_binding, top_key};
use aa_core::{api::RunOutput, pb};
use serde_json::Value;

pub const UNKNOWN_FLAG_PROBE: &str = "--aa-probe-unknown";
const MAX_NO_EQUALS_TRIES: usize = 8;

/// A wrong value for a typed schema property, or `None` when any string is valid.
fn wrong_value(schema: &Value, key: &str) -> Option<&'static str> {
    let prop = schema.get("properties")?.get(key)?;
    let item = match prop.get("type") {
        Some(Value::String(t)) if t == "array" => prop.get("items").unwrap_or(&Value::Null),
        _ => prop,
    };
    if item.get("enum").is_some() {
        return Some("aa-probe-not-in-enum");
    }
    let types: Vec<&str> = match item.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if types.iter().any(|t| *t == "integer" || *t == "number") {
        Some("aa-probe-not-a-number")
    } else if types.contains(&"boolean") {
        Some("aa-probe-not-a-bool")
    } else {
        None
    }
}

fn usage_error(out: &RunOutput, b: &pb::CliArgvBinding) -> bool {
    let code_ok = match out.exit_code {
        Some(c) if b.usage_exit_code != 0 => c == b.usage_exit_code,
        Some(c) => c != 0,
        None => false,
    };
    code_ok && (b.usage_stderr_marker.is_empty() || out.stderr_excerpt.contains(&b.usage_stderr_marker))
}

enum Verdict {
    Rejected,
    Accepted,
    Unattributable,
}

fn verdict(out: &RunOutput, b: &pb::CliArgvBinding, control_ok: bool) -> Verdict {
    if out.timed_out {
        Verdict::Unattributable
    } else if usage_error(out, b) {
        Verdict::Rejected
    } else if out.exit_code == Some(0) {
        Verdict::Accepted
    } else if control_ok && out.exit_code.is_some() {
        Verdict::Rejected
    } else {
        Verdict::Unattributable
    }
}

/// L1.
pub async fn run_parser_lane(ctx: &LaneCtx<'_>, cap: &aa_core::pb::Capability) -> aa_core::pb::VerificationRecord {
    let mut rec = record(ctx, cap, pb::Lane::Parser);
    rec.non_claims = vec![
        "semantic correctness not established".to_string(),
        "only the unknown-flag and wrong-typed probes were checked".to_string(),
        super::NON_CLAIM_REMOTE.to_string(),
    ];
    let Ok(b) = cli_binding(cap) else {
        return finish(rec, pb::LaneStatus::Skipped, "L1 applies to cli_argv bindings only");
    };
    rec.isolation = ctx.runner.max_isolation() as i32;
    let (base, fixture_id) = match cap.examples.first() {
        Some(ex) => match parse_input(&ex.input_json) {
            Ok(v) => (v, ex.fixture_id.clone()),
            Err(e) => return finish(rec, pb::LaneStatus::Error, format!("example 0: {}", e.message)),
        },
        None => (Value::Object(Default::default()), String::new()),
    };
    let schema: Value = serde_json::from_str(&cap.input_schema_json).unwrap_or(Value::Null);

    let control = match run_example(ctx, cap, &base, &fixture_id, None).await {
        Ok(r) => r,
        Err(e) => return finish(rec, pb::LaneStatus::Error, format!("control run: {}", e.message)),
    };
    let control_ok = control.out.exit_code == Some(0) && !control.out.timed_out;
    let mut isolation = control.out.isolation;

    // (probe label, input, extra args)
    let mut probes: Vec<(String, Value, Vec<String>)> = vec![(
        UNKNOWN_FLAG_PROBE.to_string(),
        base.clone(),
        vec![UNKNOWN_FLAG_PROBE.to_string()],
    )];
    for m in &b.args {
        let style = pb::ArgStyle::try_from(m.style).unwrap_or(pb::ArgStyle::Unspecified);
        if !matches!(style, pb::ArgStyle::Option | pb::ArgStyle::RepeatedOption) || m.flag.is_empty() {
            continue;
        }
        let Some(key) = top_key(&m.pointer) else { continue };
        let Some(bad) = wrong_value(&schema, &key) else {
            continue;
        };
        let mut input = base.clone();
        if let Some(o) = input.as_object_mut() {
            o.remove(&key);
        }
        let extra = if m.no_equals_form {
            vec![m.flag.clone(), bad.to_string()]
        } else {
            vec![format!("{}={bad}", m.flag)]
        };
        probes.push((format!("{} {}", m.pointer, extra.join(" ")), input, extra));
    }

    for (label, input, extra) in &probes {
        let run = match run_example(ctx, cap, input, &fixture_id, Some(extra)).await {
            Ok(r) => r,
            Err(e) => return finish(rec, pb::LaneStatus::Error, format!("probe {label}: {}", e.message)),
        };
        if run.out.isolation != pb::Isolation::Confined {
            isolation = run.out.isolation;
        }
        match verdict(&run.out, b, control_ok) {
            Verdict::Rejected => {}
            Verdict::Accepted => rec.findings.push(finding(
                pb::WithheldReason::ParserMismatch,
                format!("the parser accepted the probe `{label}` (exit 0); the binding cannot rely on it rejecting bad input"),
            )),
            Verdict::Unattributable => rec.findings.push(finding(
                pb::WithheldReason::ParserMismatch,
                format!(
                    "the probe `{label}` exited {:?} (usage code {}), and the control run exited {:?}: the rejection cannot be attributed to the probe",
                    run.out.exit_code, b.usage_exit_code, control.out.exit_code
                ),
            )),
        }
        if !run.observed.is_empty() {
            let paths: Vec<String> = run
                .observed
                .iter()
                .take(5)
                .map(|o| format!("{}:{}", o.root, o.path))
                .collect();
            rec.findings.push(finding(
                pb::WithheldReason::UnexplainedChange,
                format!("the probe `{label}` changed files: {}", paths.join(", ")),
            ));
            rec.observed_files.extend(run.observed);
        }
    }

    // no_equals_form detection on a usage-rejected control run.
    if usage_error(&control.out, b) {
        let mut tried = 0;
        for (i, m) in b.args.iter().enumerate() {
            let style = pb::ArgStyle::try_from(m.style).unwrap_or(pb::ArgStyle::Unspecified);
            if !matches!(style, pb::ArgStyle::Option | pb::ArgStyle::RepeatedOption)
                || m.no_equals_form
                || base.pointer(&m.pointer).is_none_or(Value::is_null)
                || tried >= MAX_NO_EQUALS_TRIES
            {
                continue;
            }
            tried += 1;
            let mut alt = cap.clone();
            if let Some(pb::binding::Kind::Cli(c)) = alt.binding.as_mut().and_then(|x| x.kind.as_mut()) {
                c.args[i].no_equals_form = true;
            }
            if let Ok(r) = run_example(ctx, &alt, &base, &fixture_id, None).await
                && r.out.exit_code == Some(0)
            {
                let mut f = finding(
                    pb::WithheldReason::ParserMismatch,
                    format!(
                        "no_equals_form:{} (\"{}=value\" is rejected, \"{} value\" is accepted)",
                        m.pointer, m.flag, m.flag
                    ),
                );
                f.evidence_ids.push(format!("no_equals_form:{}", m.pointer));
                rec.evidence_ids.push(format!("no_equals_form:{}", m.pointer));
                rec.findings.push(f);
            }
        }
    }

    rec.isolation = isolation as i32;
    if isolation != pb::Isolation::Confined {
        rec.non_claims.push(super::NON_CLAIM_PROCESS.to_string());
    }
    let status = if rec.findings.is_empty() {
        pb::LaneStatus::Passed
    } else {
        pb::LaneStatus::Failed
    };
    finish(rec, status, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_values_by_schema() {
        let s = serde_json::json!({"properties": {
            "n": {"type": "integer"}, "f": {"type": ["number", "null"]}, "b": {"type": "boolean"},
            "e": {"enum": ["a", "b"]}, "s": {"type": "string"}, "arr": {"type": "array", "items": {"type": "integer"}}
        }});
        assert_eq!(wrong_value(&s, "n"), Some("aa-probe-not-a-number"));
        assert_eq!(wrong_value(&s, "f"), Some("aa-probe-not-a-number"));
        assert_eq!(wrong_value(&s, "b"), Some("aa-probe-not-a-bool"));
        assert_eq!(wrong_value(&s, "e"), Some("aa-probe-not-in-enum"));
        assert_eq!(wrong_value(&s, "arr"), Some("aa-probe-not-a-number"));
        assert_eq!(wrong_value(&s, "s"), None);
        assert_eq!(wrong_value(&s, "missing"), None);
    }

    fn out(code: Option<i32>, stderr: &str) -> RunOutput {
        let mut o = aa_core::api::testing::ok_output("");
        o.exit_code = code;
        o.stderr_excerpt = stderr.to_string();
        o
    }

    #[test]
    fn verdicts() {
        let git = pb::CliArgvBinding {
            usage_exit_code: 129,
            ..Default::default()
        };
        assert!(matches!(verdict(&out(Some(129), ""), &git, false), Verdict::Rejected));
        assert!(matches!(verdict(&out(Some(0), ""), &git, true), Verdict::Accepted));
        assert!(matches!(verdict(&out(Some(128), ""), &git, true), Verdict::Rejected));
        assert!(matches!(
            verdict(&out(Some(128), ""), &git, false),
            Verdict::Unattributable
        ));
        let gh = pb::CliArgvBinding {
            usage_exit_code: 1,
            usage_stderr_marker: "unknown flag".into(),
            ..Default::default()
        };
        assert!(matches!(
            verdict(&out(Some(1), "unknown flag: --x"), &gh, false),
            Verdict::Rejected
        ));
        assert!(matches!(
            verdict(&out(Some(1), "other"), &gh, false),
            Verdict::Unattributable
        ));
        let any = pb::CliArgvBinding::default();
        assert!(matches!(verdict(&out(Some(2), ""), &any, false), Verdict::Rejected));
    }
}
