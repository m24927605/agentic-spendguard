//! Lock-order token derivation (per Stage 2 §8.2.1.1).
//!
//! ```text
//! sorted = sort_lex_by(claims, key=(budget_id, unit_id))
//! canon  = ",".join(f"{c.budget_id}:{c.unit_id}" for c in sorted)
//! token  = "v1:" + hex(sha256(canon))
//! ```
//!
//! Caller may either supply a pre-derived token (server validates exact
//! match) or omit it (server derives + echoes back in response).

use sha2::{Digest, Sha256};

use crate::proto::common::v1::BudgetClaim;

const VERSION_PREFIX: &str = "v1:";

/// Per-claim entry pair: (budget_id, unit_id, account_kind).
///
/// A `BudgetClaim` produces TWO ledger_entries — one debiting `available_budget`
/// and one crediting `reserved_hold`. The lock_order_token is derived over the
/// distinct (budget, unit, kind) triples actually touched, in canonical
/// lex order, matching what the Postgres `post_ledger_transaction` function
/// computes from the resolved entries.
pub fn derive(claims: &[BudgetClaim]) -> String {
    let mut keys: Vec<(String, String, &'static str)> = Vec::with_capacity(claims.len() * 2);
    for c in claims {
        let unit_id = c
            .unit
            .as_ref()
            .map(|u| u.unit_id.clone())
            .unwrap_or_default();
        keys.push((c.budget_id.clone(), unit_id.clone(), "available_budget"));
        keys.push((c.budget_id.clone(), unit_id, "reserved_hold"));
    }
    keys.sort();
    keys.dedup();

    let canonical = keys
        .iter()
        .map(|(b, u, k)| format!("{}:{}:{}", b, u, k))
        .collect::<Vec<_>>()
        .join(",");

    let digest = Sha256::digest(canonical.as_bytes());
    format!("{}{}", VERSION_PREFIX, hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::common::v1::{
        budget_claim::Direction, BudgetClaim, UnitRef, unit_ref::Kind,
    };

    fn claim(budget_id: &str, unit_id: &str) -> BudgetClaim {
        BudgetClaim {
            budget_id: budget_id.into(),
            unit: Some(UnitRef {
                unit_id: unit_id.into(),
                kind: Kind::Monetary as i32,
                currency: "USD".into(),
                ..Default::default()
            }),
            amount_atomic: "1000".into(),
            direction: Direction::Debit as i32,
            window_instance_id: "w".into(),
        }
    }

    #[test]
    fn order_independent() {
        let a = derive(&[claim("budget_b", "u_b"), claim("budget_a", "u_a")]);
        let b = derive(&[claim("budget_a", "u_a"), claim("budget_b", "u_b")]);
        assert_eq!(a, b);
    }

    #[test]
    fn version_prefix() {
        let token = derive(&[claim("b", "u")]);
        assert!(token.starts_with(VERSION_PREFIX));
        assert_eq!(token.len(), VERSION_PREFIX.len() + 64);
    }

    #[test]
    fn distinct_inputs_distinct_tokens() {
        let a = derive(&[claim("budget_a", "u")]);
        let b = derive(&[claim("budget_b", "u")]);
        assert_ne!(a, b);
    }
}
