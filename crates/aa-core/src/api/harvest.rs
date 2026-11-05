//! Harvester contract (frozen; implemented by aa-harvest).

use super::exec::ProcessRunner;
use crate::pb;

/// A verbatim raw self-description artifact, stored in the CAS.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RawHarvest {
    /// e.g. `"click-info@1"`.
    pub harvester: String,
    pub name: String,
    pub media_type: String,
    pub uri: String,
    /// A `pb::DiscoveryAuthority` number.
    pub authority: i32,
    pub bytes: Vec<u8>,
}

impl RawHarvest {
    /// CAS digest of the verbatim bytes.
    pub fn sha256(&self) -> String {
        crate::digest::sha256_hex(&self.bytes)
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    /// e.g. `["writer","add-paragraph"]`.
    pub native_path: Vec<String>,
    /// id, title/description (TargetText), input_schema_json, binding, provenance; NO effects/admission.
    pub capability: pb::Capability,
    /// Raise-only native hints (e.g. enum names), confidence <= DECLARED.
    pub native_signals: Vec<pb::EffectSignal>,
    /// REPL / bare group.
    pub interactive: bool,
}

pub struct HarvestCtx<'a> {
    pub target: &'a pb::Target,
    /// Overlay `harvest[].options`.
    pub options: &'a serde_json::Value,
    pub runner: &'a dyn ProcessRunner,
    pub scratch_parent: &'a std::path::Path,
    pub allow_network: bool,
}

#[async_trait::async_trait]
pub trait Harvester: Send + Sync {
    /// e.g. `"click-info"`.
    fn id(&self) -> &'static str;
    /// e.g. `1`.
    fn version(&self) -> u32;
    fn authority(&self) -> pb::DiscoveryAuthority;
    fn applies(&self, target: &pb::Target, options: &serde_json::Value) -> bool;
    /// Effectful.
    async fn harvest(&self, ctx: HarvestCtx<'_>) -> crate::AaResult<Vec<RawHarvest>>;
    /// PURE.
    fn candidates(&self, target: &pb::Target, raw: &[RawHarvest]) -> crate::AaResult<Vec<Candidate>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_sha256_is_of_bytes() {
        let r = RawHarvest {
            harvester: "click-info@1".into(),
            name: "root".into(),
            media_type: "application/json".into(),
            uri: "argv:x".into(),
            authority: pb::DiscoveryAuthority::Introspect as i32,
            bytes: b"{}".to_vec(),
        };
        assert_eq!(r.sha256(), crate::digest::sha256_hex(b"{}"));
    }
}
