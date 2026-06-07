//! D13 COV_60 — Subscription-Tier vs BYOK classifier.
//!
//! Runs BEFORE the sidecar opens a ledger transaction.  Inspects the
//! caller-supplied tenant_id + model_id + claim shape (proxied via the
//! `ClassifierInput`) and decides whether to route through the
//! subscription meter path (no ledger write) or the normal BYOK lane.
//!
//! Design §4.1 mandates that subscription classification requires
//! BOTH the bearer-token prefix AND the User-Agent string to match a
//! known subscription pattern — UA alone is forgeable, and operators
//! routinely use `claude-cli` with BYOK keys.  Either signal alone
//! falls back to BYOK.
//!
//! Tokens are NEVER logged beyond their first 13 characters (design
//! decision §10 — constant-length prefix extraction).

/// All inputs the classifier inspects.  Mirrors what the egress proxy
/// already extracts during routing so we don't introduce a hyper /
/// http coupling into the sidecar.
#[derive(Debug, Clone)]
pub struct ClassifierInput<'a> {
    /// Provider canonical name — "anthropic" | "openai" | "" (unknown).
    pub provider: &'a str,
    /// Model id resolved by routing (e.g. "claude-3-5-sonnet-20241022").
    pub model_id: &'a str,
    /// First 13 chars of the bearer token, or "" if not present /
    /// non-bearer scheme.
    pub auth_token_prefix: &'a str,
    /// Verbatim User-Agent header.
    pub user_agent: &'a str,
    /// Tenant id as decided by the adapter handshake.  Empty string
    /// when not yet enrolled.
    pub tenant_id: &'a str,
}

/// Classification outcome (closed enum — additive only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionKind {
    /// Bring-your-own-key, normal ledger flow.
    Byok,
    /// Claude Code Pro/Max with `sk-ant-oat01-…` OAuth token.
    ClaudeCodePro,
    /// Codex CLI signed in with ChatGPT Plus/Pro (JWT bearer).
    CodexChatGpt,
    /// Both signals match a subscription pattern but we cannot
    /// confidently name the plan — meter under "unknown" plan tag.
    UnknownSubscription,
}

impl SubscriptionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Byok => "byok",
            Self::ClaudeCodePro => "claude_code_pro",
            Self::CodexChatGpt => "codex_chatgpt",
            Self::UnknownSubscription => "unknown",
        }
    }

    pub fn is_subscription(self) -> bool {
        matches!(
            self,
            Self::ClaudeCodePro | Self::CodexChatGpt | Self::UnknownSubscription
        )
    }
}

/// Maximum bearer-token prefix the classifier is allowed to inspect
/// (design §8 decision 9 — constant-length prefix, never logged
/// beyond this cap).
pub const AUTH_TOKEN_PREFIX_CAP: usize = 13;

/// Classify a decision request as `Byok` or one of the subscription
/// kinds. Both the token prefix AND the user-agent must match a
/// known subscription pattern.
pub fn classify(input: &ClassifierInput<'_>) -> SubscriptionKind {
    // Defensive cap: never read beyond AUTH_TOKEN_PREFIX_CAP bytes of
    // the token prefix even if the caller passes a larger slice.
    let prefix_full: &str = input.auth_token_prefix;
    let prefix: &str = prefix_full
        .get(..prefix_full.len().min(AUTH_TOKEN_PREFIX_CAP))
        .unwrap_or("");
    let ua = input.user_agent;
    let provider = input.provider;

    // 12-char sentinel: `sk-ant-oat01` — the trailing dash is the 13th
    // char of the OAuth token, but the classifier caps inspection at 13
    // chars (AUTH_TOKEN_PREFIX_CAP) so callers may pass either 12 or 13
    // chars; we look for the 12-char prefix to be robust to both.
    let claude_token = prefix.starts_with("sk-ant-oat01");
    let claude_ua = ua.starts_with("claude-cli/") || ua.starts_with("claude-code/");

    let codex_token = is_likely_jwt_prefix(prefix);
    let codex_ua = ua.starts_with("codex_cli_rs/") || ua.starts_with("codex_cli/");

    match (provider, claude_token, claude_ua, codex_token, codex_ua) {
        ("anthropic", true, true, _, _) => SubscriptionKind::ClaudeCodePro,
        ("openai", _, _, true, true) => SubscriptionKind::CodexChatGpt,
        // Both signals indicate subscription but no provider match — rare; meter anyway.
        (_, true, true, _, _) => SubscriptionKind::UnknownSubscription,
        (_, _, _, true, true) => SubscriptionKind::UnknownSubscription,
        _ => SubscriptionKind::Byok,
    }
}

/// Codex CLI ChatGPT-OAuth tokens are JWTs whose serialized header
/// always begins with the base64url prefix `eyJ`. We do NOT parse the
/// JWT (no crate dep, no side-channel); we sniff this constant prefix.
///
/// BYOK OpenAI keys start with `sk-` so the two are unambiguous.
fn is_likely_jwt_prefix(prefix: &str) -> bool {
    prefix.starts_with("eyJ")
}

