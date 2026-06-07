pub mod audit;
pub mod bootstrap;
pub mod clients;
pub mod config;
pub mod contract;
pub mod decision;
pub mod domain;
pub mod drain;
pub mod fencing;
pub mod http_companion;
pub mod metrics;
pub mod prompt_hash;
pub mod server;
// D13 — subscription-tier meter (Claude Code Pro / Codex on ChatGPT
// Plus). Pure-Rust module; no runtime deps beyond chrono + serde_json
// for the synthetic 429 body. Sidecar branches on
// DecisionRequest.reservation_source = SUBSCRIPTION_METER at the top
// of `decision::transaction::run_through_reserve` to skip ledger
// writes (see subscription_meter::route_decision_request).
pub mod subscription_meter;

pub mod proto {
    pub mod common {
        pub mod v1 {
            tonic::include_proto!("spendguard.common.v1");
        }
    }
    pub mod sidecar_adapter {
        pub mod v1 {
            tonic::include_proto!("spendguard.sidecar_adapter.v1");
        }
    }
    pub mod ledger {
        pub mod v1 {
            tonic::include_proto!("spendguard.ledger.v1");
        }
    }
    pub mod canonical_ingest {
        pub mod v1 {
            tonic::include_proto!("spendguard.canonical_ingest.v1");
        }
    }
    // SLICE_09 Phase E: run_cost_projector client stub. Spec
    // run-cost-projector-spec-v1alpha1.md §2.1.
    pub mod run_cost_projector {
        pub mod v1 {
            tonic::include_proto!("spendguard.run_cost_projector.v1");
        }
    }
}
