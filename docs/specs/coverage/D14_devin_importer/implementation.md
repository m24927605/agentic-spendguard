# D14 — Implementation

Companion to [`design.md`](design.md). Lays out crate layout, key types, schema delta, and the live HTTP client behind the `live` feature flag.

## 1. Files touched

```
services/canonical_ingest/migrations/
    0047_audit_outbox_import_source_devin.sql   # widen CHECK to include 'devin_team_api'

services/importer_devin/                        # NEW crate
    Cargo.toml
    README.md
    assets/
        devin_acu_prices.json                   # price table asset
    src/
        lib.rs                                  # re-exports
        import_record.rs                        # ImportRecord + import_record_to_audit_row
        acu_price_table.rs                      # loader + conversion
        cloudevent_envelope.rs                  # CloudEvent builder
        fixture_loader.rs                       # default-feature fixture loader
        live/
            mod.rs                              # gated on `live` feature
            client.rs                           # reqwest-based DevinClient
            poll_loop.rs                        # cron/interval poll harness
            errors.rs                           # surface 401/403/429/5xx
        bin/
            importer_devin.rs                   # binary entrypoint
    tests/
        fixtures/
            devin_usage.json                    # canonical sanitized snapshot
            PROVENANCE.md                       # generator script SHA-256
        fixture_round_trip.rs
        cloudevent_envelope_golden.rs
        pg_check_constraint.rs                  # mig 0047 round-trip
        live_client_wiremock.rs                 # gated on `live`

docs/specs/coverage/D14_devin_importer/
    cloudevent-schema.md                        # sibling schema doc (NEW)

deploy/demo/
    Makefile                                    # +demo-verify-import-devin-fixture
    runtime/import_devin_fixture_demo.sh
    verify_step_import_devin_fixture.sql

docs/site-v2/src/content/docs/integrations/
    devin-billing-importer.md                   # Starlight integration page

README.md                                       # +adapter row
```

## 2. Schema migration

### 2.1 `0047_audit_outbox_import_source_devin.sql`

```sql
-- D14 — widen import_source CHECK to allow 'devin_team_api'.
-- D13 mig 0046 added the column with two values; D14 is purely additive.
ALTER TABLE audit_outbox
  DROP CONSTRAINT IF EXISTS audit_outbox_import_source_check;

ALTER TABLE audit_outbox
  ADD CONSTRAINT audit_outbox_import_source_check
    CHECK (import_source IS NULL OR import_source IN
           ('anthropic_console_usage',
            'openai_admin_usage',
            'devin_team_api'));
```

`migration_inventory.toml` updated with mig 0047 checksum (per existing convention).

## 3. Crate layout

### 3.1 `Cargo.toml`

```toml
[package]
name        = "spendguard-importer-devin"
version     = "0.0.1"
edition     = "2021"
publish     = false

[features]
default = []
# All HTTP and async runtime gated behind this feature; default build is
# pure-Rust no-IO contract surface.
live    = ["dep:reqwest", "dep:tokio", "dep:url"]

[dependencies]
spendguard-common = { path = "../common" }
anyhow            = "1"
chrono            = { version = "0.4", features = ["serde"] }
serde             = { version = "1", features = ["derive"] }
serde_json        = "1"
uuid              = { version = "1", features = ["v7", "serde"] }
sha2              = "0.10"

# Live-only deps — optional, do NOT appear in default-feature cargo tree.
reqwest = { version = "0.12", default-features = false,
            features = ["json", "rustls-tls"], optional = true }
tokio   = { version = "1", features = ["rt", "rt-multi-thread", "macros",
            "time", "signal"], optional = true }
url     = { version = "2", optional = true }

[dev-dependencies]
sqlx       = { version = "0.8", features = ["runtime-tokio", "postgres",
                                            "macros", "chrono", "uuid"] }
wiremock   = "0.6"
tokio      = { version = "1", features = ["macros", "rt-multi-thread"] }
```

### 3.2 `src/lib.rs`

