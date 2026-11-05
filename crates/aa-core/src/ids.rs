//! Identifiers: target ids, capability ids, MCP tool names and ULID-based runtime ids (architecture §7.1).

use crate::{AaError, AaResult, digest::sha256_hex};
use std::collections::BTreeSet;

/// Maximum MCP tool-name length (`^[a-zA-Z0-9_-]{1,64}$`).
pub const MCP_NAME_MAX: usize = 64;
/// Length of the deterministic collision suffix (hex chars of sha256(capability id)).
pub const MCP_SUFFIX_HEX: usize = 6;

/// Validates `[a-z0-9][a-z0-9-]*(\.[a-z0-9-]+)*`.
pub fn validate_target_id(id: &str) -> AaResult<()> {
    let bad = || {
        AaError::invalid(format!(
            "invalid target id {id:?}: must match [a-z0-9][a-z0-9-]*(\\.[a-z0-9-]+)*"
        ))
    };
    let mut segments = id.split('.');
    let first = segments.next().ok_or_else(bad)?;
    let seg_ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    if !seg_ok(first) || !first.as_bytes()[0].is_ascii_alphanumeric() {
        return Err(bad());
    }
    if segments.all(seg_ok) { Ok(()) } else { Err(bad()) }
}

/// `<target>.<native path...>`: each path segment is lowercased and `_` becomes `-`; segments join with `.`.
pub fn capability_id(target_id: &str, native_path: &[&str]) -> String {
    let mut id = String::from(target_id);
    for seg in native_path {
        id.push('.');
        id.extend(
            seg.chars()
                .flat_map(char::to_lowercase)
                .map(|c| if c == '_' { '-' } else { c }),
        );
    }
    id
}

/// `<prefix>_<noun>_<verb>` from a capability id (the `<target>.` prefix is dropped), restricted to
/// `[a-zA-Z0-9_-]` and at most 64 chars. When the base name is taken or had to be truncated, a deterministic
/// `_<6 hex of sha256(capability_id)>` suffix is appended (with further deterministic rehashes in the
/// astronomically unlikely event that is taken too). The result is never in `taken`.
pub fn mcp_name(prefix: &str, capability_id: &str, target_id: &str, taken: &BTreeSet<String>) -> String {
    let rest = capability_id
        .strip_prefix(target_id)
        .and_then(|r| r.strip_prefix('.'))
        .unwrap_or(if capability_id == target_id { "" } else { capability_id });
    let prefix = if prefix.is_empty() { target_id } else { prefix };
    let mut raw = String::from(prefix);
    if !rest.is_empty() {
        raw.push('_');
        raw.push_str(rest);
    }
    let mut base: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if base.is_empty() {
        base.push('_');
    }
    if base.len() <= MCP_NAME_MAX && !taken.contains(&base) {
        return base;
    }
    base.truncate(MCP_NAME_MAX - 1 - MCP_SUFFIX_HEX);
    for n in 0u32.. {
        let seed = if n == 0 {
            capability_id.to_string()
        } else {
            format!("{capability_id}#{n}")
        };
        let candidate = format!("{base}_{}", &sha256_hex(seed.as_bytes())[..MCP_SUFFIX_HEX]);
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("u32 suffix space exhausted")
}

/// `"act_" + ULID`.
pub fn new_action_id() -> String {
    format!("act_{}", ulid::Ulid::generate())
}

/// `"ses_" + ULID`.
pub fn new_session_id() -> String {
    format!("ses_{}", ulid::Ulid::generate())
}

/// `"cnf_" + ULID`.
pub fn new_confirmation_id() -> String {
    format!("cnf_{}", ulid::Ulid::generate())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_ids() {
        for ok in ["git", "gh", "cli-anything.libreoffice", "a", "0x", "a-b.c-d.e", "a.-x"] {
            assert!(validate_target_id(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "-git", "Git", "git.", ".git", "a..b", "a_b", "a b", "a/b", "é", "git.",
        ] {
            assert!(validate_target_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn capability_ids() {
        assert_eq!(capability_id("git", &["status"]), "git.status");
        assert_eq!(
            capability_id("cli-anything.libreoffice", &["writer", "Add_Paragraph"]),
            "cli-anything.libreoffice.writer.add-paragraph"
        );
        assert_eq!(capability_id("git", &[]), "git");
    }

    #[test]
    fn mcp_names() {
        let mut taken = BTreeSet::new();
        let a = mcp_name("git", "git.branch.list", "git", &taken);
        assert_eq!(a, "git_branch_list");
        taken.insert(a.clone());
        let b = mcp_name("git", "git.branch.list", "git", &taken);
        assert_ne!(a, b);
        assert!(
            b.starts_with("git_branch_list_") && b.len() == "git_branch_list_".len() + 6,
            "{b}"
        );
        assert_eq!(b, mcp_name("git", "git.branch.list", "git", &taken), "deterministic");
        assert_eq!(
            mcp_name("", "cli-anything.lo.writer.add", "cli-anything.lo", &BTreeSet::new()),
            "cli-anything_lo_writer_add"
        );

        let long = format!("t.{}", "x".repeat(100));
        let n = mcp_name("t", &long, "t", &BTreeSet::new());
        assert_eq!(n.len(), MCP_NAME_MAX);
        assert!(n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn runtime_ids() {
        let a = new_action_id();
        assert!(a.starts_with("act_") && a.len() == 30, "{a}");
        assert_ne!(a, new_action_id());
        assert!(new_session_id().starts_with("ses_"));
        assert!(new_confirmation_id().starts_with("cnf_"));
    }
}
