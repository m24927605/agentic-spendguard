//! Concrete `CostRule` implementations.
//!
//! v0.1 ships 3 of 4 planned rules:
//!   * `idle_reservation_rate_v1` — budget-scoped (CA-P3.1);
//!     emits a 2-op identity-pinned RFC-6902 patch.
//!   * `failed_retry_burn_v1` (CA-P1.5) — run-scope; no patch yet.
//!   * `runaway_loop_v1` (CA-P1.5) — run-scope; no patch yet.
//!
//! `tool_call_repeated_v1` is deferred to v0.2 (requires SDK
//! extension to carry `tool_name` + `tool_args_hash`).

pub mod failed_retry_burn;
pub mod idle_reservation_rate;
pub mod runaway_loop;