```rust
//! D14 — Devin billing importer.
//!
//! Fully managed cloud-agent (Archetype IV) — SpendGuard cannot gate
//! Devin sessions. This crate imports post-hoc usage from the Devin
//! Team API, converts ACU → estimated USD via a vendored price table,
//! and emits `spendguard.audit.import.devin_acu` CloudEvents.

pub mod acu_price_table;
pub mod cloudevent_envelope;
pub mod fixture_loader;
pub mod import_record;

#[cfg(feature = "live")]
pub mod live;

pub use acu_price_table::{AcuPriceTable, AcuRate, PriceLookupError};
pub use cloudevent_envelope::{CloudEventEnvelope, EnvelopeBuildError};
pub use fixture_loader::FixtureLoader;
pub use import_record::{ImportRecord, import_record_to_audit_row};
```

### 3.3 `src/import_record.rs`

```rust
//! Pure conversion: Devin usage record → SpendGuard audit row.

use chrono::{DateTime, Utc};
use spendguard_common::AuditRow;

#[derive(Debug, Clone)]
pub struct ImportRecord {
    pub tenant_id:        String,
    pub budget_id:        String,
    pub devin_team_id:    String,
    pub devin_session_id: String,
    pub acu_consumed:     f64,
    pub plan:             String,                  // "team" | "enterprise"
    pub window_start:     DateTime<Utc>,
    pub window_end:       DateTime<Utc>,
    pub ingestion_mode:   IngestionMode,           // Fixture | Live
    pub fixture_provenance_sha256: Option<String>, // None when Live
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestionMode { Fixture, Live }

/// Pure conversion. No I/O, no global state. Tested by contract.
pub fn import_record_to_audit_row(
    rec: &ImportRecord,
    prices: &crate::AcuPriceTable,
) -> Result<AuditRow, crate::PriceLookupError> {
    let rate = prices.lookup(&rec.plan)?;

    let amount_micro_usd = match rate.usd_per_acu {
        Some(usd) => Some((rec.acu_consumed * usd * 1_000_000.0).round() as i64),
        None      => None,   // enterprise negotiated rate
    };

    let reason_code = if amount_micro_usd.is_none() {
        Some("devin_enterprise_negotiated_rate".to_string())
    } else {
        None
    };

    Ok(AuditRow {
        tenant_id:          rec.tenant_id.clone(),
        budget_id:          Some(rec.budget_id.clone()),
        reservation_source: "subscription_meter".into(),
        import_source:      Some("devin_team_api".into()),
        amount_micro_usd,
        pricing_version:    Some(prices.pricing_version.clone()),
        model:              format!("devin/acu/{}", rec.plan),
        reason_code,
        occurred_at:        rec.window_end,
        ..Default::default()
    })
}
```

### 3.4 `src/acu_price_table.rs`

```rust
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AcuPriceTable {
    pub pricing_version: String,
    pub effective_from:  chrono::DateTime<chrono::Utc>,
    pub currency:        String,
    pub rates:           Vec<AcuRate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcuRate {
    pub plan:        String,
    pub usd_per_acu: Option<f64>,         // None = enterprise negotiated
    #[serde(default)]
    pub note:        Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PriceLookupError {
    #[error("plan not found in price table: {0}")]
    PlanNotFound(String),
}

impl AcuPriceTable {
    pub fn load_from_embedded() -> Self {
        // include_str! the asset; parse once at process start.
        let raw = include_str!("../assets/devin_acu_prices.json");
        serde_json::from_str(raw).expect("embedded price table valid JSON")
    }

    pub fn lookup(&self, plan: &str) -> Result<&AcuRate, PriceLookupError> {
        self.rates.iter()
            .find(|r| r.plan == plan)
            .ok_or_else(|| PriceLookupError::PlanNotFound(plan.into()))
    }
}
```

### 3.5 `src/cloudevent_envelope.rs`

