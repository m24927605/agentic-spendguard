pub mod config;
pub mod domain;
pub mod handlers;
pub mod persistence;
pub mod server;

pub mod proto {
    pub mod common {
        pub mod v1 {
            tonic::include_proto!("spendguard.common.v1");
        }
    }
    pub mod canonical_ingest {
        pub mod v1 {
            tonic::include_proto!("spendguard.canonical_ingest.v1");
        }
    }
}
