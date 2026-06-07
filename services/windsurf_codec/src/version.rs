//! Wire-version registry for Windsurf Cascade.
//!
//! Per D18 design.md §3 decision 6 and §4 architecture: each captured
//! fixture pins a `cascade_wire_version`. The codec advertises which
//! versions it can decode. An inbound frame whose version is not in
//! this list MUST fail closed (no silent best-effort decode) — that's
//! the SOW-required posture mirrored from D17's `W6`/`W7` contract.

use std::fmt;
use std::sync::OnceLock;

use bytes::Bytes;
use sha2::{Digest, Sha256};

use crate::windsurf_proto::{CascadeRequest, CascadeResponseDelta};
use prost::Message;

/// Cascade wire version. Either an explicit `cascade_wire_version`
/// field from the request/response envelope, or a SHA-256 of the
/// first 64 bytes of the streaming preamble for frames that lack the
/// field.
///
/// Per D18 design.md §3 decision 6: pinning by hash is acceptable
/// when the explicit field is missing (early-Cascade captures), but
/// every hash-pin MUST be registered via
/// `SPENDGUARD_WINDSURF_PREAMBLE_HASHES` and listed in PROVENANCE.md.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WireVersion {
    /// Explicit `cascade_wire_version` string (e.g. `"cascade.v2.1"`).
    Explicit(String),
    /// SHA-256 of the first 64 bytes of the preamble for frames that
    /// lack the explicit field.
    PreambleHash([u8; 32]),
}

impl fmt::Display for WireVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Explicit(s) => write!(f, "explicit:{s}"),
            Self::PreambleHash(h) => write!(f, "preamble_sha256:{}", hex::encode(h)),
        }
    }
}

/// Known explicit wire versions this codec can decode. Reviewer
/// rejects any expansion not accompanied by a fixture in
/// `fixtures/synthetic/cascade_*.windsurf-rpc`.
///
/// SLICE 80 ships with `v2.0` and `v2.1` covered. Future Cascade
/// releases require a re-capture under SOW and an expansion of this
/// list.
pub const KNOWN_WIRE_VERSIONS: &[&str] = &["cascade.v2.0", "cascade.v2.1"];

/// True iff the given [`WireVersion`] is in the registry.
pub fn is_known(v: &WireVersion) -> bool {
    match v {
        WireVersion::Explicit(s) => KNOWN_WIRE_VERSIONS.contains(&s.as_str()),
        WireVersion::PreambleHash(h) => env_pinned_hashes().contains(h),
    }
}

/// Read the comma-separated `SPENDGUARD_WINDSURF_PREAMBLE_HASHES` env
/// var at startup; cached via `OnceLock`. Each entry is a 64-char
/// lowercase hex SHA-256 digest.
fn env_pinned_hashes() -> &'static [[u8; 32]] {
    static HASHES: OnceLock<Vec<[u8; 32]>> = OnceLock::new();
    HASHES.get_or_init(|| {
        std::env::var("SPENDGUARD_WINDSURF_PREAMBLE_HASHES")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .filter_map(|s| hex::decode(s.trim()).ok())
                    .filter_map(|v| <[u8; 32]>::try_from(v).ok())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Detect the wire version of a Cascade envelope body.
///
/// Cheap peek: try-decode the envelope as a [`CascadeRequest`] OR
/// [`CascadeResponseDelta`] and read the optional
/// `cascade_wire_version` field. Both messages reserve tag 99 for
/// the version stamp so either direction yields the same field.
/// Falls back to SHA-256 of the first 64 bytes of the body when the
/// field is absent — those preamble hashes must be registered via
/// the env var per D18 design.md §3 decision 6.
pub fn detect_version(body: &Bytes) -> WireVersion {
    if let Ok(req) = CascadeRequest::decode(body.clone()) {
        if let Some(v) = req.cascade_wire_version {
            return WireVersion::Explicit(v);
        }
    }
    if let Ok(delta) = CascadeResponseDelta::decode(body.clone()) {
        if let Some(v) = delta.cascade_wire_version {
            return WireVersion::Explicit(v);
        }
    }
    let mut h = Sha256::new();
    h.update(&body[..body.len().min(64)]);
    WireVersion::PreambleHash(h.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_versions_listed() {
        assert!(KNOWN_WIRE_VERSIONS.contains(&"cascade.v2.0"));
        assert!(KNOWN_WIRE_VERSIONS.contains(&"cascade.v2.1"));
    }

    #[test]
    fn explicit_version_is_known() {
        assert!(is_known(&WireVersion::Explicit("cascade.v2.0".into())));
        assert!(is_known(&WireVersion::Explicit("cascade.v2.1".into())));
    }

    #[test]
    fn unknown_explicit_version_rejected() {
        assert!(!is_known(&WireVersion::Explicit("cascade.v9.9".into())));
    }

    #[test]
    fn preamble_hash_not_in_env_is_rejected() {
        // No env var set in this test — any hash is rejected.
        let zero = [0u8; 32];
        assert!(!is_known(&WireVersion::PreambleHash(zero)));
    }

    #[test]
    fn display_format_includes_kind_prefix() {
        let e = WireVersion::Explicit("cascade.v2.0".into());
        assert_eq!(format!("{e}"), "explicit:cascade.v2.0");

        let h = WireVersion::PreambleHash([0xab; 32]);
        let s = format!("{h}");
        assert!(s.starts_with("preamble_sha256:"));
        assert_eq!(s.len(), "preamble_sha256:".len() + 64);
    }

    #[test]
    fn detect_version_from_explicit_field() {
        let req = CascadeRequest {
            messages: vec![],
            model_name: "gpt-4o".to_string(),
            max_tokens: None,
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".into()),
        };
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        let detected = detect_version(&Bytes::from(buf));
        assert_eq!(detected, WireVersion::Explicit("cascade.v2.0".into()));
    }

    #[test]
    fn detect_version_falls_back_to_preamble_hash() {
        // A payload that won't decode as CascadeRequest at all → falls
        // through to preamble hash.
        let raw = Bytes::from_static(&[0xffu8; 80]);
        let detected = detect_version(&raw);
        assert!(matches!(detected, WireVersion::PreambleHash(_)));
    }
}
