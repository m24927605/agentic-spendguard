//! Phase A stub. Real implementation in Phase B (RunState cache + LRU eviction).
//!
//! Spec ref `run-cost-projector-spec-v1alpha1.md` §7.

#![allow(dead_code)]

use uuid::Uuid;

#[derive(Debug, Default, Clone)]
pub struct RunState {
    pub tenant_id: Uuid,
    pub run_id: Uuid,
    pub agent_id: String,
    pub model: String,
    pub steps_completed: i64,
    pub cumulative_cost_atomic: i64,
    pub per_step_costs: Vec<i64>,
    pub last_predicted_remaining_cost: Option<i64>,
    pub drift_consecutive_count: u32,
    pub signal3_hint_planned_steps: Option<i32>,
}
