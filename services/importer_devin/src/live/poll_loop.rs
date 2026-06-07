//! D14 COV_71 — bounded exponential-backoff poll loop. Review-standards
//! T11: max backoff cap ≤ 1 hour to prevent unbounded growth that could
//! DoS the Devin Team API.

use std::time::Duration;

/// Configuration for the poll cycle.
#[derive(Debug, Clone)]
pub struct PollConfig {
    /// Steady-state interval between pulls. Default 1 hour.
    pub interval: Duration,
    /// Lower bound for the exponential backoff when an error occurs.
    pub backoff_min: Duration,
    /// Upper bound for the backoff. Review-standards T11: ≤ 1 hour.
    pub backoff_max: Duration,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(3_600),
            backoff_min: Duration::from_secs(5),
            backoff_max: Duration::from_secs(3_600),
        }
    }
}

impl PollConfig {
    /// Compute the next backoff given the previous backoff. Caps at
    /// `backoff_max` per T11. Doubles each call.
    pub fn next_backoff(&self, prev: Duration) -> Duration {
        let next = prev.saturating_mul(2);
        let next = next.max(self.backoff_min);
        next.min(self.backoff_max)
    }
}

/// One synchronous cycle of the poll loop. Returns the rows on
/// success, or `LiveError` on failure. Caller is responsible for
/// looping + applying `next_backoff`.
///
/// We keep this thin on purpose — the actual scheduling lives in the
/// binary so testing the backoff math here doesn't require a tokio
/// runtime spinning real timers.
pub async fn run_poll_cycle(
    client: &super::client::DevinClient,
    team_id: &str,
    window: (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>),
) -> Result<Vec<super::client::UsageRow>, super::errors::LiveError> {
    client.fetch_team_usage(team_id, window.0, window.1).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_backoff_caps_at_backoff_max() {
        let cfg = PollConfig::default();
        // Doubling from a tiny start: 5s → 10s → 20s → 40s → ... and
        // eventually ≥ 3600s, should clamp at 3600s.
        let mut d = cfg.backoff_min;
        for _ in 0..30 {
            d = cfg.next_backoff(d);
            assert!(d <= cfg.backoff_max, "backoff escaped cap: {d:?}",);
        }
        assert_eq!(d, cfg.backoff_max);
    }

    #[test]
    fn next_backoff_respects_min() {
        let cfg = PollConfig::default();
        let zero = Duration::from_secs(0);
        let next = cfg.next_backoff(zero);
        assert!(next >= cfg.backoff_min);
    }

    #[test]
    fn next_backoff_doubles_until_capped() {
        let cfg = PollConfig {
            interval: Duration::from_secs(10),
            backoff_min: Duration::from_secs(2),
            backoff_max: Duration::from_secs(60),
        };
        assert_eq!(
            cfg.next_backoff(Duration::from_secs(2)),
            Duration::from_secs(4)
        );
        assert_eq!(
            cfg.next_backoff(Duration::from_secs(4)),
            Duration::from_secs(8)
        );
        assert_eq!(
            cfg.next_backoff(Duration::from_secs(32)),
            Duration::from_secs(60)
        );
        assert_eq!(
            cfg.next_backoff(Duration::from_secs(60)),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn t11_backoff_cap_is_at_most_one_hour() {
        let cfg = PollConfig::default();
        assert!(cfg.backoff_max <= Duration::from_secs(3_600));
    }
}
