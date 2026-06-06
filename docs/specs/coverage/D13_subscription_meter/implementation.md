# D13 — Implementation

Companion to [`design.md`](design.md). Lays out crate layout, key types, code skeleton, schema changes, and the proto delta.

## 1. Files touched

```
proto/spendguard/common/v1/common.proto          # +ReservationSource enum
services/canonical_ingest/migrations/
    0044_audit_outbox_reservation_source.sql     # +reservation_source column + backfill
    0045_subscription_caps.sql                   # tenant-scoped cap config
    0046_audit_outbox_import_source.sql          # +import_source for reconciler

services/egress_proxy/
    src/lib.rs                                   # re-export new modules
    src/subscription.rs                          # NEW — classifier
    src/subscription_meter.rs                    # NEW — meter estimate + cap eval
    src/subscription_cap_store.rs                # NEW — postgres-backed CapStore
    src/routing.rs                               # +codex/responses row
    src/decision.rs                              # branch on SubscriptionKind
    src/forward.rs                               # hard-cap short circuit
    src/main.rs                                  # config wiring
    tests/fixtures/subscription/                 # NEW — HAR fixtures + PROVENANCE.md
    tests/subscription_classifier.rs
    tests/subscription_meter_e2e.rs
    tests/hard_cap_synthetic_429.rs

services/sidecar/
    src/decision/transaction.rs                  # skip ledger write when meter-only
    src/server/adapter_uds.rs                    # carry reservation_source through

services/importer_anthropic/                     # NEW — stub crate
services/importer_openai/                        # NEW — stub crate

deploy/demo/
    Makefile                                     # +demo-verify-subscription-meter-{claude_code,codex}
    verify_step_subscription_meter_claude_code.sql
    verify_step_subscription_meter_codex.sql
    runtime/subscription_meter_demo.sh

docs/site-v2/src/content/docs/integrations/
    subscription-meter-claude-code-pro.md
    subscription-meter-codex-chatgpt.md
```

## 2. Proto delta

```proto
// proto/spendguard/common/v1/common.proto
//
// Additive enum — value 0 unspecified preserves wire-compat for clients
// emitted before D13.
enum ReservationSource {
  RESERVATION_SOURCE_UNSPECIFIED = 0;
  RESERVATION_SOURCE_BYOK = 1;
  RESERVATION_SOURCE_SUBSCRIPTION_METER = 2;
}

// ClaimEstimate (existing) gets:
message ClaimEstimate {
  // ...existing fields...
  ReservationSource reservation_source = 17;  // D13 — next free tag
}
```

## 3. Schema migrations

### 3.1 `0044_audit_outbox_reservation_source.sql`

```sql
-- D13 — reservation_source tag for subscription-meter mode.
ALTER TABLE audit_outbox
  ADD COLUMN reservation_source TEXT NOT NULL DEFAULT 'byok'
    CHECK (reservation_source IN ('byok', 'subscription_meter'));

-- Backfill: every existing row predates D13 and is BYOK-ledger-charged.
-- The DEFAULT clause covers new rows; existing rows already have 'byok'
-- via the DEFAULT (no UPDATE needed on supported PG ≥ 11 fast-path).

-- Partial index — subscription rows are the analytics hot path.
CREATE INDEX idx_audit_outbox_subscription_meter
  ON audit_outbox (tenant_id, occurred_at)
  WHERE reservation_source = 'subscription_meter';
```

### 3.2 `0045_subscription_caps.sql`

