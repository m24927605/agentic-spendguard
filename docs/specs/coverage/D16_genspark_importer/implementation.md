# D16 — Implementation

Companion to [`design.md`](design.md). Lays out crate layout, key types, code skeleton, schema delta, fixture format.

## 1. Files touched

```
Cargo.toml                                       # +services/importer_genspark to workspace excludes
services/importer_genspark/                      # NEW crate
    Cargo.toml
    README.md
    build.rs                                     # (only if proto codegen needed; otherwise omit)
    src/
        lib.rs                                   # public API: import_window_from_fixture,
                                                 #             import_record_to_audit_row,
                                                 #             load_price_table
        main.rs                                  # bin: spendguard-importer-genspark
        record.rs                                # ImportRecord, AdminApiResponse parser
        price.rs                                 # GensparkPriceTable + credit→USD conversion
        audit.rs                                 # import_record_to_audit_row (pure)
        emit.rs                                  # write_audit_row + sign CloudEvent
        live.rs                                  # #[cfg(feature = "live")] reqwest client
        errors.rs                                # ImporterError enum
    tests/
        fixtures/
            genspark_usage.json                  # Plus tier, 1 workspace, 7-day window
            genspark_usage_premium.json          # Premium tier, multi-workspace
            genspark_usage_unknown_plan.json     # forces fallback path
            PROVENANCE.md                        # capture date + redaction script SHA-256
        contract.rs                              # import_record_to_audit_row unit tests
        fixture_import.rs                        # end-to-end fixture replay against test PG
        live_gating.rs                           # GENSPARK_API_TOKEN gate tests (no live HTTP)
        price_table.rs                           # price loader + unknown-plan fallback
    config/
        genspark_credit_price.toml               # committed pricing table (versioned)

services/canonical_ingest/migrations/
    0053_audit_outbox_import_genspark.sql        # +'import_genspark' reservation_source +
                                                 # +'genspark_billing' import_source CHECK
    down/
        0053_audit_outbox_import_genspark.sql    # rollback

deploy/demo/
    Makefile                                     # +demo-verify-import-genspark-fixture
    verify_step_import_genspark_fixture.sql
    runtime/import_genspark_demo.sh

docs/site-v2/src/content/docs/integrations/
    genspark-billing-importer.md                 # Starlight doc page

README.md                                        # +Adapter table row "Genspark billing importer"
```

## 2. Cargo.toml workspace update

```toml
# Cargo.toml (workspace root) — additive
[workspace]
exclude = [
  # ...existing entries...
  "services/importer_genspark",
]
```

The crate ships as workspace-excluded (matches every other `services/*` member of this repo per the captured root Cargo.toml). `cargo build -p spendguard-importer-genspark --manifest-path services/importer_genspark/Cargo.toml` is the canonical build command.

## 3. Crate Cargo.toml

```toml
# services/importer_genspark/Cargo.toml
[package]
name = "spendguard-importer-genspark"
version = "0.1.0-alpha"
edition = "2021"
license = "Apache-2.0"
publish = false
description = "D16: post-hoc billing importer for Genspark Super Agent. Reconciles vendor credit consumption into SpendGuard audit_outbox."

[[bin]]
name = "spendguard-importer-genspark"
path = "src/main.rs"

[lib]
name = "spendguard_importer_genspark"
path = "src/lib.rs"

[features]
default = []
# Pulls reqwest + the live admin-API client. OFF by default per design §3.3.
live = ["dep:reqwest", "dep:secrecy"]

[dependencies]
anyhow = "1"
thiserror = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4", "v7", "serde"] }
sha2 = "0.10"
hex = "0.4"
sqlx = { version = "0.8", default-features = false, features = [
    "runtime-tokio", "tls-rustls", "postgres", "uuid", "chrono", "json",
] }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"

# Live-mode deps — gated behind `live` feature, OFF by default.
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"], optional = true }
secrecy = { version = "0.8", optional = true }

[dev-dependencies]
tokio = { version = "1", features = ["full", "test-util"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "migrate"] }
tempfile = "3"
```

Reviewer cross-checks `[features] live = ["dep:reqwest", "dep:secrecy"]` so the default build pulls neither.

## 4. Schema migration

### 4.1 `0053_audit_outbox_import_genspark.sql`

