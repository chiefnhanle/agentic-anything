//! Canonical JSON (RFC 8785 / JCS) and the digests built on it.
//!
//! Proto messages serialize through their pbjson impls (proto3-JSON: camelCase names, enum names, 64-bit
//! integers as strings, default values omitted), so a digest of a message is a digest of its canonical
//! proto3-JSON form.

use crate::{AaError, AaResult, pb};
use hmac::{KeyInit, Mac};
use sha2::{Digest, Sha256};

/// RFC 8785 canonical JSON bytes of `v`.
pub fn jcs<T: serde::Serialize + ?Sized>(v: &T) -> AaResult<Vec<u8>> {
    serde_json_canonicalizer::to_vec(&v).map_err(|e| AaError::internal(format!("JCS serialization failed: {e}")))
}

/// Lowercase hex of SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// `sha256_hex(jcs(v))`.
pub fn digest<T: serde::Serialize + ?Sized>(v: &T) -> AaResult<String> {
    Ok(sha256_hex(&jcs(v)?))
}

/// Digest of a message with one string field blanked first (lock_digest, entry_sha256, ...).
pub fn self_digest<T: serde::Serialize + Clone>(v: &T, blank: impl FnOnce(&mut T)) -> AaResult<String> {
    let mut copy = v.clone();
    blank(&mut copy);
    digest(&copy)
}

/// `"hmac-sha256:<hex>"` of the canonical JSON of `input` under the engine-local key.
pub fn args_commitment(key: &[u8; 32], input: &serde_json::Value) -> AaResult<String> {
    let mut mac = <hmac::Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .map_err(|e| AaError::internal(format!("HMAC key rejected: {e}")))?;
    mac.update(&jcs(input)?);
    Ok(format!("hmac-sha256:{}", hex(&mac.finalize().into_bytes())))
}

/// `sha256(JCS(ActionDigestInput))`: binds a confirmation to exactly one action.
pub fn action_digest(input: &pb::ActionDigestInput) -> AaResult<String> {
    digest(input)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[usize::from(b >> 4)] as char);
        s.push(DIGITS[usize::from(b & 0x0f)] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// RFC 8785 Appendix B: IEEE-754 bit patterns and their canonical ES6 number serialization.
    /// (NaN and Infinity are not representable in serde_json and are rejected by JCS.)
    const APPENDIX_B: &str = include_str!("../tests/fixtures/rfc8785_appendix_b.txt");

    #[test]
    fn rfc8785_appendix_b_numbers() {
        let mut n = 0;
        for line in APPENDIX_B
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (bits, expected) = line.split_once(char::is_whitespace).expect("fixture line");
            let expected = expected.trim();
            let f = f64::from_bits(u64::from_str_radix(bits, 16).expect("hex bits"));
            // Fixture self-check: the expected text parses back to the same double (or -0 => 0).
            assert_eq!(
                expected.parse::<f64>().expect("expected parses"),
                if f == 0.0 { 0.0 } else { f },
                "{bits}"
            );
            let got = String::from_utf8(jcs(&Value::from(f)).expect("jcs")).expect("utf8");
            assert_eq!(got, expected, "bits {bits}");
            n += 1;
        }
        assert!(n >= 20, "fixture has {n} vectors");
    }

    #[test]
    fn rfc8785_key_ordering_vector() {
        // RFC 8785 §3.2.3: keys are sorted by UTF-16 code units.
        let input = r#"{
            "\u20ac": "Euro Sign",
            "\r": "Carriage Return",
            "\ufb33": "Hebrew Letter Dalet With Dagesh",
            "1": "One",
            "\ud83d\ude00": "Emoji: Grinning Face",
            "\u0080": "Control",
            "\u00f6": "Latin Small Letter O With Diaeresis"
        }"#;
        let v: Value = serde_json::from_str(input).expect("parse");
        let out = String::from_utf8(jcs(&v).expect("jcs")).expect("utf8");
        let expected = concat!(
            "{\"\\r\":\"Carriage Return\",\"1\":\"One\",\"\u{80}\":\"Control\",",
            "\"\u{f6}\":\"Latin Small Letter O With Diaeresis\",\"\u{20ac}\":\"Euro Sign\",",
            "\"\u{1f600}\":\"Emoji: Grinning Face\",\"\u{fb33}\":\"Hebrew Letter Dalet With Dagesh\"}"
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn rfc8785_main_example() {
        // RFC 8785 §3.2.2 sample input and its canonical output.
        let input = r#"{
            "numbers": [333333333.33333329, 1E30, 4.50, 2e-3, 0.000000000000000000000000001],
            "string": "\u20ac$\u000F\u000aA'\u0042\u0022\u005c\\\"\/",
            "literals": [null, true, false]
        }"#;
        let v: Value = serde_json::from_str(input).expect("parse");
        let out = String::from_utf8(jcs(&v).expect("jcs")).expect("utf8");
        let expected = concat!(
            r#"{"literals":[null,true,false],"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27],"#,
            "\"string\":\"\u{20ac}$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}"
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn digests() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(&json!({"b": 1, "a": [true]})).unwrap(),
            sha256_hex(br#"{"a":[true],"b":1}"#)
        );
        let lock = pb::CapabilityLock {
            schema: "aa.lock/v1".into(),
            lock_digest: "stale".into(),
            ..Default::default()
        };
        let d = self_digest(&lock, |l| l.lock_digest.clear()).unwrap();
        assert_eq!(d, sha256_hex(br#"{"schema":"aa.lock/v1"}"#));
    }

    #[test]
    fn hmac_commitment() {
        // Same key + equal canonical input => equal commitment; order-insensitive.
        let key = [7u8; 32];
        let a = args_commitment(&key, &json!({"x": 1, "y": "z"})).unwrap();
        let b = args_commitment(&key, &json!({"y": "z", "x": 1})).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("hmac-sha256:") && a.len() == "hmac-sha256:".len() + 64);
        assert_ne!(a, args_commitment(&[8u8; 32], &json!({"x": 1, "y": "z"})).unwrap());
        // Known answer: HMAC-SHA256(key = 32 x 0x00, "{}").
        let kat = args_commitment(&[0u8; 32], &json!({})).unwrap();
        assert_eq!(
            kat,
            "hmac-sha256:22f8eea909400af98adf3681a9f31923ef6b7fcba4abb553d92823a3e9d5c25e"
        );
    }

    #[test]
    fn action_digest_is_digest_of_proto3_json() {
        let input = pb::ActionDigestInput {
            capability_id: "git.status".into(),
            generation: 3,
            dry_run: true,
            ..Default::default()
        };
        let expected = sha256_hex(br#"{"capabilityId":"git.status","dryRun":true,"generation":"3"}"#);
        assert_eq!(action_digest(&input).unwrap(), expected);
    }
}
