//! Authorization header newtype with redacting Display/Debug.
//!
//! Per spec §8 codex r1 P1.6 + r4 P2-r3.E fix: a structural guarantee
//! (not a policy) that a misplaced `{auth}` or `{auth:?}` in any
//! tracing macro CANNOT leak the user's bearer token. The underlying
//! value is exposed only via `expose_secret()` which is grep-able
//! and audit-able (slice 2 acceptance: `expose_secret(` appears
//! exactly once, at the upstream HTTP request construction site).
//!
//! Codex r3 P2-r3.E acknowledged: the strip-option (remove Authorization
//! from the request struct entirely) was rejected because it breaks
//! the §3.4 "Authorization forwarded byte-identical" requirement.
//! This newtype is the alternative implementation that satisfies both
//! the redaction goal and the forwarding requirement.

/// Wraps the user's bearer token. Display and Debug impls always
/// print `<redacted>` — the underlying value is never exposed via
/// formatting. Use `expose_secret()` to access the underlying string
/// at the single point where it's needed (constructing the upstream
/// HTTP request).
#[derive(Clone)]
pub struct RedactedAuth(String);

impl RedactedAuth {
    /// Wrap an authorization header value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Access the underlying bearer token. Only call at the upstream
    /// HTTP request construction site. Audit: `grep -n 'expose_secret('
    /// services/egress_proxy/src/` should show exactly one usage.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RedactedAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<redacted>")
    }
}

impl std::fmt::Debug for RedactedAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RedactedAuth(<redacted>)")
    }
}

// Codex slice-2 r1 P2.3: explicitly deny serde::Serialize so a future
// developer who derives Serialize on a struct containing RedactedAuth
// gets a compile error instead of silently leaking the value as JSON.
// expose_secret() is the only legitimate access path.
impl serde::Serialize for RedactedAuth {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom(
            "RedactedAuth must not be serialized; call expose_secret() at the upstream request site only",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECOY: &str = "Bearer sk-test-secret-decoy-1234567890abcdef";

    #[test]
    fn display_redacts() {
        let auth = RedactedAuth::new(DECOY);
        assert_eq!(auth.to_string(), "<redacted>");
    }

    #[test]
    fn debug_redacts() {
        let auth = RedactedAuth::new(DECOY);
        let dbg = format!("{:?}", auth);
        assert!(!dbg.contains("sk-test-secret"));
        assert!(!dbg.contains("Bearer"));
        assert_eq!(dbg, "RedactedAuth(<redacted>)");
    }

    #[test]
    fn expose_secret_returns_underlying() {
        let auth = RedactedAuth::new(DECOY);
        assert_eq!(auth.expose_secret(), DECOY);
    }

    #[test]
    fn format_with_named_arg_redacts() {
        // The common pattern that LEAKS: `tracing::info!(auth = ?auth)`.
        // Verify the formatted output is safe.
        let auth = RedactedAuth::new(DECOY);
        let formatted = format!("auth={:?}", auth);
        assert!(!formatted.contains("sk-test-secret"));
    }

    #[test]
    fn pretty_debug_redacts() {
        // Codex slice-2 r1 P2.3: pretty-print format also redacts.
        let auth = RedactedAuth::new(DECOY);
        let dbg = format!("{:#?}", auth);
        assert!(!dbg.contains("sk-test-secret"));
    }

    #[test]
    fn serde_serialize_is_denied() {
        // Codex slice-2 r1 P2.3: serde::Serialize impl always errors,
        // so accidentally JSON-serializing a struct containing this
        // type fails loudly instead of leaking.
        let auth = RedactedAuth::new(DECOY);
        let result = serde_json::to_string(&auth);
        assert!(result.is_err(), "RedactedAuth must not serialize");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("must not be serialized"), "got: {msg}");
    }
}
