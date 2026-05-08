//! Producer sequence allocator.
//!
//! Receiver maintains a monotonic atomic counter per workload_instance_id.
//! On startup, recovers max producer_sequence from audit_outbox_global_keys
//! to avoid collision with prior runs (Codex r1 P1.3 / r2 V2.1 fix).

use sqlx::{PgPool, Row};
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

/// Look up the highest producer_sequence ever inserted for this
/// (tenant_id, workload_instance_id) in audit_outbox_global_keys.
/// Returns 0 if no rows exist (fresh DB).
pub async fn recover_max_seq(
    pool: &PgPool,
    tenant_id: Uuid,
    workload_instance_id: &str,
) -> Result<u64, sqlx::Error> {
    let row = sqlx::query(
        "SELECT COALESCE(MAX(producer_sequence), 0)::BIGINT AS max_seq \
           FROM audit_outbox_global_keys \
          WHERE tenant_id = $1 \
            AND workload_instance_id = $2",
    )
    .bind(tenant_id)
    .bind(workload_instance_id)
    .fetch_one(pool)
    .await?;

    let max_seq: i64 = row.get("max_seq");
    if max_seq < 0 {
        Ok(0)
    } else {
        Ok(max_seq as u64)
    }
}

/// Atomic allocator. Initialized to recover_max_seq + 1 on startup.
pub struct SequenceAllocator {
    counter: AtomicU64,
}

impl SequenceAllocator {
    pub fn new(start: u64) -> Self {
        Self {
            counter: AtomicU64::new(start),
        }
    }

    /// Allocate one sequence value. Returns the allocated value.
    pub fn next_one(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Allocate `n` contiguous sequence values atomically.
    /// Returns `(start, end_inclusive)`.
    /// Panics on `n == 0` (defensive).
    pub fn next_block(&self, n: u64) -> (u64, u64) {
        assert!(n > 0, "next_block requires n > 0");
        let start = self.counter.fetch_add(n, Ordering::Relaxed);
        (start, start + n - 1)
    }
}
