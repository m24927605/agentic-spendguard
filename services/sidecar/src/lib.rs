pub mod audit;
pub mod bootstrap;
pub mod clients;
pub mod config;
pub mod contract;
pub mod decision;
pub mod domain;
pub mod drain;
pub mod fencing;
pub mod metrics;
pub mod prompt_hash;
pub mod server;

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
}