// ============================================================================
// TESTS  (≥ 8 cases per scope)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(provider: &'a str, token: &'a str, ua: &'a str) -> ClassifierInput<'a> {
        ClassifierInput {
            provider,
            model_id: "model-x",
            auth_token_prefix: token,
            user_agent: ua,
            tenant_id: "tenant-test",
        }
    }

    // ── BYOK happy path ──────────────────────────────────────────────────

    #[test]
    fn byok_anthropic_api_key_is_byok() {
        let r = classify(&input("anthropic", "sk-ant-api03", "claude-cli/1.4.2"));
        // Token prefix is api03 (not oat01) → BYOK regardless of UA.
        assert_eq!(r, SubscriptionKind::Byok);
    }

    #[test]
    fn byok_openai_sk_proj_is_byok() {
        let r = classify(&input("openai", "sk-proj-abc12", "codex_cli_rs/0.1.4"));
        assert_eq!(r, SubscriptionKind::Byok);
    }

    // ── ClaudeCodePro ────────────────────────────────────────────────────

    #[test]
    fn claude_oat01_token_and_claude_cli_ua_is_pro() {
        let r = classify(&input("anthropic", "sk-ant-oat01", "claude-cli/1.4.2"));
        assert_eq!(r, SubscriptionKind::ClaudeCodePro);
    }

    #[test]
    fn claude_code_ua_variant_is_pro() {
        // Newer claude-code UA prefix is also accepted.
        let r = classify(&input("anthropic", "sk-ant-oat01", "claude-code/0.2.7"));
        assert_eq!(r, SubscriptionKind::ClaudeCodePro);
    }

    #[test]
    fn claude_oat01_without_matching_ua_is_byok() {
        // Token alone is not enough (design §4.1).
        let r = classify(&input("anthropic", "sk-ant-oat01", "curl/8.4.0"));
        assert_eq!(r, SubscriptionKind::Byok);
    }

    // ── CodexChatGpt ─────────────────────────────────────────────────────

    #[test]
    fn codex_jwt_token_and_codex_cli_ua_is_codex() {
        let r = classify(&input("openai", "eyJhbGciOi", "codex_cli_rs/0.1.4"));
        assert_eq!(r, SubscriptionKind::CodexChatGpt);
    }

    #[test]
    fn codex_jwt_without_matching_ua_is_byok() {
        // UA alone forgeable — token-only must NOT match.
        let r = classify(&input("openai", "eyJhbGciOi", "node/20.10.0"));
        assert_eq!(r, SubscriptionKind::Byok);
    }

    // ── Forgery safety (UA-only forgery) ─────────────────────────────────

    #[test]
    fn forged_ua_with_byok_key_stays_byok() {
        // Operator running BYOK key but with claude-cli UA — still BYOK.
        let r = classify(&input("anthropic", "sk-ant-api03", "claude-cli/1.4.2"));
        assert_eq!(r, SubscriptionKind::Byok);
    }

    // ── Token prefix capping ─────────────────────────────────────────────

    #[test]
    fn auth_token_prefix_capped_at_13_chars() {
        // Caller passes a 100-char string; classifier must NOT panic
        // and must still recognise the first 13 chars.
        let long = "sk-ant-oat01-".to_string() + &"x".repeat(87);
        let r = classify(&input("anthropic", &long, "claude-cli/1.4.2"));
        assert_eq!(r, SubscriptionKind::ClaudeCodePro);
    }

    #[test]
    fn empty_token_and_ua_is_byok() {
        let r = classify(&input("anthropic", "", ""));
        assert_eq!(r, SubscriptionKind::Byok);
    }

    // ── Provider arm fallthrough ─────────────────────────────────────────

    #[test]
    fn unknown_provider_with_subscription_signals_falls_back_to_unknown() {
        // Both signals match Claude pro, but provider name is "" — we
        // still meter, just under unknown plan tag.
        let r = classify(&input("", "sk-ant-oat01", "claude-cli/1.4.2"));
        assert_eq!(r, SubscriptionKind::UnknownSubscription);
    }

    #[test]
    fn subscription_kind_str_representation() {
        assert_eq!(SubscriptionKind::Byok.as_str(), "byok");
        assert_eq!(SubscriptionKind::ClaudeCodePro.as_str(), "claude_code_pro");
        assert_eq!(SubscriptionKind::CodexChatGpt.as_str(), "codex_chatgpt");
        assert_eq!(SubscriptionKind::UnknownSubscription.as_str(), "unknown");
    }

    #[test]
    fn is_subscription_predicate_is_correct() {
        assert!(!SubscriptionKind::Byok.is_subscription());
        assert!(SubscriptionKind::ClaudeCodePro.is_subscription());
        assert!(SubscriptionKind::CodexChatGpt.is_subscription());
        assert!(SubscriptionKind::UnknownSubscription.is_subscription());
    }
}
