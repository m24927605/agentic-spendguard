//! D16 COV_87 — live HTTP client, feature-gated on `live`.
//!
//! Default build (`cargo tree -e=normal`) does NOT pull `reqwest`,
//! `hyper-tls`, `native-tls`, `openssl-sys` (acceptance A2.4 +
//! review-standards T3 / T4).

pub mod client;
pub mod errors;
pub mod poll_loop;

pub use client::{GensparkClient, UsageRow};
pub use errors::LiveError;
pub use poll_loop::{run_poll_cycle, PollConfig};
