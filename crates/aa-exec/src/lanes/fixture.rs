//! Fixture building: seed state inside a lane's private root (`root.cwd`).
//!
//! Steps run in order:
//! - `file`: writes bytes to `<root.cwd>/<path>` (parents created; no absolute paths or `..`);
//! - `argv`: runs a pinned helper. `argv[0]` must name a pinned `FileIdentity`, by role (`"executable"`) or by
//!   the file name of its realpath (`"git"`); it runs in `<root.cwd>/<cwd>` with the root's HOME/TMPDIR, under
//!   the lane's confinement when available;
//! - `capability`: runs another capability of the lock (servable or not) through `binding.plan()` in the same
//!   root, so its HOME and state handle effects persist into the example run;
//! - `include_fixture`: builds another fixture first (cycles are rejected).
//!
//! `ready_capability_id` is deliberately NOT run: a "ready" read could itself mutate the fixture (plain
//! `git status` refreshes `.git/index`) and hide the very side effect L3 must observe.

use super::{LaneCtx, confinement, invoke_ctx, parse_input, resolve_root_refs, safe_rel};
use crate::root::PrivateRoot;
use crate::supervisor::DEFAULT_PATH;
use aa_core::{AaError, AaResult, api::RunSpec, pb};
use std::path::Path;

const STEP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
const MAX_INCLUDE_DEPTH: usize = 16;

fn fixture_error(id: &str, msg: impl std::fmt::Display) -> AaError {
    AaError::new(
        pb::ErrorCode::ConfigurationInvalid,
        format!("fixture {id:?}: {msg}"),
        "fix the fixture in the overlay, then recompile: aa discover <target>",
    )
}

/// Flattens `include_fixture` steps depth-first; rejects unknown ids and cycles.
fn flatten<'l>(
    lock: &'l pb::CapabilityLock,
    id: &str,
    stack: &mut Vec<String>,
    out: &mut Vec<(&'l str, &'l pb::fixture_step::Step)>,
) -> AaResult<()> {
    if stack.iter().any(|s| s == id) {
        stack.push(id.to_string());
        return Err(fixture_error(id, format!("include cycle {}", stack.join(" -> "))));
    }
    if stack.len() >= MAX_INCLUDE_DEPTH {
        return Err(fixture_error(id, "includes nest too deeply"));
    }
    let fixture = lock
        .target
        .as_ref()
        .and_then(|t| t.fixtures.iter().find(|f| f.id == id))
        .ok_or_else(|| fixture_error(id, "no such fixture in the lock"))?;
    stack.push(id.to_string());
    for step in &fixture.steps {
        match &step.step {
            Some(pb::fixture_step::Step::IncludeFixture(inner)) => flatten(lock, inner, stack, out)?,
            Some(s) => out.push((fixture.id.as_str(), s)),
            None => return Err(fixture_error(id, "empty step")),
        }
    }
    stack.pop();
    Ok(())
}

/// Resolves `argv[0]` of a fixture step to a pinned realpath.
fn pinned(ctx: &LaneCtx<'_>, name: &str) -> Option<String> {
    let files = &super::identity(ctx).files;
    files
        .iter()
        .find(|f| f.role == name)
        .or_else(|| {
            files
                .iter()
                .find(|f| Path::new(&f.realpath).file_name().is_some_and(|n| n == name))
        })
        .map(|f| f.realpath.clone())
        .filter(|p| Path::new(p).is_absolute())
}

pub async fn build_fixture(ctx: &LaneCtx<'_>, fixture_id: &str, root: &PrivateRoot) -> aa_core::AaResult<()> {
    let mut steps = Vec::new();
    flatten(ctx.lock, fixture_id, &mut Vec::new(), &mut steps)?;
    for (owner, step) in steps {
        match step {
            pb::fixture_step::Step::File(f) => {
                let rel = safe_rel(&f.path)?;
                if rel.as_os_str().is_empty() {
                    return Err(fixture_error(owner, "file step without a path"));
                }
                let path = root.cwd.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, &f.content)?;
            }
            pb::fixture_step::Step::Argv(a) => {
                let Some(name) = a.argv.first() else {
                    return Err(fixture_error(owner, "argv step without argv"));
                };
                let exe = pinned(ctx, name)
                    .ok_or_else(|| fixture_error(owner, format!("argv[0] {name:?} is not a pinned identity file")))?;
                let cwd = root.cwd.join(safe_rel(&a.cwd)?);
                let dir = Path::new(&exe).parent().map(|p| p.to_string_lossy().into_owned());
                let path = dir.map_or_else(|| DEFAULT_PATH.to_string(), |d| format!("{d}:{DEFAULT_PATH}"));
                let mut argv = vec![exe];
                argv.extend(a.argv[1..].iter().cloned());
                let spec = RunSpec {
                    argv,
                    cwd,
                    env: root.base_env(&path),
                    stdin: None,
                    deadline: STEP_DEADLINE,
                    max_stdout: 1024 * 1024,
                    max_stderr_excerpt: 2048,
                    confinement: confinement(ctx, root),
                    run_id: aa_core::ids::new_action_id(),
                };
                let out = ctx.runner.run(spec).await?;
                if out.exit_code != Some(0) || out.timed_out {
                    return Err(fixture_error(
                        owner,
                        format!(
                            "step {:?} failed (exit {:?}, timed out {}): {}",
                            a.argv,
                            out.exit_code,
                            out.timed_out,
                            out.stderr_excerpt.trim()
                        ),
                    ));
                }
            }
            pb::fixture_step::Step::Capability(call) => {
                let cap = ctx
                    .lock
                    .capabilities
                    .iter()
                    .find(|c| c.id == call.capability_id)
                    .ok_or_else(|| fixture_error(owner, format!("unknown capability {:?}", call.capability_id)))?;
                let input = resolve_root_refs(&parse_input(&call.input_json)?, root)?;
                let ictx = invoke_ctx(ctx, cap, root);
                let spec = ctx.binding.plan(cap, &input, &ictx)?;
                let out = ctx.runner.run(spec).await?;
                if out.exit_code != Some(0) || out.timed_out {
                    return Err(fixture_error(
                        owner,
                        format!(
                            "capability step {} failed (exit {:?}, timed out {}): {}",
                            cap.id,
                            out.exit_code,
                            out.timed_out,
                            out.stderr_excerpt.trim()
                        ),
                    ));
                }
            }
            // Flattened by `flatten` above.
            pb::fixture_step::Step::IncludeFixture(_) => {}
        }
    }
    Ok(())
}