```rust
//! Build a CloudEvent 1.0 envelope per the
//! `spendguard.audit.import.devin_acu` schema (sibling doc
//! `cloudevent-schema.md`). Golden-tested verbatim.

use serde::Serialize;
use uuid::Uuid;

const EVENT_TYPE:  &str = "spendguard.audit.import.devin_acu";
const EVENT_SRC:   &str = "spendguard-importer-devin";
const DATA_SCHEMA: &str = "v1alpha1";

#[derive(Debug, Serialize)]
pub struct CloudEventEnvelope {
    pub specversion:     String,
    #[serde(rename = "type")]
    pub event_type:      String,
    pub source:          String,
    pub id:              String,
    pub time:            String,
    pub datacontenttype: String,
    pub subject:         String,
    pub data:            CloudEventData,
}

#[derive(Debug, Serialize)]
pub struct CloudEventData {
    pub schema_version:    String,
    pub tenant_id:         String,
    pub budget_id:         String,
    pub devin_team_id:     String,
    pub devin_session_id:  String,
    pub acu_consumed:      f64,
    pub usd_per_acu:       Option<f64>,
    pub amount_micro_usd:  Option<i64>,
    pub pricing_version:   String,
    pub window_start:      String,
    pub window_end:        String,
    pub reservation_source:String,
    pub import_source:     String,
    pub ingestion_mode:    String,
    pub fixture_provenance_sha256: Option<String>,
}

pub fn build(rec: &crate::ImportRecord,
             prices: &crate::AcuPriceTable)
    -> CloudEventEnvelope
{
    let rate = prices.lookup(&rec.plan).expect("plan validated upstream");
    let usd_per_acu = rate.usd_per_acu;
    let amount = usd_per_acu
        .map(|u| (rec.acu_consumed * u * 1_000_000.0).round() as i64);

    CloudEventEnvelope {
        specversion:     "1.0".into(),
        event_type:      EVENT_TYPE.into(),
        source:          EVENT_SRC.into(),
        id:              Uuid::now_v7().to_string(),
        time:            chrono::Utc::now().to_rfc3339(),
        datacontenttype: "application/json".into(),
        subject: format!(
            "tenant/{}/devin/team/{}/session/{}",
            rec.tenant_id, rec.devin_team_id, rec.devin_session_id),
        data: CloudEventData {
            schema_version:    DATA_SCHEMA.into(),
            tenant_id:         rec.tenant_id.clone(),
            budget_id:         rec.budget_id.clone(),
            devin_team_id:     rec.devin_team_id.clone(),
            devin_session_id:  rec.devin_session_id.clone(),
            acu_consumed:      rec.acu_consumed,
            usd_per_acu,
            amount_micro_usd:  amount,
            pricing_version:   prices.pricing_version.clone(),
            window_start:      rec.window_start.to_rfc3339(),
            window_end:        rec.window_end.to_rfc3339(),
            reservation_source:"subscription_meter".into(),
            import_source:     "devin_team_api".into(),
            ingestion_mode:    match rec.ingestion_mode {
                crate::import_record::IngestionMode::Fixture => "fixture".into(),
                crate::import_record::IngestionMode::Live    => "live".into(),
            },
            fixture_provenance_sha256: rec.fixture_provenance_sha256.clone(),
        },
    }
}
```

### 3.6 `src/fixture_loader.rs`

```rust
use std::path::Path;
use sha2::{Sha256, Digest};
use crate::import_record::{ImportRecord, IngestionMode};

pub struct FixtureLoader { /* path + cached SHA-256 */ }

impl FixtureLoader {
    pub fn new(path: &Path) -> anyhow::Result<Self> { /* read + hash */ }

    /// Returns synthetic ImportRecords parsed from the snapshot.
    /// Every record is tagged ingestion_mode = Fixture and carries
    /// the fixture's SHA-256 hash so audit rows are auditable.
    pub fn records(&self) -> anyhow::Result<Vec<ImportRecord>> { /* parse */ }
}
```

### 3.7 `src/live/client.rs` (feature `live`)

