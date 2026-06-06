# D15 — Implementation

Companion to [`design.md`](design.md). Lays out crate layout, key types, fixture format, migration SQL, and the live HTTP client behind the `live` feature.

## 1. Files touched

```
services/importer_manus/                              # NEW crate
    Cargo.toml
    README.md
    src/
        lib.rs                                        # module re-exports
        record.rs                                     # UsageRecord + ImportRecord
        pricing.rs                                    # credit_to_usd_micros + price table loader
        fixture.rs                                    # load_fixture() — JSON deserialise
        audit.rs                                      # import_record_to_audit_row()
        error.rs                                      # ImporterError + MeterError
        live.rs                                       # #[cfg(feature = "live")] HTTP client
    assets/
        price_table.toml                              # tier → micro-USD/credit
    tests/
        fixtures/
            manus_usage.json                          # 8 sessions × 3 tiers
            PROVENANCE.md                             # redaction script SHA-256
        contract.rs                                   # CHECK round-trip
        fixture_e2e.rs                                # fixture import end-to-end
        live_mock.rs                                  # #[cfg(feature = "live")] httpmock test

services/canonical_ingest/migrations/
    0047_audit_outbox_extend_reservation_source.sql   # extend CHECK ← import_* family
    0048_audit_outbox_extend_import_source.sql        # extend CHECK ← *_admin_usage
    down/
        0047_audit_outbox_extend_reservation_source_down.sql
        0048_audit_outbox_extend_import_source_down.sql

services/canonical_ingest/
    migration_inventory.toml                          # +0047, +0048 entries

services/canonical_ingest/src/
    append_audit_outbox.rs                            # accept import_source param

services/outbox_forwarder/src/
    cloudevent_types.rs                               # register spendguard.audit.import.manus_credit

deploy/demo/
    Makefile                                          # +demo-import-manus-fixture
    runtime/import_manus_demo.sh                      # NEW
    verify_step_import_manus.sql                      # NEW

docs/site-v2/src/content/docs/integrations/
    manus-importer.md                                 # NEW Starlight page

Cargo.toml                                            # +members = ["services/importer_manus"]
README.md                                             # +Adapter integrations row
```

## 2. Cargo.toml (default features pull NO HTTP)

```toml
[package]
name = "spendguard-importer-manus"
version = "0.0.1"
edition = "2021"
publish = false
description = "Manus admin REST → SpendGuard audit_outbox reconciler (Archetype IV)"

[features]
default = []
# All HTTP / live polling behind this flag. Default build is pure ETL.
live = ["reqwest", "tokio/rt-multi-thread"]

[dependencies]
spendguard-common  = { path = "../common" }
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
toml               = "0.8"
chrono             = { version = "0.4", features = ["serde"] }
anyhow             = "1"
thiserror          = "1"
tracing            = "0.1"

# Live-mode only — kept optional to keep default build HTTP-free.
reqwest            = { version = "0.12", default-features = false,
                       features = ["json", "rustls-tls"], optional = true }
tokio              = { version = "1", default-features = false, features = ["macros"] }

[dev-dependencies]
httpmock           = "0.7"
tokio              = { version = "1", features = ["macros", "rt-multi-thread"] }
sqlx               = { version = "0.7", features = ["postgres", "runtime-tokio-rustls"] }
```

Verification (acceptance `A2.5`): `cargo tree -p spendguard-importer-manus -e=normal | grep -E 'reqwest|hyper-tls'` returns nothing.

## 3. Key types

### 3.1 `record.rs`

```rust
//! D15 — Wire-shape (UsageRecord, from admin REST) and internal-shape
//! (ImportRecord, after tier validation). The split keeps deserialisation
//! tolerant (extra fields ignored) and downstream code total over a
//! validated subset.

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Single session record from `GET /v1/usage`. Extra fields ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct UsageRecord {
    pub session_id: String,
    pub workspace_id: String,
    pub tier: String,
    pub credits_consumed: i64,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

/// Validated, tier-resolved record. All downstream code uses this shape.
#[derive(Debug, Clone)]
pub struct ImportRecord {
    pub session_id: String,
    pub workspace_id: String,
    pub tier: Tier,
    pub credits_consumed: i64,
    pub status: SessionStatus,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    TeamPlan,
    Enterprise,
    EnterpriseByok,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Completed,
    Failed,
    Cancelled,
    InProgress,  // skipped — admin API can list in-flight sessions; we ignore
}
```

