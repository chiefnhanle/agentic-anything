//! Compiles `proto/agentic_anything/v1/*.proto` with prost (+ tonic services behind `grpc`), generates
//! pbjson serde impls, and emits the `RPC_POLICY` table from the `(agentic_anything.v1.rpc_policy)` method
//! option. A method without that option fails the build (build-plan §3.0.2).
use prost_reflect::{DescriptorPool, Value};
use std::{env, fmt::Write as _, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    let out = PathBuf::from(env::var("OUT_DIR")?);
    let desc = out.join("agentic_anything_v1.bin");
    let mut files: Vec<PathBuf> = fs::read_dir(root.join("agentic_anything/v1"))?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "proto"))
        .collect();
    files.sort();
    let grpc = env::var_os("CARGO_FEATURE_GRPC").is_some();
    tonic_prost_build::configure()
        .build_server(grpc)
        .build_client(grpc)
        .file_descriptor_set_path(&desc)
        .compile_well_known_types(true)
        .extern_path(".google.protobuf", "::pbjson_types")
        .compile_protos(&files, std::slice::from_ref(&root))?;
    let bytes = fs::read(&desc)?;
    pbjson_build::Builder::new()
        .register_descriptors(&bytes)?
        .build(&[".agentic_anything.v1"])?;

    // RPC_POLICY: every method MUST carry (agentic_anything.v1.rpc_policy) or the build fails.
    let pool = DescriptorPool::decode(bytes.as_slice())?;
    let ext = pool
        .get_extension_by_name("agentic_anything.v1.rpc_policy")
        .ok_or("rpc_policy extension not found in descriptor set")?;
    let mut t = String::from("/// Generated from the `(agentic_anything.v1.rpc_policy)` option of every RPC.\n");
    t.push_str("pub static RPC_POLICY: &[RpcPolicyRow] = &[\n");
    for svc in pool.services() {
        for m in svc.methods() {
            let o = m.options();
            assert!(o.has_extension(&ext), "method {} lacks rpc_policy", m.full_name());
            let v = o.get_extension(&ext);
            let msg = v.as_message().ok_or("rpc_policy is not a message")?;
            let roles: Vec<i32> = msg
                .get_field_by_name("roles")
                .ok_or("rpc_policy.roles missing")?
                .as_list()
                .ok_or("rpc_policy.roles is not a list")?
                .iter()
                .map(|r| match r {
                    Value::EnumNumber(n) => Ok(*n),
                    other => Err(format!("unexpected role value {other:?}")),
                })
                .collect::<Result<_, _>>()?;
            let halted = msg
                .get_field_by_name("allowed_when_halted")
                .and_then(|v| v.as_bool())
                .ok_or("rpc_policy.allowed_when_halted missing")?;
            let audited = msg
                .get_field_by_name("audited")
                .and_then(|v| v.as_bool())
                .ok_or("rpc_policy.audited missing")?;
            writeln!(
                t,
                "    RpcPolicyRow {{ path: \"/{}/{}\", roles: &{:?}, allowed_when_halted: {}, audited: {} }},",
                svc.full_name(),
                m.name(),
                roles,
                halted,
                audited
            )?;
        }
    }
    t.push_str("];\n");
    fs::write(out.join("rpc_policy.rs"), t)?;
    println!("cargo:rerun-if-changed={}", root.display());
    Ok(())
}
