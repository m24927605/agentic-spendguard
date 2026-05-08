//! Producer sequence allocator with cold-path startup recovery.
//! Mirrors services/webhook_receiver/src/persistence/sequence.rs.

use sqlx::{PgPool, Row};
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

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
    Ok(max_seq.max(0) as u64)
}

pub struct SequenceAllocator {
    counter: AtomicU64,
}

impl SequenceAllocator {
    pub fn new(start: u64) -> Self {
        Self {
            counter: AtomicU64::new(start),
        }
    }
    pub fn next_one(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }
}
