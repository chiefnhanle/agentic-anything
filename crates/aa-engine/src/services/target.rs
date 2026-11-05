//! TargetService: ListTargets, GetTarget, CreateTarget, ImportCliAnything (AssumeEffects is slice 2).
//!
//! Registration is STATIC: it resolves and pins identity (realpath + sha256, never running the target),
//! writes `target.json` and the starter overlay (or keeps an existing one) and audits the authorization
//! statement verbatim (`TARGET_REGISTERED`).

use super::{RpcResult, page, unimplemented};
use crate::{Engine, role_layer::principal_of, to_status};
use aa_core::{AaError, AaResult, pb};
use aa_proto::v1::target_service_server::TargetService;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) struct TargetSvc(pub Arc<Engine>);

/// A name on PATH or a path; must be an executable regular file. Returns the realpath.
fn resolve_executable(name: &str) -> AaResult<PathBuf> {
    let is_exec = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    let found = if name.contains('/') {
        let p = PathBuf::from(name);
        is_exec(&p).then_some(p)
    } else {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .filter(|d| d.is_absolute())
                .map(|d| d.join(name))
                .find(|p| is_exec(p))
        })
    };
    let p = found.ok_or_else(|| {
        AaError::new(
            pb::ErrorCode::TargetUnavailable,
            format!("executable {name:?} not found (or not executable)"),
            "pass an absolute path: aa target add <id> --exec /path/to/tool",
        )
    })?;
    Ok(std::fs::canonicalize(p)?)
}

/// The interpreter named by a console script's `#!` line (`#!/venv/bin/python` or `#!/usr/bin/env python3`).
fn shebang_interpreter(script: &Path) -> AaResult<PathBuf> {
    let text = std::fs::read(script)?;
    let first = text.split(|b| *b == b'\n').next().unwrap_or_default();
    let line = String::from_utf8_lossy(first);
    let rest = line
        .strip_prefix("#!")
        .ok_or_else(|| AaError::invalid(format!("{} has no #! line", script.display())))?;
    let mut parts = rest.split_whitespace();
    let interp = parts.next().unwrap_or_default();
    if interp.ends_with("/env") {
        let name = parts.find(|a| !a.starts_with('-')).unwrap_or_default();
        return resolve_executable(name);
    }
    if !Path::new(interp).is_absolute() {
        return Err(AaError::invalid(format!(
            "{}: interpreter {interp:?} is not absolute",
            script.display()
        )));
    }
    // Keep the venv path itself (a venv's python is a symlink whose location selects the venv).
    Ok(PathBuf::from(interp))
}

impl Engine {
    /// Audited RPCs record every call: a refused one gets an AdminEvent naming the RPC and the error code.
    fn audit_refusal(&self, p: &pb::Principal, rpc: &str, subject: &str, e: &AaError) -> tonic::Status {
        let details = serde_json::json!({
            "rpc": format!("/agentic_anything.v1.TargetService/{rpc}"), "subject": subject,
            "refused": e.code.as_str_name(), "message": e.message,
        });
        if let Err(err) = self.admin(p, pb::AdminEventKind::Unspecified, subject, details) {
            tracing::error!(error = %err, "could not audit a refused {rpc}");
        }
        to_status(e)
    }

    fn register(&self, principal: &pb::Principal, req: pb::CreateTargetRequest) -> AaResult<pb::CreateTargetResponse> {
        aa_core::ids::validate_target_id(&req.target_id)?;
        let authorization = req.authorization_statement.trim();
        if authorization.is_empty() {
            return Err(AaError::invalid(
                "authorization_statement is required (who authorized automating this target)",
            ));
        }
        use pb::target_locator::Locator;
        let (kind, files, locator_json) = match req.locator.and_then(|l| l.locator) {
            Some(Locator::Executable(name)) => {
                let exe = resolve_executable(&name)?;
                let files = vec![self.sem.file_identity("executable", &exe)?];
                (pb::TargetKind::CliTool, files, serde_json::json!({"executable": name}))
            }
            Some(Locator::PythonConsoleScript(script)) => {
                let script_path = PathBuf::from(&script);
                if !script_path.is_absolute() || !script_path.is_file() {
                    return Err(AaError::invalid(format!(
                        "python_console_script {script:?} must be an absolute path to a file"
                    )));
                }
                let interp = shebang_interpreter(&script_path)?;
                let mut files = vec![
                    self.sem.file_identity("executable", &script_path)?,
                    self.sem.file_identity_as_invoked("interpreter", &interp)?,
                ];
                // A framework Python re-execs its Python.app binary; pin it so confinement allows that exec.
                if let Some(app) = aa_exec::identity::framework_python_app(&interp) {
                    files.push(self.sem.file_identity(aa_exec::identity::INTERPRETER_APP_ROLE, &app)?);
                }
                (
                    pb::TargetKind::CliTool,
                    files,
                    serde_json::json!({"python_console_script": script}),
                )
            }
            Some(_) => {
                return Err(AaError::new(
                    pb::ErrorCode::InvalidRequest,
                    "only executable and python_console_script locators are supported in this release",
                    "aa target add <id> --exec <path> | --python-script <path>",
                ));
            }
            None => return Err(AaError::invalid("locator is required")),
        };
        let mut identity = pb::TargetIdentity {
            files,
            ..Default::default()
        };
        identity.identity_digest = self.sem.identity_digest(&identity)?;
        let mut target = pb::Target {
            id: req.target_id.clone(),
            display_name: req.target_id.clone(),
            kind: kind as i32,
            identity: Some(identity),
            status: pb::TargetStatus::Scaffolded as i32,
            authorization_statement: authorization.to_string(),
            ..Default::default()
        };
        let harvesters = self.harvester_names(&target, &req.harvesters)?;
        let snapshot = target.clone();
        let overlay_sha =
            self.keep_or_write_overlay(&mut target, || self.sem.starter_overlay(&snapshot, &harvesters))?;
        self.store.write_target(&target)?;
        let details = serde_json::json!({
            "authorization_statement": authorization,
            "identity_digest": target.identity.as_ref().map(|i| i.identity_digest.clone()),
            "locator": locator_json, "harvesters": harvesters, "overlay_sha256": overlay_sha,
        });
        self.admin(principal, pb::AdminEventKind::TargetRegistered, &target.id, details)?;
        Ok(pb::CreateTargetResponse {
            overlay_path: self.store.overlay_path(&target.id).display().to_string(),
            target: Some(target),
        })
    }

