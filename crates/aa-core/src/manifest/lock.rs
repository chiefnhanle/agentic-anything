//! Canonical lock file I/O (`capabilities.lock.json`, schema `aa.lock/v1`).
//!
//! The on-disk form is the RFC 8785 canonical form of the lock's proto3-JSON (pbjson) plus one trailing `\n`, so
//! `lock_to_json(lock_from_json(b)) == b` for every canonical file, and git diffs of the lock are stable.

use crate::{AaError, digest, pb};

/// The `schema` value of a lock.
pub const LOCK_SCHEMA: &str = "aa.lock/v1";

fn bad(msg: impl Into<String>) -> AaError {
    AaError::new(
        pb::ErrorCode::ConfigurationInvalid,
        msg,
        "recompile the lock from the overlay and harvest artifacts: aa discover <target>",
    )
}

/// `sha256(JCS(lock with lock_digest = ""))`.
pub fn compute_digest(lock: &pb::CapabilityLock) -> crate::AaResult<String> {
    digest::self_digest(lock, |l| l.lock_digest.clear())
}

/// Canonical JCS bytes + trailing `\n`.
pub fn lock_to_json(lock: &pb::CapabilityLock) -> crate::AaResult<Vec<u8>> {
    let mut bytes = digest::jcs(lock)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Validates `schema == "aa.lock/v1"` and `lock_digest`.
pub fn lock_from_json(bytes: &[u8]) -> crate::AaResult<pb::CapabilityLock> {
    let lock: pb::CapabilityLock =
        serde_json::from_slice(bytes).map_err(|e| bad(format!("capabilities.lock.json is not a valid lock: {e}")))?;
    if lock.schema != LOCK_SCHEMA {
        return Err(bad(format!("lock schema {:?} is not {LOCK_SCHEMA:?}", lock.schema)));
    }
    let expected = compute_digest(&lock)?;
    if lock.lock_digest != expected {
        return Err(bad(format!(
            "lock_digest mismatch: file says {:?}, content hashes to {expected:?} (edited by hand?)",
            lock.lock_digest
        )));
    }
    Ok(lock)
}

pub fn finalize_digest(lock: &mut pb::CapabilityLock) -> crate::AaResult<()> {
    lock.lock_digest = compute_digest(lock)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper() {
        let mut lock = pb::CapabilityLock {
            schema: LOCK_SCHEMA.into(),
            capabilities: vec![pb::Capability {
                id: "t.x".into(),
                ..Default::default()
            }],
            rule_versions: [("compile".to_string(), "1".to_string())].into_iter().collect(),
            ..Default::default()
        };
        finalize_digest(&mut lock).unwrap();
        let bytes = lock_to_json(&lock).unwrap();
        assert!(bytes.ends_with(b"}\n"));
        let back = lock_from_json(&bytes).unwrap();
        assert_eq!(back, lock);
        assert_eq!(lock_to_json(&back).unwrap(), bytes);
        let tampered = String::from_utf8(bytes).unwrap().replace("t.x", "t.y");
        assert!(lock_from_json(tampered.as_bytes()).is_err());
        let mut wrong = lock.clone();
        wrong.schema = "aa.lock/v0".into();
        finalize_digest(&mut wrong).unwrap();
        assert!(lock_from_json(&lock_to_json(&wrong).unwrap()).is_err());
    }
}
