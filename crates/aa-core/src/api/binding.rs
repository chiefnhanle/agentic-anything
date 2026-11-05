//! Binding contract (frozen; `cli_argv` implemented by `aa-exec::CliArgvBinding`).

use super::exec::{ConfinementSpec, RunSpec};
use crate::pb;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind {
    CliArgv,
    Mcp,
    Http,
    Grpc,
    Adapter,
}

/// The kind of a proto binding; `None` when the oneof is unset.
pub fn binding_kind(b: &pb::Binding) -> Option<BindingKind> {
    use pb::binding::Kind;
    Some(match b.kind.as_ref()? {
        Kind::Cli(_) => BindingKind::CliArgv,
        Kind::Mcp(_) => BindingKind::Mcp,
        Kind::Http(_) => BindingKind::Http,
        Kind::Grpc(_) => BindingKind::Grpc,
        Kind::Adapter(_) => BindingKind::Adapter,
    })
}

#[derive(Debug, Clone)]
pub struct InvokeCtx {
    pub action_id: String,
    /// Verified just before dispatch.
    pub identity: pb::TargetIdentity,
    /// Engine-supplied (session workspace) or canonical scoped path.
    pub state_handle: Option<PathBuf>,
    /// Where the binding creates its private root.
    pub scratch_parent: PathBuf,
    pub deadline: std::time::Duration,
    pub dry_run: bool,
    pub confinement: Option<ConfinementSpec>,
}

#[derive(Debug, Clone)]
pub struct BindingOutput {
    pub outcome: pb::Outcome,
    /// JSON_STDOUT parsed; TEXT_STDOUT => `{"text","lines"}`.
    pub output_json: Option<serde_json::Value>,
    /// UPSTREAM_ERROR / UNREADABLE_OUTPUT / TIMEOUT / ...
    pub error: Option<crate::AaError>,
    pub process: pb::ProcessReport,
    pub observed_files: Vec<pb::ObservedFileEffect>,
}

#[async_trait::async_trait]
pub trait Binding: Send + Sync {
    fn kind(&self) -> BindingKind;
    /// Pure: the exact argv/request that WOULD be dispatched (used by tests, L1, and the audit record).
    fn plan(&self, cap: &pb::Capability, input: &serde_json::Value, ctx: &InvokeCtx) -> crate::AaResult<RunSpec>;
    async fn invoke(
        &self,
        cap: &pb::Capability,
        input: &serde_json::Value,
        ctx: &InvokeCtx,
    ) -> crate::AaResult<BindingOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(binding_kind(&pb::Binding::default()), None);
        let b = pb::Binding {
            kind: Some(pb::binding::Kind::Cli(pb::CliArgvBinding::default())),
        };
        assert_eq!(binding_kind(&b), Some(BindingKind::CliArgv));
        let b = pb::Binding {
            kind: Some(pb::binding::Kind::Adapter(pb::AdapterOperationBinding::default())),
        };
        assert_eq!(binding_kind(&b), Some(BindingKind::Adapter));
    }
}
