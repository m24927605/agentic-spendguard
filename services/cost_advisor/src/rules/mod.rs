//! Concrete `CostRule` implementations.
//!
//! P0 ships one placeholder rule so the trait surface, the proto
//! contract, and the `SqlCostRule` adapter can be compiled and
//! type-checked together. The real v0.1 rule
//! (`idle_reservation_rate_v1`, the only one fireable under current
//! schema per cost-advisor-p0-audit-report §4) lands in P1 alongside
//! the runtime that executes the SQL.

pub mod idle_reservation_rate;