```sql
CREATE TABLE subscription_caps (
    tenant_id          TEXT NOT NULL,
    budget_id          TEXT NOT NULL,
    mode               TEXT NOT NULL
        CHECK (mode IN ('meter', 'soft_cap', 'hard_cap')),
    threshold_usd      NUMERIC(20, 8) NOT NULL CHECK (threshold_usd >= 0),
    -- ISO-8601 duration; default 'P1M' = calendar month aligned UTC.
    threshold_window   TEXT NOT NULL DEFAULT 'P1M'
        CHECK (threshold_window IN ('P1D', 'P7D', 'P1M')),
    -- nullable: when null, derive from threshold_window + UTC clock.
    window_anchor_utc  TIMESTAMPTZ NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, budget_id)
);

-- Row-Level Security: each tenant only sees their own caps.
ALTER TABLE subscription_caps ENABLE ROW LEVEL SECURITY;
CREATE POLICY subscription_caps_tenant_isolation ON subscription_caps
    FOR ALL TO sidecar_runtime
    USING (tenant_id = current_setting('spendguard.tenant_id'));
```

### 3.3 `0046_audit_outbox_import_source.sql`

```sql
-- D13 §5 — importer reconciler integration point. Nullable: present
-- only for rows written by importer_anthropic / importer_openai. Live
-- proxy + sidecar rows leave this NULL.
ALTER TABLE audit_outbox
  ADD COLUMN import_source TEXT NULL
    CHECK (import_source IS NULL OR import_source IN
           ('anthropic_console_usage', 'openai_admin_usage'));

CREATE INDEX idx_audit_outbox_import_source
  ON audit_outbox (tenant_id, import_source, occurred_at)
  WHERE import_source IS NOT NULL;
```

## 4. Key types

### 4.1 `subscription.rs`

```rust
//! D13 — Subscription-tier vs BYOK classifier.
//!
//! Runs AFTER `routing::route()` and BEFORE `decision::estimate_call_cost`.
//! The classifier inspects three signals: Authorization-token prefix,
//! User-Agent, and a handful of vendor-CLI-specific headers. Both the
//! header AND the key prefix must match for non-Byok classification
//! per design §4.1 (User-Agent is forgeable; operators routinely use
//! `claude-cli` with BYOK keys).

use http::HeaderMap;
use serde_json::Value;
use crate::routing::ProviderConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionKind {
    Byok,
    ClaudeCodePro,
    CodexChatGpt,
    Unknown,
}

impl SubscriptionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Byok => "byok",
            Self::ClaudeCodePro => "claude_code_pro",
            Self::CodexChatGpt => "codex_chatgpt",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_subscription(self) -> bool {
        matches!(self, Self::ClaudeCodePro | Self::CodexChatGpt)
    }
}

/// Classify an inbound request. Defaults to `Byok` unless both the
/// Authorization prefix AND the User-Agent match a known subscription
/// pattern (design §4.1).
pub fn classify(
    cfg: &ProviderConfig,
    headers: &HeaderMap,
    _body: &Value,
) -> SubscriptionKind {
    let auth_prefix = extract_auth_token_prefix(headers);
    let user_agent  = headers.get(http::header::USER_AGENT)
                             .and_then(|v| v.to_str().ok())
                             .unwrap_or("");

    match cfg.kind {
        crate::routing::ProviderKind::Anthropic
            if auth_prefix.as_deref() == Some("sk-ant-oat01-")
               && user_agent.starts_with("claude-cli/")
        => SubscriptionKind::ClaudeCodePro,

        crate::routing::ProviderKind::OpenAi
            if is_codex_chatgpt_jwt(&auth_prefix)
               && user_agent.starts_with("codex_cli_rs/")
        => SubscriptionKind::CodexChatGpt,

        _ => SubscriptionKind::Byok,
    }
}

/// Returns the first N chars of the bearer token for prefix matching.
/// Never logs the full token. Returns `None` for non-bearer schemes.
fn extract_auth_token_prefix(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(http::header::AUTHORIZATION)?
                     .to_str().ok()?;
    let token = raw.strip_prefix("Bearer ")?;
    Some(token.chars().take(13).collect())
}

/// Codex CLI ChatGPT-OAuth tokens are JWTs; the issuer claim is
/// `https://auth.openai.com`. We do NOT parse the JWT (no jwt crate
/// dep); we sniff the base64url header byte prefix that decodes to
/// the standard JWT header.
fn is_codex_chatgpt_jwt(prefix: &Option<String>) -> bool {
    prefix.as_deref().map(|p| p.starts_with("eyJ")).unwrap_or(false)
}
```

### 4.2 `subscription_meter.rs`

```rust
//! D13 — Meter-only estimate + cap evaluation.
//!
//! No `sidecar.RequestDecision` call, no `reservations` write — this
//! is the explicit fork from `decision::estimate_call_cost` when the
//! classifier returns a subscription kind.