### 3.2 `pricing.rs`

```rust
//! D15 — Credit → micro-USD conversion. Pure, no IO, integer math.

use crate::error::MeterError;
use crate::record::{ImportRecord, Tier};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct PriceTable {
    pub tiers: HashMap<String, TierPricing>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TierPricing {
    pub credit_cost_micro_usd: i64,
}

impl PriceTable {
    pub fn load_embedded() -> Self {
        const TOML: &str = include_str!("../assets/price_table.toml");
        toml::from_str(TOML).expect("embedded price_table.toml is well-formed")
    }

    pub fn credit_cost(&self, tier: Tier) -> Result<i64, MeterError> {
        let key = match tier {
            Tier::TeamPlan       => "team_plan",
            Tier::Enterprise     => "enterprise",
            Tier::EnterpriseByok => "enterprise_byok",
        };
        self.tiers.get(key)
            .map(|t| t.credit_cost_micro_usd)
            .ok_or(MeterError::UnknownTier(tier))
    }
}

/// Pure: `credits * micro_usd_per_credit`. Saturating multiply guards
/// against i64 overflow on hostile fixture input.
pub fn credit_to_usd_micros(
    rec: &ImportRecord,
    table: &PriceTable,
) -> Result<i64, MeterError> {
    let per_credit = table.credit_cost(rec.tier)?;
    let total = rec.credits_consumed.saturating_mul(per_credit);
    if total < 0 {
        return Err(MeterError::NegativeAmount);
    }
    Ok(total)
}
```

### 3.3 `fixture.rs`

```rust
//! D15 — Fixture loader. Wraps `serde_json` with explicit per-record
//! tier validation so a bad fixture fails at load time, not at insert.

use crate::error::ImporterError;
use crate::record::{ImportRecord, SessionStatus, Tier, UsageRecord};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct UsageEnvelope {
    sessions: Vec<UsageRecord>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

pub fn load_fixture<P: AsRef<Path>>(path: P) -> Result<Vec<ImportRecord>, ImporterError> {
    let raw = std::fs::read_to_string(path)?;
    let env: UsageEnvelope = serde_json::from_str(&raw)?;
    env.sessions.into_iter().map(validate_record).collect()
}

fn validate_record(rec: UsageRecord) -> Result<ImportRecord, ImporterError> {
    let tier = match rec.tier.as_str() {
        "team_plan"       => Tier::TeamPlan,
        "enterprise"      => Tier::Enterprise,
        "enterprise_byok" => Tier::EnterpriseByok,
        other => return Err(ImporterError::UnknownTier(other.into())),
    };
    let status = match rec.status.as_str() {
        "completed"   => SessionStatus::Completed,
        "failed"      => SessionStatus::Failed,
        "cancelled"   => SessionStatus::Cancelled,
        "in_progress" => SessionStatus::InProgress,
        other => return Err(ImporterError::UnknownStatus(other.into())),
    };
    if rec.credits_consumed < 0 {
        return Err(ImporterError::NegativeCredits);
    }
    Ok(ImportRecord {
        session_id: rec.session_id,
        workspace_id: rec.workspace_id,
        tier,
        credits_consumed: rec.credits_consumed,
        status,
        window_start: rec.started_at,
        window_end: rec.completed_at,
    })
}
```

### 3.4 `audit.rs`

