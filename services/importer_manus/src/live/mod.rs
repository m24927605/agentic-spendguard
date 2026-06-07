//! D15 COV_73 — live HTTP client, feature-gated on `live`.
//!
//! Default build (`cargo tree -e=normal`) does NOT pull `reqwest`,
//! `hyper-tls`, `native-tls`, `openssl-sys` (review-standards T3 / T4
//! / T11).

pub mod client;
pub mod errors;
pub mod poll_loop;

pub use client::{ManusClient, UsageEnvelope, UsageRow};
pub use errors::{LiveError, TransportKind};
pub use poll_loop::{run_poll_cycle, PollConfig};