```sql
-- D16 §3.2 — Add 'import_genspark' to reservation_source + 'genspark_billing' to
-- import_source. ADDITIVE: drops + re-creates each CHECK constraint with the
-- expanded value set. No row data changes.

BEGIN;

-- 1) Expand reservation_source CHECK (D13/0044 introduced 'byok' + 'subscription_meter').
ALTER TABLE audit_outbox
  DROP CONSTRAINT IF EXISTS audit_outbox_reservation_source_check;
ALTER TABLE audit_outbox
  ADD CONSTRAINT audit_outbox_reservation_source_check
  CHECK (reservation_source IN ('byok', 'subscription_meter', 'import_genspark'));

-- 2) Expand import_source CHECK (D13/0046 introduced anthropic / openai usage values).
ALTER TABLE audit_outbox
  DROP CONSTRAINT IF EXISTS audit_outbox_import_source_check;
ALTER TABLE audit_outbox
  ADD CONSTRAINT audit_outbox_import_source_check
  CHECK (import_source IS NULL OR import_source IN
         ('anthropic_console_usage', 'openai_admin_usage', 'genspark_billing'));

-- 3) Partial index — Genspark rows are an analytics hot path.
CREATE INDEX IF NOT EXISTS idx_audit_outbox_import_genspark
  ON audit_outbox (tenant_id, occurred_at)
  WHERE reservation_source = 'import_genspark';

COMMIT;
```

### 4.2 `down/0053_audit_outbox_import_genspark.sql`

```sql
BEGIN;
DROP INDEX IF EXISTS idx_audit_outbox_import_genspark;
-- Re-narrow CHECK constraints to the D13 set. Will FAIL if any
-- 'import_genspark' rows exist — caller must purge or migrate first.
ALTER TABLE audit_outbox
  DROP CONSTRAINT IF EXISTS audit_outbox_reservation_source_check;
ALTER TABLE audit_outbox
  ADD CONSTRAINT audit_outbox_reservation_source_check
  CHECK (reservation_source IN ('byok', 'subscription_meter'));
ALTER TABLE audit_outbox
  DROP CONSTRAINT IF EXISTS audit_outbox_import_source_check;
ALTER TABLE audit_outbox
  ADD CONSTRAINT audit_outbox_import_source_check
  CHECK (import_source IS NULL OR import_source IN
         ('anthropic_console_usage', 'openai_admin_usage'));
COMMIT;
```

## 5. Key types

### 5.1 `record.rs` — ImportRecord + admin-API parser

```rust
//! D16 — Genspark admin-API record shape. Shape is what the API actually
//! returns (recorded in `tests/fixtures/genspark_usage.json`). Field names
//! mirror the Genspark JSON contract; serde rename_all = "snake_case".

use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AdminApiResponse {
    pub records: Vec<ImportRecord>,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ImportRecord {
    /// Genspark workspace ID. Becomes tenant_id in audit_outbox.
    pub workspace_id: String,
    /// Subscription plan at the time of consumption. Maps to GensparkPriceTable.
    pub plan: String,             // "plus" | "pro" | "premium" | "unknown"
    /// Credits consumed in the window. Genspark integer.
    pub credits_consumed: i64,
    /// Window boundaries (Genspark aggregates daily by default).
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    /// Optional task category for dashboard breakdown.
    pub task_category: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Pagination {
    pub next_cursor: Option<String>,
}
```

### 5.2 `price.rs` — credit → USD

```rust
//! D16 — Versioned pricing table loader + credit conversion.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct GensparkPriceTable {
    pub pricing_version: String,
    pub plans: HashMap<String, PlanRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanRow {
    pub monthly_usd: f64,
    pub monthly_credits: i64,
    /// Cached: monthly_usd / monthly_credits. Loader fills this in.
    #[serde(default)]
    pub effective_usd_per_credit: Option<f64>,
}

impl GensparkPriceTable {
    pub fn load(path: &Path) -> Result<Self, crate::errors::ImporterError> {
        let raw = std::fs::read_to_string(path)?;
        let mut tbl: GensparkPriceTable = toml::from_str(&raw)?;
        for row in tbl.plans.values_mut() {
            if row.monthly_credits > 0 {
                row.effective_usd_per_credit =
                    Some(row.monthly_usd / row.monthly_credits as f64);
            }
        }
        Ok(tbl)
    }

    /// Returns micro-USD for `credits` under `plan`. `None` if the plan
    /// is unknown (caller emits the row with amount_micro_usd=0 +
    /// reason_code = "genspark_plan_unknown" per design §3.1).
    pub fn credits_to_micro_usd(&self, plan: &str, credits: i64) -> Option<i64> {
        let row = self.plans.get(plan)?;
        let per_credit = row.effective_usd_per_credit?;
        let usd = per_credit * credits as f64;
        Some((usd * 1_000_000.0) as i64)
    }
}
```