```rust
//! D15 — Convert ImportRecord → AuditRow with the canonical
//! reservation_source / import_source tagging. Pure.

use crate::error::MeterError;
use crate::pricing::{credit_to_usd_micros, PriceTable};
use crate::record::ImportRecord;
use spendguard_common::AuditRow;

/// Canonical reservation_source value for Manus imports.
pub const RESERVATION_SOURCE: &str = "import_manus";

/// Canonical import_source value (matches mig 0048 CHECK).
pub const IMPORT_SOURCE: &str = "manus_admin_usage";

/// CloudEvent type emitted by outbox_forwarder for these rows.
pub const CLOUDEVENT_TYPE: &str = "spendguard.audit.import.manus_credit";

pub fn import_record_to_audit_row(
    rec: &ImportRecord,
    table: &PriceTable,
) -> Result<AuditRow, MeterError> {
    let amount_micro_usd = credit_to_usd_micros(rec, table)?;
    Ok(AuditRow {
        tenant_id: rec.workspace_id.clone(),
        reservation_source: RESERVATION_SOURCE.into(),
        import_source: Some(IMPORT_SOURCE.into()),
        // Model is unknown — Manus does not expose per-LLM-call detail.
        // We tag with the deterministic synthetic identifier so downstream
        // analytics can group by it.
        model: "manus.session/credit".into(),
        input_tokens: 0,
        output_tokens: 0,
        amount_micro_usd,
        occurred_at: rec.window_end,
        // Carry the session ID into the dedupe key so two imports of the
        // same admin-API window are idempotent.
        dedupe_key: Some(format!("manus:{}", rec.session_id)),
        ..Default::default()
    })
}
```

### 3.5 `live.rs` (feature-gated)

```rust
//! D15 — Live HTTP polling against api.manus.ai. Compiled only when
//! `--features live` is set; default build pulls no reqwest.

#![cfg(feature = "live")]

use crate::error::ImporterError;
use crate::fixture::validate_record_public as _;  // re-exported helper
use crate::record::{ImportRecord, UsageRecord};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::env;

const DEFAULT_BASE_URL: &str = "https://api.manus.ai";

#[derive(Debug, Deserialize)]
struct UsageEnvelope {
    sessions: Vec<UsageRecord>,
    next_cursor: Option<String>,
}

pub struct LiveClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl LiveClient {
    /// Construct from `MANUS_API_TOKEN`; returns `MissingToken` if unset.
    pub fn from_env() -> Result<Self, ImporterError> {
        let token = env::var("MANUS_API_TOKEN")
            .map_err(|_| ImporterError::MissingToken)?;
        if token.is_empty() {
            return Err(ImporterError::MissingToken);
        }
        let base_url = env::var("MANUS_API_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let http = reqwest::Client::builder()
            .user_agent(concat!("spendguard-importer-manus/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self { http, base_url, token })
    }

    /// Poll `[since, until)` with cursor pagination. Returns validated
    /// records (unknown tiers fail-closed and are dropped with a WARN).
    pub async fn poll_usage(
        &self,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<Vec<ImportRecord>, ImporterError> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let url = format!("{}/v1/usage", self.base_url);
            let mut req = self.http.get(&url)
                .bearer_auth(&self.token)
                .query(&[("since", since.to_rfc3339()),
                         ("until", until.to_rfc3339())]);
            if let Some(c) = &cursor {
                req = req.query(&[("cursor", c)]);
            }
            let env: UsageEnvelope = req.send().await?
                .error_for_status()?
                .json().await?;
            for raw in env.sessions {
                match crate::fixture::validate_record_public(raw) {
                    Ok(rec) => out.push(rec),
                    Err(e)  => tracing::warn!(error = ?e,
                                  "skipping malformed Manus session"),
                }
            }
            match env.next_cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
        }
        Ok(out)
    }
}
```

## 4. Migrations

### 4.1 `0047_audit_outbox_extend_reservation_source.sql`

