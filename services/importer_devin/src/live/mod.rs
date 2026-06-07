//! D14 COV_71 — live HTTP client, feature-gated on `live`.
//!
//! Default build (`cargo tree -e=normal`) does NOT pull `reqwest`,
//! `hyper-tls`, `native-tls`, `openssl-sys` (acceptance A2.4 +
//! review-standards T2 / T4).

pub mod client;
pub mod errors;
pub mod poll_loop;

pub use client::{DevinClient, UsageRow};
pub use errors::LiveError;
pub use poll_loop::{run_poll_cycle, PollConfig};