    /// Keeps an existing overlay (taking display name / MCP prefix from it) or writes `starter()`.
    fn keep_or_write_overlay(&self, target: &mut pb::Target, starter: impl FnOnce() -> String) -> AaResult<String> {
        match self.store.read_overlay(&target.id)? {
            Some(existing) => {
                if let Ok(o) = self.sem.parse_overlay(&existing) {
                    if let Some(d) = o.display_name {
                        target.display_name = d;
                    }
                    if let Some(p) = o.mcp_prefix {
                        target.mcp_prefix = p;
                    }
                }
                Ok(aa_core::digest::sha256_hex(existing.as_bytes()))
            }
            None => {
                let yaml = starter();
                self.store.write_overlay(&target.id, &yaml)
            }
        }
    }

    async fn import(
        self: &Arc<Self>,
        principal: &pb::Principal,
        req: pb::ImportCliAnythingRequest,
    ) -> AaResult<pb::ImportCliAnythingResponse> {
        let repo = PathBuf::from(&req.repo_path);
        if !repo.is_absolute() {
            return Err(AaError::invalid("repo_path must be absolute"));
        }
        let available = self.sem.registry_names(&repo)?;
        if req.harnesses.is_empty() {
            return Ok(pb::ImportCliAnythingResponse {
                harnesses: vec![],
                available,
            });
        }
        let authorization = req.authorization_statement.trim().to_string();
        if authorization.is_empty() {
            return Err(AaError::invalid("authorization_statement is required"));
        }
        let python = PathBuf::from(&req.python);
        if !python.is_absolute() {
            return Err(AaError::invalid(
                "python must be an absolute path to an operator-installed interpreter",
            ));
        }
        let pp = self.pipeline_principal(principal);
        let gate = self.gate();
        let mut out = Vec::new();
        for name in &req.harnesses {
            let target_id = format!("cli-anything.{name}");
            let mut item = pb::ImportedHarness {
                name: name.clone(),
                target_id: target_id.clone(),
                ..Default::default()
            };
            if !available.contains(name) {
                item.error = Some(
                    crate::ports::not_found(
                        format!("{name:?} is not in the registry"),
                        "aa import cli-anything <repo> (lists harnesses)",
                    )
                    .to_proto(),
                );
                out.push(item);
                continue;
            }
            let placeholder = pb::Target {
                id: target_id.clone(),
                authorization_statement: authorization.clone(),
                ..Default::default()
            };
            let sem = Arc::clone(&self.sem);
            let fut = async { sem.import_harness(&repo, name, &python, &authorization) };
            let result = gate
                .pipeline(
                    &pp,
                    "aa.pipeline.import.cli-anything",
                    &placeholder,
                    pb::DiscoveryAuthority::Static,
                    fut,
                )
                .await
                .and_then(|(h, action_id)| self.store_import(principal, h, &authorization, &action_id));
            match result {
                Ok((tree_sha, install_sha)) => {
                    item.source_tree_sha = tree_sha;
                    item.install_cmd_artifact_sha256 = install_sha;
                }
                Err(e) => item.error = Some(e.to_proto()),
            }
            out.push(item);
        }
        Ok(pb::ImportCliAnythingResponse {
            harnesses: out,
            available,
        })
    }