### 5.3 `audit.rs` — pure record → row

```rust
//! D16 — Pure conversion from ImportRecord to the audit_outbox row.
//! No IO, no global state. Mirrored on D13's importer_anthropic contract.

use crate::record::ImportRecord;
use crate::price::GensparkPriceTable;
use spendguard_common::AuditRow;

/// Pure conversion. Caller persists with `emit::write_audit_row`.
pub fn import_record_to_audit_row(
    rec: &ImportRecord,
    price: &GensparkPriceTable,
) -> AuditRow {
    let (amount_micro_usd, reason_code) = match price.credits_to_micro_usd(&rec.plan, rec.credits_consumed) {
        Some(usd) => (usd, None),
        None => (0, Some("genspark_plan_unknown".to_string())),
    };

    AuditRow {
        tenant_id: rec.workspace_id.clone(),
        reservation_source: "import_genspark".into(),
        import_source: Some("genspark_billing".into()),
        model: format!("genspark/{}", rec.plan),  // synthetic — Genspark hides upstream model
        input_tokens: 0,                           // unavailable from billing API
        output_tokens: 0,
        amount_micro_usd,
        pricing_version: price.pricing_version.clone(),
        occurred_at: rec.window_end,
        reason_code,
        ..Default::default()
    }
}
```

### 5.4 `emit.rs` — write row + sign CloudEvent

```rust
//! D16 — Persist audit row + emit signed CloudEvent
//! (type = "spendguard.audit.import.genspark_credit").

use spendguard_common::{AuditRow, CloudEvent};
use sqlx::PgPool;

pub async fn write_audit_row(pool: &PgPool, row: &AuditRow) -> anyhow::Result<()> {
    // INSERT into audit_outbox via the canonical_ingest write path
    // (importer_anthropic / openai use the same helper; here we reuse it).
    spendguard_common::audit_outbox::insert(pool, row).await?;
    Ok(())
}

pub fn build_cloudevent(row: &AuditRow) -> CloudEvent {
    CloudEvent {
        // type identifier locked in design §6 decision #5.
        ty: "spendguard.audit.import.genspark_credit".into(),
        source: "spendguard-importer-genspark".into(),
        subject: row.tenant_id.clone(),
        time: row.occurred_at,
        data: serde_json::to_vec(row).expect("AuditRow is serde-safe"),
        ..Default::default()
    }
}
```

### 5.5 `live.rs` — admin-API client (feature-gated)

```rust
//! D16 §3.3 — Live admin-API client. OFF by default; pulls reqwest
//! + secrecy ONLY when the `live` Cargo feature is enabled.

#![cfg(feature = "live")]

use crate::record::AdminApiResponse;
use chrono::{DateTime, Utc};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use std::env;

const GENSPARK_ADMIN_BASE_URL: &str = "https://api.genspark.ai/v1/admin/usage";
const MIN_TOKEN_LEN: usize = 32;

pub struct GensparkAdminClient {
    http: Client,
    token: SecretString,
}

impl GensparkAdminClient {
    /// Reads GENSPARK_API_TOKEN once at startup. Refuses to start on:
    /// - missing var
    /// - empty value
    /// - len < 32 chars (catches placeholders like "TODO")
    pub fn from_env() -> anyhow::Result<Self> {
        let raw = env::var("GENSPARK_API_TOKEN")
            .map_err(|_| anyhow::anyhow!("GENSPARK_API_TOKEN not set"))?;
        if raw.trim().is_empty() {
            anyhow::bail!("GENSPARK_API_TOKEN is empty");
        }
        if raw.len() < MIN_TOKEN_LEN {
            anyhow::bail!(
                "GENSPARK_API_TOKEN is {} chars; expected >= {}",
                raw.len(), MIN_TOKEN_LEN
            );
        }
        Ok(Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            token: SecretString::new(raw),
        })
    }

    pub async fn fetch_window(
        &self,
        workspace_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<AdminApiResponse> {
        let resp = self.http
            .get(GENSPARK_ADMIN_BASE_URL)
            .query(&[
                ("workspace", workspace_id),
                ("from", &from.to_rfc3339()),
                ("to", &to.to_rfc3339()),
            ])
            .bearer_auth(self.token.expose_secret())
            .send().await?
            .error_for_status()?;
        Ok(resp.json::<AdminApiResponse>().await?)
    }
}
```

