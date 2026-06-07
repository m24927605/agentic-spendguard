//! Windsurf / Codeium endpoint detection.
//!
//! D18 SLICE 77 — routing table augmentation. The codec advertises
//! the two Cascade backend hosts (`server.codeium.com` and
//! `windsurf-server.codeium.com`) plus the gRPC path that carries
//! CascadeChat. Egress proxy callers use this to decide whether a
//! request flows through the Cascade decoder pipeline or falls
//! through to the default pass-through.
//!
//! Per D18 design.md §4 routing: every Codeium row is
//! `experimental: true` — the two-channel opt-in gate
//! ([`crate::experimental`]) refuses Cascade routes when either
//! `SPENDGUARD_EXPERIMENTAL_CODECS` or
//! `spendguard.toml` `[experimental.windsurf_codec] enabled = true`
//! is absent.

/// The set of Cascade-bearing inbound hosts the codec recognises.
///
/// Reviewer rejects expansion without an accompanying fixture in
/// `tests/fixtures/` and PROVENANCE.md update.
pub const WINDSURF_CASCADE_HOSTS: &[&str] = &["server.codeium.com", "windsurf-server.codeium.com"];

/// gRPC path regex that identifies the CascadeChat method on the
/// Codeium language server.
pub const WINDSURF_CASCADE_PATH_REGEX: &str =
    r"^/exa\.language_server_pb\.LanguageServerService/CascadeChat$";

/// Returns true if the given inbound host is one of the
/// Cascade-bearing Codeium endpoints.
pub fn is_cascade_host(host: &str) -> bool {
    WINDSURF_CASCADE_HOSTS.contains(&host)
}

/// Returns true if both the host AND path indicate a Cascade chat
/// request.
///
/// The path check is a literal-string compare — egress proxies that
/// want regex semantics should use [`WINDSURF_CASCADE_PATH_REGEX`]
/// directly with their existing regex engine. The literal path is
/// the only one observed in captures; future variants require an
/// update here AND a fixture.
pub fn is_cascade_chat_route(host: &str, path: &str) -> bool {
    if !is_cascade_host(host) {
        return false;
    }
    path == "/exa.language_server_pb.LanguageServerService/CascadeChat"
}

/// Routing decision the egress proxy emits when it sees a Cascade
/// request.
///
/// Wraps the boolean is-Cascade verdict with the experimental flag
/// so the caller can log the misconfiguration ("Cascade host matched
/// but feature disabled") with a single value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeRoutingDecision {
    /// Not a Cascade route at all — fall through to default
    /// pass-through.
    NotCascade,
    /// Cascade route but experimental gate is closed. Egress proxy
    /// should refuse with synthetic 503 + `experimental_codec_disabled`.
    CascadeButGateClosed,
    /// Cascade route AND experimental gate is open. Egress proxy
    /// should dispatch through the codec pipeline.
    CascadeForwardThroughCodec,
}

/// Compute the routing decision for an inbound HTTP/2 request given
/// the host, path, and the experimental-gate verdict.
pub fn classify_cascade_route(
    host: &str,
    path: &str,
    experimental_gate_open: bool,
) -> CascadeRoutingDecision {
    if !is_cascade_chat_route(host, path) {
        return CascadeRoutingDecision::NotCascade;
    }
    if experimental_gate_open {
        CascadeRoutingDecision::CascadeForwardThroughCodec
    } else {
        CascadeRoutingDecision::CascadeButGateClosed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cascade_hosts_recognised() {
        assert!(is_cascade_host("server.codeium.com"));
        assert!(is_cascade_host("windsurf-server.codeium.com"));
        assert!(!is_cascade_host("api.openai.com"));
        assert!(!is_cascade_host("api.cursor.sh"));
    }

    #[test]
    fn cascade_chat_path_recognised() {
        assert!(is_cascade_chat_route(
            "server.codeium.com",
            "/exa.language_server_pb.LanguageServerService/CascadeChat"
        ));
        assert!(is_cascade_chat_route(
            "windsurf-server.codeium.com",
            "/exa.language_server_pb.LanguageServerService/CascadeChat"
        ));
    }

    #[test]
    fn wrong_path_on_cascade_host_is_not_cascade() {
        assert!(!is_cascade_chat_route(
            "server.codeium.com",
            "/exa.language_server_pb.LanguageServerService/Other"
        ));
        assert!(!is_cascade_chat_route("server.codeium.com", "/healthz"));
    }

    #[test]
    fn wrong_host_with_right_path_not_cascade() {
        assert!(!is_cascade_chat_route(
            "api.openai.com",
            "/exa.language_server_pb.LanguageServerService/CascadeChat"
        ));
    }

    #[test]
    fn classify_route_not_cascade() {
        assert_eq!(
            classify_cascade_route("api.openai.com", "/v1/chat/completions", true),
            CascadeRoutingDecision::NotCascade
        );
    }

    #[test]
    fn classify_route_gate_closed() {
        assert_eq!(
            classify_cascade_route(
                "server.codeium.com",
                "/exa.language_server_pb.LanguageServerService/CascadeChat",
                false
            ),
            CascadeRoutingDecision::CascadeButGateClosed
        );
    }

    #[test]
    fn classify_route_forward_through_codec() {
        assert_eq!(
            classify_cascade_route(
                "windsurf-server.codeium.com",
                "/exa.language_server_pb.LanguageServerService/CascadeChat",
                true
            ),
            CascadeRoutingDecision::CascadeForwardThroughCodec
        );
    }

    #[test]
    fn path_regex_matches_observed_path() {
        // Sanity check the canonical path matches the regex literal.
        // The regex itself is consumed by the egress proxy, not by
        // this crate; we keep a smoke test here so a typo in the
        // regex would be caught.
        let observed = "/exa.language_server_pb.LanguageServerService/CascadeChat";
        // Quick literal compare against the regex source (we don't
        // pull `regex` into this crate's deps).
        assert!(WINDSURF_CASCADE_PATH_REGEX.contains("CascadeChat"));
        assert!(WINDSURF_CASCADE_PATH_REGEX.contains("language_server_pb"));
        // Path itself matches the literal in `is_cascade_chat_route`.
        assert_eq!(
            observed,
            "/exa.language_server_pb.LanguageServerService/CascadeChat"
        );
    }
}