    fn store_import(
        &self,
        principal: &pb::Principal,
        h: crate::ports::Imported,
        authorization: &str,
        action_id: &str,
    ) -> AaResult<(String, String)> {
        let mut target = h.target;
        aa_core::ids::validate_target_id(&target.id)?;
        if target.authorization_statement.trim().is_empty() {
            target.authorization_statement = authorization.to_string();
        }
        if target.status == pb::TargetStatus::Unspecified as i32 {
            target.status = pb::TargetStatus::Scaffolded as i32;
        }
        let yaml = h.overlay_yaml.clone();
        let overlay_sha = self.keep_or_write_overlay(&mut target, || yaml)?;
        let sources = self.store_raws(&target.id, action_id, &h.raw)?;
        let install_sha = h
            .raw
            .iter()
            .zip(&sources)
            .find(|(r, _)| r.name.contains("install_cmd"))
            .map(|(_, s)| s.sha256.clone())
            .unwrap_or_default();
        self.store.write_target(&target)?;
        let tree = target
            .identity
            .as_ref()
            .map(|i| i.source_tree_sha.clone())
            .unwrap_or_default();
        let details = serde_json::json!({
            "authorization_statement": target.authorization_statement,
            "identity_digest": target.identity.as_ref().map(|i| i.identity_digest.clone()),
            "source_tree_sha": tree, "install_cmd_artifact_sha256": install_sha,
            "install_cmd_flags": h.install_cmd_flags, "install_cmd_executed": false, "overlay_sha256": overlay_sha,
        });
        self.admin(principal, pb::AdminEventKind::TargetRegistered, &target.id, details)?;
        Ok((tree, install_sha))
    }
}

#[tonic::async_trait]
impl TargetService for TargetSvc {
    async fn list_targets(
        &self,
        request: tonic::Request<pb::ListTargetsRequest>,
    ) -> RpcResult<pb::ListTargetsResponse> {
        principal_of(&request)?;
        let e = &self.0;
        let mut targets = Vec::new();
        for id in e.store.list_targets().map_err(|e| to_status(&e))? {
            if let Some(t) = e.store.read_target(&id).map_err(|e| to_status(&e))? {
                targets.push(t);
            }
        }
        let (targets, page) = page(targets, request.get_ref().page.as_ref(), 200);
        Ok(tonic::Response::new(pb::ListTargetsResponse {
            targets,
            page: Some(page),
        }))
    }

    async fn get_target(&self, request: tonic::Request<pb::GetTargetRequest>) -> RpcResult<pb::GetTargetResponse> {
        principal_of(&request)?;
        let id = &request.get_ref().target_id;
        let e = &self.0;
        let target = e.store.read_target(id).map_err(|e| to_status(&e))?.ok_or_else(|| {
            to_status(&crate::ports::not_found(
                format!("unknown target {id:?}"),
                "aa target add <id> ...",
            ))
        })?;
        let lock_digest = e
            .store
            .read_lock(id)
            .map_err(|e| to_status(&e))?
            .map(|l| l.lock_digest)
            .unwrap_or_default();
        Ok(tonic::Response::new(pb::GetTargetResponse {
            target: Some(target),
            lock_digest,
        }))
    }

    async fn create_target(
        &self,
        request: tonic::Request<pb::CreateTargetRequest>,
    ) -> RpcResult<pb::CreateTargetResponse> {
        let p = principal_of(&request)?;
        let target_id = request.get_ref().target_id.clone();
        self.0
            .register(&p, request.into_inner())
            .map(tonic::Response::new)
            .map_err(|e| self.0.audit_refusal(&p, "CreateTarget", &target_id, &e))
    }

    async fn import_cli_anything(
        &self,
        request: tonic::Request<pb::ImportCliAnythingRequest>,
    ) -> RpcResult<pb::ImportCliAnythingResponse> {
        let p = principal_of(&request)?;
        let repo = request.get_ref().repo_path.clone();
        self.0
            .import(&p, request.into_inner())
            .await
            .map(tonic::Response::new)
            .map_err(|e| self.0.audit_refusal(&p, "ImportCliAnything", &repo, &e))
    }

    async fn assume_effects(
        &self,
        request: tonic::Request<pb::AssumeEffectsRequest>,
    ) -> RpcResult<pb::AssumeEffectsResponse> {
        principal_of(&request)?;
        Err(unimplemented("TargetService.AssumeEffects"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_resolution_and_shebangs() {
        let sh = resolve_executable("sh").unwrap();
        assert!(sh.is_absolute());
        assert!(resolve_executable("/definitely/not/here").is_err());
        assert!(resolve_executable("aa-no-such-tool-xyz").is_err());

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("notes-cli");
        std::fs::write(&script, "#!/venv/bin/python3\nimport sys\n").unwrap();
        assert_eq!(
            shebang_interpreter(&script).unwrap(),
            PathBuf::from("/venv/bin/python3")
        );
        std::fs::write(&script, "#!/usr/bin/env -S sh\n").unwrap();
        assert!(shebang_interpreter(&script).unwrap().ends_with("sh"));
        std::fs::write(&script, "print('no shebang')\n").unwrap();
        assert!(shebang_interpreter(&script).is_err());
    }
}