### 5.6 `lib.rs` — public entrypoints

```rust
//! D16 — Genspark billing importer public API.

pub mod audit;
pub mod emit;
pub mod errors;
pub mod price;
pub mod record;

#[cfg(feature = "live")]
pub mod live;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::path::Path;

/// Default-mode entry point: read `fixture_path`, convert + persist.
/// Mode the CI / demo / contract tests use.
pub async fn import_window_from_fixture(
    pool: &PgPool,
    fixture_path: &Path,
    price_path: &Path,
) -> anyhow::Result<usize> {
    let raw = std::fs::read_to_string(fixture_path)?;
    let resp: record::AdminApiResponse = serde_json::from_str(&raw)?;
    let price = price::GensparkPriceTable::load(price_path)?;

    let mut written = 0;
    for rec in &resp.records {
        let row = audit::import_record_to_audit_row(rec, &price);
        emit::write_audit_row(pool, &row).await?;
        written += 1;
    }
    Ok(written)
}

/// Live mode: same shape, fetched from the admin API. Behind `live` feature.
#[cfg(feature = "live")]
pub async fn import_window_live(
    pool: &PgPool,
    client: &live::GensparkAdminClient,
    workspace_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    price_path: &Path,
) -> anyhow::Result<usize> {
    let resp = client.fetch_window(workspace_id, from, to).await?;
    let price = price::GensparkPriceTable::load(price_path)?;
    let mut written = 0;
    for rec in &resp.records {
        let row = audit::import_record_to_audit_row(rec, &price);
        emit::write_audit_row(pool, &row).await?;
        written += 1;
    }
    Ok(written)
}
```

## 6. Pricing table — `genspark_credit_price.toml`

```toml
# services/importer_genspark/config/genspark_credit_price.toml
# D16 §3.1 — Versioned, committed-source. Drift caught by tests, not by silent change.
pricing_version = "genspark-2026-06-06"

[plans.plus]
monthly_usd     = 19.99
monthly_credits = 10000

[plans.pro]
monthly_usd     = 24.99
monthly_credits = 12500

[plans.premium]
monthly_usd     = 249.99
monthly_credits = 125000
```

## 7. Bin entrypoint — `main.rs`

```rust
//! spendguard-importer-genspark CLI: one-shot worker.
//! Usage:
//!   spendguard-importer-genspark \
//!     --window-from 2026-06-01T00:00:00Z \
//!     --window-to   2026-06-07T00:00:00Z \
//!     [--fixture <path>]            # default mode
//!     [--workspace <id>]            # live mode (requires --features live + GENSPARK_API_TOKEN)
//!     --price <path>                # default: services/importer_genspark/config/genspark_credit_price.toml
//!     --database-url $DATABASE_URL

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    #[arg(long)] window_from: chrono::DateTime<chrono::Utc>,
    #[arg(long)] window_to:   chrono::DateTime<chrono::Utc>,
    #[arg(long)] fixture: Option<PathBuf>,
    #[arg(long)] workspace: Option<String>,
    #[arg(long, default_value = "services/importer_genspark/config/genspark_credit_price.toml")]
    price: PathBuf,
    #[arg(long, env = "DATABASE_URL")] database_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    let pool = sqlx::PgPool::connect(&args.database_url).await?;

    let written = match (&args.fixture, &args.workspace) {
        (Some(path), _) => {
            spendguard_importer_genspark::import_window_from_fixture(&pool, path, &args.price).await?
        }
        #[cfg(feature = "live")]
        (None, Some(ws)) => {
            let client = spendguard_importer_genspark::live::GensparkAdminClient::from_env()?;
            spendguard_importer_genspark::import_window_live(
                &pool, &client, ws, args.window_from, args.window_to, &args.price,
            ).await?
        }
        (None, None) => anyhow::bail!("--fixture <path> required in default build (live feature OFF)"),
        #[cfg(not(feature = "live"))]
        (None, Some(_)) => anyhow::bail!("--workspace requires --features live"),
    };
    tracing::info!(written, "import complete");
    Ok(())
}
```

## 8. Fixture JSON shape

`services/importer_genspark/tests/fixtures/genspark_usage.json`:

