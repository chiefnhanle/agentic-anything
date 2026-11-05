//! Generated agentic-anything v1 protocol types (prost + pbjson serde), the encoded descriptor set and
//! the `RPC_POLICY` table. Tonic clients/servers are generated only with the `grpc` feature.

pub mod v1 {
    #![allow(clippy::all, clippy::pedantic)]
    #![allow(missing_docs, rustdoc::all)]
    include!(concat!(env!("OUT_DIR"), "/agentic_anything.v1.rs"));
    include!(concat!(env!("OUT_DIR"), "/agentic_anything.v1.serde.rs"));
}

/// One row per RPC: the roles allowed to call it, whether it is allowed while halted, and whether every
/// call is audited. `roles` holds `v1::Role` numbers.
#[derive(Debug, Clone, Copy)]
pub struct RpcPolicyRow {
    pub path: &'static str,
    pub roles: &'static [i32],
    pub allowed_when_halted: bool,
    pub audited: bool,
}

include!(concat!(env!("OUT_DIR"), "/rpc_policy.rs"));

/// Looks up the policy row for a gRPC path such as `/agentic_anything.v1.EngineService/Halt`.
pub fn rpc_policy(path: &str) -> Option<&'static RpcPolicyRow> {
    RPC_POLICY.iter().find(|r| r.path == path)
}

/// Encoded `FileDescriptorSet` of every `agentic_anything.v1` proto (and its imports).
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/agentic_anything_v1.bin"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const OPERATOR: i32 = v1::Role::Operator as i32;
    const AGENT: i32 = v1::Role::Agent as i32;
    const ENGINE_INTERNAL: i32 = v1::Role::EngineInternal as i32;

    #[test]
    fn table_has_one_row_per_rpc() {
        assert_eq!(RPC_POLICY.len(), 41);
        let paths: BTreeSet<_> = RPC_POLICY.iter().map(|r| r.path).collect();
        assert_eq!(paths.len(), RPC_POLICY.len(), "duplicate RPC paths");
        for r in RPC_POLICY {
            assert!(r.path.starts_with("/agentic_anything.v1."), "{}", r.path);
            assert!(!r.roles.is_empty(), "{} is unreachable", r.path);
            assert!(
                !r.roles.contains(&(v1::Role::None as i32)),
                "{} grants ROLE_NONE",
                r.path
            );
        }
    }

    #[test]
    fn operator_only_rpcs() {
        for path in [
            "/agentic_anything.v1.PolicyService/DecideConfirmation",
            "/agentic_anything.v1.PolicyService/IssueGrant",
            "/agentic_anything.v1.EngineService/Unhalt",
            "/agentic_anything.v1.TargetService/AssumeEffects",
        ] {
            let row = rpc_policy(path).unwrap_or_else(|| panic!("{path} missing"));
            assert_eq!(row.roles, &[OPERATOR], "{path}");
        }
    }

    #[test]
    fn adapter_service_is_engine_internal_only() {
        let rows: Vec<_> = RPC_POLICY
            .iter()
            .filter(|r| r.path.starts_with("/agentic_anything.v1.AdapterService/"))
            .collect();
        assert_eq!(rows.len(), 6);
        for r in rows {
            assert_eq!(r.roles, &[ENGINE_INTERNAL], "{}", r.path);
        }
    }

    #[test]
    fn halt_is_reachable_by_both_roles_while_halted() {
        let row = rpc_policy("/agentic_anything.v1.EngineService/Halt").expect("Halt row");
        assert!(row.roles.contains(&AGENT) && row.roles.contains(&OPERATOR));
        assert!(row.allowed_when_halted);
        assert!(rpc_policy("/agentic_anything.v1.EngineService/Nope").is_none());
    }

    #[test]
    fn descriptor_set_is_embedded() {
        assert!(FILE_DESCRIPTOR_SET.len() > 1000);
    }

    #[test]
    fn pbjson_uses_proto3_json_names() {
        let e = v1::Effects {
            mutates_target: v1::Tri::True as i32,
            ..Default::default()
        };
        let s = serde_json::to_string(&e).expect("serialize");
        assert_eq!(s, r#"{"mutatesTarget":"TRI_TRUE"}"#);
    }

    #[cfg(feature = "grpc")]
    #[test]
    fn grpc_services_are_generated() {
        fn _assert_types() {
            let _ = std::mem::size_of::<v1::engine_service_client::EngineServiceClient<tonic::transport::Channel>>();
        }
        assert_eq!(
            v1::engine_service_server::SERVICE_NAME,
            "agentic_anything.v1.EngineService"
        );
    }
}
