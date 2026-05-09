//! Sidecar runtime state shared across handlers.
//!
//! Holds the cached endpoint catalog, contract bundle, schema bundle,
//! pricing snapshot, and active fencing epoch. Updated by background
//! refresh tasks; read by every decision request.

use std::collections::HashMap;
use std::sync::{atomic::AtomicU64, Arc};

use chrono::{DateTime, Utc};
use parking_lot::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    clients::{canonical_ingest::CanonicalIngestClient, ledger::LedgerClient},
    decision::idempotency::IdempotencyCache,
};

#[derive(Clone)]
pub struct SidecarState {
    pub inner: Arc<SidecarStateInner>,
}

pub struct SidecarStateInner {
    /// Last successful manifest pull. Drives critical_max_stale gate.
    pub last_manifest_verified_at: RwLock<Option<DateTime<Utc>>>,

    /// Currently-active endpoint catalog version + body.
    pub catalog: RwLock<Option<CachedCatalog>>,

    /// Active contract bundle (Contract DSL + matrix + frozen pricing).
    pub contract_bundle: RwLock<Option<CachedContractBundle>>,

    /// Active schema bundle (Trace §12).
    pub schema_bundle: RwLock<Option<CachedSchemaBundle>>,

    /// Active fencing scope id + epoch (held while sidecar is the writer).
    pub fencing: RwLock<Option<ActiveFencing>>,

    /// True after preStop drain has begun. Decision RPCs return Draining.
    pub draining: RwLock<bool>,

    /// gRPC client to ledger.
    pub ledger: LedgerClient,

    /// gRPC client to canonical ingest (for non-audit observability events).
    pub canonical_ingest: CanonicalIngestClient,

    /// Adapter-side idempotency cache (Contract §6 idempotency_key).
    pub idempotency: IdempotencyCache,

    /// Monotonic per-process producer sequence. Initialized at startup
    /// from `Ledger.ReplayAuditFromCursor` so it doesn't collide with
    /// previously-emitted audit_outbox rows after a restart.
    /// (Stage 2 §4.3 audit_outbox.producer_sequence is globally UNIQUE
    /// per (tenant, workload_instance_id, producer_sequence).)
    pub producer_sequence: Arc<AtomicU64>,

    /// Phase 2B Step 7 reservation cache (hot-path optimization).
    /// Populated at end of `run_through_reserve` Success branch with the
    /// per-reservation context the commit path needs (decision_id,
    /// fencing_epoch_at_post, original_reserved_amount, pricing tuple,
    /// budget+window+unit). Evicted on commit/release.
    ///
    /// Cache MISS is allowed: `recover_reservation_ctx` falls through to
    /// `Ledger.QueryReservationContext` for durable recovery (Codex round
    /// 1 P1.2 fix). The cache is performance, not correctness.
    pub reservation_cache: Mutex<HashMap<Uuid, ReservationCtx>>,

    /// Phase 2B Step 7.5: secondary index decision_id -> reservation_id
    /// for ConfirmPublishOutcome.APPLY_FAILED routing. PublishOutcomeRequest
    /// only carries decision_id; this map tells us which reservation to
    /// release.
    ///
    /// POC limitation (Codex round 1 Q2): restart loses this map. After
    /// restart, ConfirmPublishOutcome.APPLY_FAILED returns a typed POC
    /// limitation error to the adapter and the reservation TTL-releases
    /// naturally.
    ///
    /// LLM_CALL_POST.RUN_ABORTED (or PROVIDER_ERROR / CLIENT_TIMEOUT)
    /// path does NOT need this map — the LlmCallPostPayload carries
    /// reservation_id directly.
    pub decision_id_to_reservation: Mutex<HashMap<Uuid, Uuid>>,

    /// Reservation TTL in seconds (Codex TTL r1 P1.4). Read from
    /// SPENDGUARD_SIDECAR_RESERVATION_TTL_SECONDS at startup; default
    /// 600s. DEMO_MODE=ttl_sweep overrides to 5s.
    pub reservation_ttl_seconds: i64,
}