```json
{
  "window_start": "2026-06-01T00:00:00Z",
  "window_end":   "2026-06-07T00:00:00Z",
  "pagination": null,
  "records": [
    {
      "workspace_id":     "FAKE_ws_alpha",
      "plan":             "plus",
      "credits_consumed": 3200,
      "window_start":     "2026-06-01T00:00:00Z",
      "window_end":       "2026-06-07T00:00:00Z",
      "task_category":    "research"
    },
    {
      "workspace_id":     "FAKE_ws_alpha",
      "plan":             "plus",
      "credits_consumed": 1850,
      "window_start":     "2026-06-07T00:00:00Z",
      "window_end":       "2026-06-08T00:00:00Z",
      "task_category":    "code_generation"
    }
  ]
}
```

`PROVENANCE.md`:

```
# D16 — fixture provenance

| File | Captured | Operator | Source | Redaction script SHA-256 |
|------|----------|----------|--------|--------------------------|
| genspark_usage.json | 2026-06-06 | MC | Genspark admin API (synthetic — no live capture) | n/a (handcrafted from documented response shape) |
| genspark_usage_premium.json | 2026-06-06 | MC | synthetic | n/a |
| genspark_usage_unknown_plan.json | 2026-06-06 | MC | synthetic | n/a |

All workspace IDs are `FAKE_ws_*` sentinels. No prompt content present
(Genspark admin API does not return prompts). No PII.
```

## 9. Demo mode

`deploy/demo/runtime/import_genspark_demo.sh`:

```bash
#!/usr/bin/env bash
# D16 demo: replay the genspark_usage.json fixture against the demo PG and
# assert audit rows landed with reservation_source = 'import_genspark'.
set -euo pipefail

FIXTURE="${1:-services/importer_genspark/tests/fixtures/genspark_usage.json}"

docker compose -f deploy/demo/compose.yaml up -d canonical_ingest

# Apply migration 0053 if not already.
psql "$DATABASE_URL" -f services/canonical_ingest/migrations/0053_audit_outbox_import_genspark.sql

# Run the importer binary in fixture mode.
cargo run --manifest-path services/importer_genspark/Cargo.toml \
    --bin spendguard-importer-genspark -- \
    --window-from 2026-06-01T00:00:00Z \
    --window-to   2026-06-07T00:00:00Z \
    --fixture "$FIXTURE" \
    --price   services/importer_genspark/config/genspark_credit_price.toml

# Verify.
psql "$DATABASE_URL" -f deploy/demo/verify_step_import_genspark_fixture.sql
```

`deploy/demo/verify_step_import_genspark_fixture.sql`:

```sql
-- D16 §9: assert importer rows landed + no ledger row was written.
DO $$
DECLARE
    import_count INT;
    ledger_count INT;
    sum_micro    BIGINT;
BEGIN
    SELECT count(*), COALESCE(sum(amount_micro_usd), 0)
      INTO import_count, sum_micro
      FROM audit_outbox
     WHERE reservation_source = 'import_genspark'
       AND import_source      = 'genspark_billing'
       AND tenant_id LIKE 'FAKE_ws_%';
    ASSERT import_count >= 2, 'expected >= 2 imported rows, got ' || import_count;
    ASSERT sum_micro > 0, 'expected non-zero priced USD aggregate';

    SELECT count(*) INTO ledger_count
      FROM ledger_entries
     WHERE created_at > now() - interval '5 minutes'
       AND tenant_id LIKE 'FAKE_ws_%';
    ASSERT ledger_count = 0,
        'importer MUST NOT write to ledger_entries (got ' || ledger_count || ')';
END $$;
```

`deploy/demo/Makefile`:

```make
demo-verify-import-genspark-fixture: demo-up
	deploy/demo/runtime/import_genspark_demo.sh \
	    services/importer_genspark/tests/fixtures/genspark_usage.json
```

## 10. Docs page outline

`docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md`:

1. What this is (post-hoc reconciliation, **not** enforcement).
2. Why we can't enforce (Genspark runs the LLM call inside its VM — link to strategy memo §"Archetype IV").
3. How to install: subscribe to a higher Genspark tier; mint admin token; set `GENSPARK_API_TOKEN`; run binary on cron.
4. Pricing table override path (`GENSPARK_PRICE_TABLE_PATH`).
5. Unknown-plan fallback semantics (row appears unpriced, never silently mis-priced).
6. What you see in the dashboard (per-workspace credit consumption, estimated USD aggregate).
7. CloudEvent type `spendguard.audit.import.genspark_credit` for downstream consumers.