```rust
#[cfg(feature = "live")]
use reqwest::{Client, StatusCode};

pub struct DevinClient {
    base_url: url::Url,
    token:    String,
    http:     Client,   // rustls-only build
}

impl DevinClient {
    pub fn from_env() -> Result<Self, LiveError> {
        let token = std::env::var("DEVIN_API_TOKEN")
            .map_err(|_| LiveError::MissingToken)?;
        let base = std::env::var("DEVIN_API_BASE_URL")
            .unwrap_or_else(|_| "https://api.devin.ai/api/v1".into());
        // … construct rustls Client with timeout=30s
    }

    pub async fn fetch_team_usage(
        &self, team_id: &str,
        start: chrono::DateTime<chrono::Utc>,
        end:   chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<UsageRow>, LiveError> { /* GET /teams/.../usage */ }
}

#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("DEVIN_API_TOKEN environment variable not set")]
    MissingToken,
    #[error("authentication failed (401)")] Unauthorized,
    #[error("forbidden (403)")] Forbidden,
    #[error("rate limited; retry after {0}s")] RateLimited(u32),
    #[error("upstream {0}")] Upstream(StatusCode),
    #[error("transport: {0}")] Transport(#[from] reqwest::Error),
}
```

### 3.8 `assets/devin_acu_prices.json`

```json
{
  "pricing_version": "devin-acu-v1-2026-06",
  "effective_from":  "2026-06-01T00:00:00Z",
  "currency":        "USD",
  "rates": [
    { "plan": "team",       "usd_per_acu": 2.25 },
    { "plan": "enterprise", "usd_per_acu": null,
      "note": "negotiated per contract; importer emits reason_code=devin_enterprise_negotiated_rate" }
  ]
}
```

## 4. Demo

### 4.1 `deploy/demo/runtime/import_devin_fixture_demo.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
docker compose -f deploy/demo/compose.yaml up -d canonical_ingest postgres

# Run the importer in fixture mode against the canonical_ingest UDS endpoint
cargo run -p spendguard-importer-devin --bin importer_devin -- \
    --mode fixture \
    --fixture services/importer_devin/tests/fixtures/devin_usage.json \
    --ingest-endpoint http://localhost:7102/v1/AppendEvents \
    --tenant demo --budget devin-budget

psql "$DATABASE_URL" -f deploy/demo/verify_step_import_devin_fixture.sql
```

### 4.2 `deploy/demo/verify_step_import_devin_fixture.sql`

```sql
DO $$
DECLARE meter_count INT; ledger_count INT;
BEGIN
    SELECT count(*) INTO meter_count
      FROM audit_outbox
     WHERE tenant_id = 'demo'
       AND import_source = 'devin_team_api'
       AND reservation_source = 'subscription_meter';
    ASSERT meter_count >= 1,
        'expected ≥ 1 Devin import audit row';

    SELECT count(*) INTO ledger_count
      FROM ledger_entries
     WHERE tenant_id = 'demo'
       AND created_at > now() - interval '5 minutes';
    ASSERT ledger_count = 0,
        'Devin importer MUST NOT write to ledger_entries';
END $$;
```

### 4.3 Makefile

```make
demo-verify-import-devin-fixture: demo-up
	deploy/demo/runtime/import_devin_fixture_demo.sh
```

## 5. Docs page outline

`docs/site-v2/src/content/docs/integrations/devin-billing-importer.md`:

1. What this is — post-hoc Devin billing import; reconciliation only.
2. Why we cannot gate — Devin runs in Cognition's cloud VM (link to strategy memo Archetype IV).
3. Install — `spendguard install --importer devin --token $DEVIN_API_TOKEN`, configurable poll interval (default 1h).
4. Fixture mode for dry-run validation: `spendguard importer-devin --mode fixture …`.
5. What you see in the dashboard — estimated $ per session, plan, ACU consumed, `pricing_version` stamped per row.
6. Enterprise rate caveat — rows emit `amount_micro_usd = NULL` + `reason_code = devin_enterprise_negotiated_rate`; dashboard shows ACU only.
7. Sample CloudEvent (wrapped in Astro `is:raw` per project convention).