#[derive(Debug, Clone)]
pub struct ReservationCtx {
    pub tenant_id: String,
    pub budget_id: Uuid,
    pub window_instance_id: Uuid,
    pub unit_id: Uuid,
    pub original_reserved_amount_atomic: String,
    pub pricing_version: String,
    pub price_snapshot_hash: Vec<u8>,
    pub fx_rate_version: String,
    pub unit_conversion_version: String,
    pub fencing_scope_id: Uuid,
    pub fencing_epoch_at_post: u64,
    pub decision_id: Uuid,
    pub ttl_expires_at: DateTime<Utc>,
    /// 'reserved' | 'committed' | 'released' | 'overrun_debt'.
    /// On cache hit this is always 'reserved' (we evict on commit/release).
    /// On cache miss + ledger lookup this can be any state; sidecar
    /// short-circuits to a typed Error on non-reserved (Codex round 2 P2.4).
    pub current_state: String,
}

#[derive(Debug, Clone)]
pub struct CachedCatalog {
    pub version_id: String,
    pub fetched_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CachedContractBundle {
    pub bundle_id: Uuid,
    pub bundle_hash: Vec<u8>,
    pub signing_key_id: String,
    pub raw: Vec<u8>,
    /// Pre-resolved pricing snapshot — frozen at bundle build time
    /// (Stage 2 §9.4). Sidecar reads this at hot path; never queries
    /// Platform Pricing DB.
    pub pricing_version: String,
    pub price_snapshot_hash: Vec<u8>,
    pub fx_rate_version: String,
    pub unit_conversion_version: String,
    /// Phase 3 wedge: parsed Contract DSL ready for hot-path evaluator.
    /// Populated by `contract::parse_from_tgz` after bundle hash verifies.
    pub parsed: crate::contract::SharedContract,
}

#[derive(Debug, Clone)]
pub struct CachedSchemaBundle {
    pub bundle_id: Uuid,
    pub bundle_hash: Vec<u8>,
    pub canonical_schema_version: String,
}

#[derive(Debug, Clone)]
pub struct ActiveFencing {
    pub scope_id: Uuid,
    pub epoch: u64,
    pub acquired_at: DateTime<Utc>,
    pub renewed_at: DateTime<Utc>,
    pub ttl_expires_at: DateTime<Utc>,
}

impl SidecarState {
    pub fn new(
        ledger: LedgerClient,
        canonical_ingest: CanonicalIngestClient,
        idempotency: IdempotencyCache,
        producer_sequence_start: u64,
        reservation_ttl_seconds: i64,
    ) -> Self {
        Self {
            inner: Arc::new(SidecarStateInner {
                last_manifest_verified_at: RwLock::new(None),
                catalog: RwLock::new(None),
                contract_bundle: RwLock::new(None),
                schema_bundle: RwLock::new(None),
                fencing: RwLock::new(None),
                draining: RwLock::new(false),
                ledger,
                canonical_ingest,
                idempotency,
                producer_sequence: Arc::new(AtomicU64::new(producer_sequence_start)),
                reservation_cache: Mutex::new(HashMap::new()),
                decision_id_to_reservation: Mutex::new(HashMap::new()),
                reservation_ttl_seconds,
            }),
        }
    }

    pub fn next_producer_sequence(&self) -> u64 {
        self.inner
            .producer_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Allocate `n` contiguous producer_sequence values atomically.
    /// Returns `(start, end_inclusive)`. Used by Step 9 InvoiceReconcile
    /// where the SP requires a decision row at seq N and an outcome row
    /// at seq N+1 (both in the same workload-instance space).
    /// Panics on `n == 0` (Codex challenge P3.2 — defensive).
    pub fn next_producer_sequence_block(&self, n: u64) -> (u64, u64) {
        assert!(n > 0, "next_producer_sequence_block requires n > 0");
        let start = self
            .inner
            .producer_sequence
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        (start, start + n - 1)
    }

    pub fn is_draining(&self) -> bool {
        *self.inner.draining.read()
    }

    pub fn mark_draining(&self) {
        *self.inner.draining.write() = true;
    }

    pub fn manifest_age_seconds(&self) -> Option<i64> {
        self.inner
            .last_manifest_verified_at
            .read()
            .map(|t| (Utc::now() - t).num_seconds())
    }
}