```sql
-- D15 — extend reservation_source CHECK to admit the import_* family.
-- Additive: drops + re-adds the CHECK to enumerate the new values.
-- Pre-existing rows stay valid (all existing values are in the new set).

ALTER TABLE audit_outbox
  DROP CONSTRAINT audit_outbox_reservation_source_check;

ALTER TABLE audit_outbox
  ADD CONSTRAINT audit_outbox_reservation_source_check
  CHECK (reservation_source IN (
    'byok',
    'subscription_meter',
    'import_devin',
    'import_manus',
    'import_genspark'
  ));

-- Partial index for the import_manus analytics path. Mirrors the
-- subscription_meter partial index from D13 mig 0044.
CREATE INDEX idx_audit_outbox_import_manus
  ON audit_outbox (tenant_id, occurred_at)
  WHERE reservation_source = 'import_manus';
```

### 4.2 `0048_audit_outbox_extend_import_source.sql`

```sql
-- D15 — extend import_source CHECK to admit the *_admin_usage family.
-- Additive: drops + re-adds the CHECK. NULL still permitted (live proxy
-- and sidecar rows never set this column).

ALTER TABLE audit_outbox
  DROP CONSTRAINT audit_outbox_import_source_check;

ALTER TABLE audit_outbox
  ADD CONSTRAINT audit_outbox_import_source_check
  CHECK (import_source IS NULL OR import_source IN (
    'anthropic_console_usage',
    'openai_admin_usage',
    'devin_admin_usage',
    'manus_admin_usage',
    'genspark_admin_usage'
  ));
```

Down migrations symmetric — re-add the narrower D13 CHECK.

### 4.3 `migration_inventory.toml` delta

```toml
[[migrations]]
filename = "0047_audit_outbox_extend_reservation_source.sql"
sha256   = "<pinned at impl time>"

[[migrations]]
filename = "0048_audit_outbox_extend_import_source.sql"
sha256   = "<pinned at impl time>"
```

## 5. Fixture JSON

`services/importer_manus/tests/fixtures/manus_usage.json`:

```json
{
  "sessions": [
    {
      "session_id": "ses_FAKE_team_completed_001",
      "workspace_id": "ws_FAKE_team_001",
      "tier": "team_plan",
      "credits_consumed": 47,
      "status": "completed",
      "started_at": "2026-06-05T14:22:08Z",
      "completed_at": "2026-06-05T14:34:51Z"
    },
    {
      "session_id": "ses_FAKE_team_failed_002",
      "workspace_id": "ws_FAKE_team_001",
      "tier": "team_plan",
      "credits_consumed": 12,
      "status": "failed",
      "started_at": "2026-06-05T15:01:11Z",
      "completed_at": "2026-06-05T15:02:42Z"
    },
    {
      "session_id": "ses_FAKE_team_cancelled_003",
      "workspace_id": "ws_FAKE_team_002",
      "tier": "team_plan",
      "credits_consumed": 0,
      "status": "cancelled",
      "started_at": "2026-06-05T16:00:00Z",
      "completed_at": "2026-06-05T16:00:30Z"
    },
    {
      "session_id": "ses_FAKE_team_inprogress_004",
      "workspace_id": "ws_FAKE_team_002",
      "tier": "team_plan",
      "credits_consumed": 8,
      "status": "in_progress",
      "started_at": "2026-06-05T17:00:00Z",
      "completed_at": "2026-06-05T17:00:00Z"
    },
    {
      "session_id": "ses_FAKE_enterprise_005",
      "workspace_id": "ws_FAKE_ent_001",
      "tier": "enterprise",
      "credits_consumed": 350,
      "status": "completed",
      "started_at": "2026-06-05T09:11:00Z",
      "completed_at": "2026-06-05T11:48:00Z"
    },
    {
      "session_id": "ses_FAKE_byok_006",
      "workspace_id": "ws_FAKE_byok_001",
      "tier": "enterprise_byok",
      "credits_consumed": 1024,
      "status": "completed",
      "started_at": "2026-06-05T20:00:00Z",
      "completed_at": "2026-06-05T22:30:00Z"
    },
    {
      "session_id": "ses_FAKE_team_large_007",
      "workspace_id": "ws_FAKE_team_001",
      "tier": "team_plan",
      "credits_consumed": 950,
      "status": "completed",
      "started_at": "2026-06-04T08:00:00Z",
      "completed_at": "2026-06-04T18:00:00Z"
    },
    {
      "session_id": "ses_FAKE_team_minimal_008",
      "workspace_id": "ws_FAKE_team_003",
      "tier": "team_plan",
      "credits_consumed": 1,
      "status": "completed",
      "started_at": "2026-06-05T12:00:00Z",
      "completed_at": "2026-06-05T12:01:00Z"
    }
  ],
  "next_cursor": null
}
```

