//! Project RPC p99 benchmark.
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §12.1 — warm cache hit
//! p99 ≤ 5ms.
//!
//! The bench drives the in-process `RunCostProjectorSvc::project` handler
//! (no gRPC round-trip; the gRPC layer adds < 1ms in tonic; this isolates
//! the layering + cache + signal computation cost which dominates the budget).
//!
//! We run two variants:
//!   1. `project_warm_cache_hit` — single run id repeated; exercises the
//!      hot path (cache hit, no recovery).
//!   2. `project_cold_start_skeleton` — fresh run id every iteration; pool
//!      = None so Signal 1 cold-starts and recovery is skipped.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use spendguard_run_cost_projector::{
    config::Config,
    proto::run_cost_projector::v1::{run_cost_projector_server::RunCostProjector, ProjectRequest},
    server::RunCostProjectorSvc,
};
use tonic::Request;
use uuid::Uuid;

fn test_cfg() -> Config {
    Config {
        listen_addr: "127.0.0.1:0".into(),
        uds_path: None,
        tls_cert_pem: None,
        tls_key_pem: None,
        tls_ca_pem: None,
        metrics_addr: "".into(),
        region: "bench".into(),
        profile: "demo".into(),
        database_url: "".into(),
        state_cache_ttl_seconds: 1800,
        state_cache_capacity: 10_000,
        replay_window_minutes: 30,
        cold_start_run_length: 10,
        drift_consecutive_threshold: 3,
        drift_ratio_threshold: 0.5,
    }
}

fn build_req(tenant: Uuid, run: Uuid) -> ProjectRequest {
    ProjectRequest {
        tenant_id: tenant.to_string(),
        run_id: run.to_string(),
        agent_id: "ag-bench".into(),
        model: "gpt-4o".into(),
        step_id: String::new(),
        decision_id: "dec-bench".into(),
        this_call_reservation_atomic: 100,
        unit_id: "USD".into(),
        budget_remaining_atomic: 1_000_000_000,
        planned_steps_hint: 0,
        planned_tools_hint: 0,
    }
}

fn project_warm_cache_hit(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let svc = RunCostProjectorSvc::new(test_cfg(), None);
    let tenant = Uuid::new_v4();
    let run = Uuid::new_v4();
    // Warm the cache with one call.
    rt.block_on(async {
        let _ = svc
            .project(Request::new(build_req(tenant, run)))
            .await
            .expect("warm-up");
    });

    c.bench_function("project_warm_cache_hit", |b| {
        b.iter(|| {
            rt.block_on(async {
                let resp = svc
                    .project(Request::new(build_req(tenant, run)))
                    .await
                    .expect("project");
                black_box(resp);
            });
        });
    });
}

fn project_cold_start_skeleton(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let svc = RunCostProjectorSvc::new(test_cfg(), None);
    let tenant = Uuid::new_v4();

    c.bench_function("project_cold_start_skeleton", |b| {
        b.iter(|| {
            // Fresh run id every iteration so the cache always misses.
            let run = Uuid::new_v4();
            rt.block_on(async {
                let resp = svc
                    .project(Request::new(build_req(tenant, run)))
                    .await
                    .expect("project");
                black_box(resp);
            });
        });
    });
}

criterion_group!(benches, project_warm_cache_hit, project_cold_start_skeleton);
criterion_main!(benches);