use crate::decision::PricingSnapshot;
use crate::routing::ProviderConfig;

#[derive(Debug, Clone)]
pub struct MeterEstimate {
    pub input_tokens: i64,
    pub estimated_output_tokens: i64,
    pub estimated_amount_micro_usd: i64,
    pub pricing_version: String,
    pub model: String,
}

pub async fn meter_only_estimate(
    cfg: &ProviderConfig,
    body: &serde_json::Value,
    pricing: &PricingSnapshot,
    tokenizer: &spendguard_tokenizer::Tokenizer,
) -> anyhow::Result<MeterEstimate> {
    // 1) Token count via tokenizer library (same path as BYOK).
    let model = crate::routing::resolve_model_id(cfg, "", body);
    let messages = serialize_messages_for_tokenizer(body);
    let tok_resp = tokenizer.encode(cfg.tokenizer_kind, &messages).await?;

    // 2) Output prediction: Strategy A fallback (no predictor call —
    //    subscription meter is best-effort, predictor is BYOK-tuned).
    let predicted_output = body.get("max_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(4096);

    // 3) Look up retail price from pricing_table (cf_price_per_million).
    let price_input  = price_lookup(pricing, cfg.kind.as_str(), &model, "input")?;
    let price_output = price_lookup(pricing, cfg.kind.as_str(), &model, "output")?;

    let amount_micro_usd =
        (tok_resp.input_tokens * price_input  / 1_000_000) +
        (predicted_output      * price_output / 1_000_000);

    Ok(MeterEstimate {
        input_tokens: tok_resp.input_tokens,
        estimated_output_tokens: predicted_output,
        estimated_amount_micro_usd: amount_micro_usd,
        pricing_version: pricing.pricing.pricing_version.clone(),
        model,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionMode { Meter, SoftCap, HardCap }

#[derive(Debug, Clone)]
pub enum CapDecision {
    Pass,
    SoftCapAlert(AlertPayload),
    HardCapBlock(Block429),
}

#[derive(Debug, Clone)]
pub struct AlertPayload {
    pub tenant_id: String,
    pub budget_id: String,
    pub threshold_usd: f64,
    pub used_usd: f64,
    pub mode: SubscriptionMode,
}

#[derive(Debug, Clone)]
pub struct Block429 {
    pub retry_after_seconds: i64,
    pub message: String,
}

#[async_trait::async_trait]
pub trait SubscriptionCapStore: Send + Sync {
    async fn fetch_cap(&self, tenant_id: &str, budget_id: &str)
        -> anyhow::Result<Option<CapRow>>;
    async fn used_in_window(&self, tenant_id: &str, budget_id: &str,
                            window_start_utc: chrono::DateTime<chrono::Utc>)
        -> anyhow::Result<i64>;  // micro_usd
}

pub async fn evaluate_cap(
    tenant_id: &str,
    budget_id: &str,
    meter: &MeterEstimate,
    mode: SubscriptionMode,
    store: &dyn SubscriptionCapStore,
) -> CapDecision {
    if matches!(mode, SubscriptionMode::Meter) {
        return CapDecision::Pass;
    }
    let cap = match store.fetch_cap(tenant_id, budget_id).await {
        Ok(Some(c)) => c,
        _ => return CapDecision::Pass,  // no cap configured → behave as meter
    };
    let window_start = compute_window_start(&cap);
    let used = store.used_in_window(tenant_id, budget_id, window_start)
                    .await.unwrap_or(0);
    let projected = used + meter.estimated_amount_micro_usd;
    let threshold_micro = (cap.threshold_usd * 1_000_000.0) as i64;

    if projected < threshold_micro {
        return CapDecision::Pass;
    }

    match mode {
        SubscriptionMode::SoftCap => CapDecision::SoftCapAlert(AlertPayload {
            tenant_id: tenant_id.into(), budget_id: budget_id.into(),
            threshold_usd: cap.threshold_usd, used_usd: used as f64 / 1e6,
            mode,
        }),
        SubscriptionMode::HardCap => CapDecision::HardCapBlock(Block429 {
            retry_after_seconds: seconds_until_window_reset(&cap),
            message: "spendguard subscription cap reached".into(),
        }),
        SubscriptionMode::Meter => unreachable!(),
    }
}
```

### 4.3 `forward.rs` hard-cap short circuit

```rust
// services/egress_proxy/src/forward.rs (additions only)
//
// Existing flow: route → estimate → sidecar.RequestDecision → upstream
// D13 fork:
//   if classify() != Byok:
//     meter_only_estimate
//     cap_decision = evaluate_cap(...)
//     match cap_decision {
//       Pass            => emit meter audit + proxy upstream
//       SoftCapAlert(p) => emit meter audit + dispatch_alert(p)
//                          + write stderr warning header + proxy upstream
//       HardCapBlock(b) => emit meter audit (decision=STOP_RUN_PROJECTION)
//                          + return synthetic 429 (no upstream call)
//     }

fn synthetic_429(block: &Block429) -> hyper::Response<hyper::Body> {
    hyper::Response::builder()
        .status(429)
        .header("Retry-After", block.retry_after_seconds.to_string())
        .header("Content-Type", "application/json")
        .body(hyper::Body::from(format!(
            r#"{{"error":{{"type":"rate_limit_exceeded","message":"{}","code":"spendguard_subscription_cap"}}}}"#,
            block.message
        )))
        .expect("static 429 body")
}
```

## 4.4 Sidecar branch

`services/sidecar/src/decision/transaction.rs` adds at the top of the
ledger write path:

```rust
if request.reservation_source ==
       common_pb::ReservationSource::SubscriptionMeter as i32
{
    // Skip ledger_entries + reservations writes. Still write
    // audit_outbox so dashboards see the meter row.
    return write_audit_only(&tx, &request, &context).await;
}
```

The proto field on `DecisionRequest` is plumbed through
`adapter_uds.rs::request_decision_inner` — purely additive, no
existing arm changes.

## 5. Importer crate skeletons (D13 §5)

`services/importer_anthropic/Cargo.toml`:

```toml
[package]
name = "spendguard-importer-anthropic"
version = "0.0.1"
edition = "2021"
publish = false

[features]
default = []
# All real logic gated behind this feature; D13 ships an empty stub.
live = []
stub = []

[dependencies]
spendguard-common = { path = "../common" }
anyhow = "1"
```

`services/importer_anthropic/src/lib.rs`:

```rust
//! Anthropic Console Usage API → audit_outbox reconciler.
//!
//! D13 §5: ships as a stub. The `import_record_to_audit_row` contract
//! is locked here so the live implementation (when Anthropic's Admin
//! API opens) is a drop-in.

pub struct ImportRecord {
    pub workspace_id: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub usd_amount: f64,
    pub window_start: chrono::DateTime<chrono::Utc>,
    pub window_end: chrono::DateTime<chrono::Utc>,
}

/// The contract every importer impl must satisfy. Tested in §5 by a
/// STUB integration test that asserts the row shape matches the
/// `audit_outbox` schema (PG 0046 import_source column).
pub fn import_record_to_audit_row(rec: &ImportRecord)
    -> spendguard_common::AuditRow
{
    spendguard_common::AuditRow {
        tenant_id: rec.workspace_id.clone(),
        reservation_source: "subscription_meter".into(),
        import_source: Some("anthropic_console_usage".into()),
        model: rec.model.clone(),
        input_tokens: rec.input_tokens,
        output_tokens: rec.output_tokens,
        amount_micro_usd: (rec.usd_amount * 1e6) as i64,
        occurred_at: rec.window_end,
        // ...rest of mandatory fields zeroed via Default::default()
        ..Default::default()
    }
}

#[cfg(feature = "live")]
pub mod live { /* Day-2: poll Anthropic Console API */ }
```

`services/importer_openai/` is structurally identical, with `import_source = "openai_admin_usage"`.

## 6. Demo modes

`deploy/demo/runtime/subscription_meter_demo.sh`:

```bash
#!/usr/bin/env bash
# D13 demo: replay a fixture HAR through the egress proxy and assert
# the meter audit row was written with reservation_source=subscription_meter.
set -euo pipefail

FIXTURE="${1:?fixture name required}"  # claude_code | codex
MODE="${2:-meter}"                     # meter | soft_cap | hard_cap

# Spin up proxy + sidecar via compose
docker compose -f deploy/demo/compose.yaml up -d egress_proxy sidecar canonical_ingest

# Set the subscription mode for the demo tenant
psql "$DATABASE_URL" -c "INSERT INTO subscription_caps
    (tenant_id, budget_id, mode, threshold_usd)
    VALUES ('demo', 'subscription-budget', '${MODE}', 0.50)
    ON CONFLICT (tenant_id, budget_id) DO UPDATE SET mode = EXCLUDED.mode;"

# Replay the HAR fixture against the proxy
python3 deploy/demo/runtime/replay_har.py \
    --har "services/egress_proxy/tests/fixtures/subscription/${FIXTURE}_session.har" \
    --proxy http://localhost:8443

# Verify
psql "$DATABASE_URL" -f "deploy/demo/verify_step_subscription_meter_${FIXTURE}.sql"
```

`deploy/demo/Makefile`:

```make
demo-verify-subscription-meter-claude-code: demo-up
	deploy/demo/runtime/subscription_meter_demo.sh claude_code meter

demo-verify-subscription-meter-codex: demo-up
	deploy/demo/runtime/subscription_meter_demo.sh codex meter

demo-verify-subscription-hard-cap: demo-up
	deploy/demo/runtime/subscription_meter_demo.sh claude_code hard_cap
```

`deploy/demo/verify_step_subscription_meter_claude_code.sql`:

```sql
-- D13 §6: assert the meter row was written + the ledger was NOT.
DO $$
DECLARE
    meter_count INT;
    ledger_count INT;
BEGIN
    SELECT count(*) INTO meter_count
      FROM audit_outbox
     WHERE tenant_id = 'demo'
       AND reservation_source = 'subscription_meter'
       AND model LIKE 'claude%';
    ASSERT meter_count >= 1, 'expected ≥ 1 meter audit row for claude code session';

    SELECT count(*) INTO ledger_count
      FROM ledger_entries
     WHERE tenant_id = 'demo'
       AND created_at > now() - interval '5 minutes';
    ASSERT ledger_count = 0,
        'subscription meter MUST NOT write to ledger_entries (got ' || ledger_count || ')';
END $$;
```

## 7. Docs page outline

`docs/site-v2/src/content/docs/integrations/subscription-meter-claude-code-pro.md`:

1. What this is (meter, not enforcement).
2. Why we can't enforce (Anthropic settles internally — link to strategy memo).
3. Three modes (meter / soft_cap / hard_cap) with UX trade-offs.
4. How to install (D02 install + set `SPENDGUARD_SUBSCRIPTION_MODE` + configure cap).
5. What you see in the dashboard (estimated retail $, not actual billing).
6. When Anthropic ships the Admin API, the importer will reconcile.
7. Hard-cap warning: shows operator a synthetic 429 → CLI appears broken.

Symmetric page for Codex / ChatGPT-OAuth.