`PROVENANCE.md` lists capture date `2026-06-06`, redaction script `scripts/redact_har.py` SHA-256 pin, and a no-PII assertion identical to D13 §7.

## 6. Demo runtime

`deploy/demo/runtime/import_manus_demo.sh`:

```bash
#!/usr/bin/env bash
# D15 demo: replay manus_usage.json through importer_manus and assert
# audit rows landed with reservation_source = 'import_manus'.
set -euo pipefail

docker compose -f deploy/demo/compose.yaml up -d canonical_ingest postgres

# Run the importer in fixture mode (binary built by the crate).
cargo run -p spendguard-importer-manus --bin import_manus_fixture -- \
    --fixture services/importer_manus/tests/fixtures/manus_usage.json \
    --canonical-ingest http://localhost:8081

psql "$DATABASE_URL" -f deploy/demo/verify_step_import_manus.sql
```

`deploy/demo/verify_step_import_manus.sql`:

```sql
DO $$
DECLARE
    manus_row_count INT;
    ledger_row_count INT;
    team_total_micro_usd BIGINT;
    expected_team_micro_usd BIGINT;
BEGIN
    -- Count: 7 completed/failed/cancelled rows (in_progress skipped).
    SELECT count(*) INTO manus_row_count
      FROM audit_outbox
     WHERE reservation_source = 'import_manus';
    ASSERT manus_row_count = 7,
        'expected 7 Manus audit rows (in_progress skipped), got ' || manus_row_count;

    -- Ledger MUST be empty for the importer path.
    SELECT count(*) INTO ledger_row_count
      FROM ledger_entries
     WHERE created_at > now() - interval '5 minutes';
    ASSERT ledger_row_count = 0,
        'Manus importer MUST NOT write to ledger_entries (got '
        || ledger_row_count || ')';

    -- team_plan credits across the 5 surviving team rows:
    --   47 + 12 + 0 + 950 + 1 = 1010 credits × 20_526 micro-USD = 20_731_260
    SELECT coalesce(sum(amount_micro_usd), 0) INTO team_total_micro_usd
      FROM audit_outbox
     WHERE reservation_source = 'import_manus'
       AND tenant_id LIKE 'ws_FAKE_team_%';
    expected_team_micro_usd := 1010 * 20526;
    ASSERT team_total_micro_usd = expected_team_micro_usd,
        'team_plan total expected ' || expected_team_micro_usd
        || ' got ' || team_total_micro_usd;
END $$;
```

`deploy/demo/Makefile`:

```make
demo-import-manus-fixture: demo-up
	deploy/demo/runtime/import_manus_demo.sh
```

## 7. Docs page outline

`docs/site-v2/src/content/docs/integrations/manus-importer.md`:

1. What this is (post-hoc reconciliation, not enforcement).
2. Why we can't enforce (Manus is Archetype IV — vendor-managed VMs, no
   per-LLM-call hook surface). Cross-link to strategy memo.
3. How fixture mode works (record the admin API response → replay).
4. How live mode works (set `MANUS_API_TOKEN`, run with `--features live`,
   schedule via cron or k8s CronJob).
5. Pricing override: how to edit the tier table for `enterprise` custom rates.
6. Dashboard view: rows tagged `reservation_source = import_manus`,
   `model = manus.session/credit`.
7. Sibling integrations: link to D14 Devin importer + D16 Genspark importer.

`is:raw` wrap on the JSON example per project memory.
