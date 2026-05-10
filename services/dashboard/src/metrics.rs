//! Round-2 #11: Prometheus metrics for operator dashboard.
//!
//! Same axum middleware shape as control_plane — no `prometheus`
//! crate, raw `AtomicU64` + manual text-format render. Counter
//! increments live in `record_metrics` (axum middleware) layered
//! over the outer Router so every route + healthz is captured.
//!
//! Surfaced counters:
//!
//!   * `spendguard_dashboard_route_calls_total{route, outcome}` —
//!     dashboard route invocation count, broken out by 2xx/3xx (ok)
//!     vs 4xx/5xx (err).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{
    extract::{MatchedPath, State},
    http::Method,
    middleware::Next,
    response::Response,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Index,
    Healthz,
    ApiBudgets,
    ApiDecisions,
    ApiDenyStats,
    ApiOutboxHealth,
    ApiAuditExport,
    Other,
}

impl Route {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Healthz => "healthz",
            Self::ApiBudgets => "api_budgets",
            Self::ApiDecisions => "api_decisions",
            Self::ApiDenyStats => "api_deny_stats",
            Self::ApiOutboxHealth => "api_outbox_health",
            Self::ApiAuditExport => "api_audit_export",
            Self::Other => "other",
        }
    }

    pub fn from_matched(method: &Method, path: &str) -> Self {
        match (method, path) {
            (&Method::GET, "/") => Self::Index,
            (&Method::GET, "/healthz") => Self::Healthz,
            (&Method::GET, "/api/budgets") => Self::ApiBudgets,
            (&Method::GET, "/api/decisions") => Self::ApiDecisions,
            (&Method::GET, "/api/deny-stats") => Self::ApiDenyStats,
            (&Method::GET, "/api/outbox-health") => Self::ApiOutboxHealth,
            (&Method::GET, "/api/audit/export") => Self::ApiAuditExport,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Err,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Err => "err",
        }
    }
}

const ROUTE_COUNT: usize = 8;

#[derive(Default)]
pub struct DashboardMetricsInner {
    counts: [[AtomicU64; 2]; ROUTE_COUNT],
}

#[derive(Clone, Default)]
pub struct DashboardMetrics {
    pub inner: Arc<DashboardMetricsInner>,
}

impl DashboardMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_route(&self, route: Route, outcome: Outcome) {
        let r = route_index(route);
        let o = match outcome {
            Outcome::Ok => 0,
            Outcome::Err => 1,
        };
        self.inner.counts[r][o].fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("# HELP spendguard_dashboard_route_calls_total Operator dashboard route invocations broken out by outcome.\n");
        out.push_str("# TYPE spendguard_dashboard_route_calls_total counter\n");
        for r in ALL_ROUTES {
            for outcome in [Outcome::Ok, Outcome::Err] {
                let r_idx = route_index(*r);
                let o_idx = match outcome {
                    Outcome::Ok => 0,
                    Outcome::Err => 1,
                };
                let v = self.inner.counts[r_idx][o_idx].load(Ordering::Relaxed);
                out.push_str(&format!(
                    "spendguard_dashboard_route_calls_total{{route=\"{}\",outcome=\"{}\"}} {}\n",
                    r.as_str(),
                    outcome.as_str(),
                    v,
                ));
            }
        }
        out
    }
}

const ALL_ROUTES: &[Route] = &[
    Route::Index,
    Route::Healthz,
    Route::ApiBudgets,
    Route::ApiDecisions,
    Route::ApiDenyStats,
    Route::ApiOutboxHealth,
    Route::ApiAuditExport,
    Route::Other,
];

fn route_index(r: Route) -> usize {
    match r {
        Route::Index => 0,
        Route::Healthz => 1,
        Route::ApiBudgets => 2,
        Route::ApiDecisions => 3,
        Route::ApiDenyStats => 4,
        Route::ApiOutboxHealth => 5,
        Route::ApiAuditExport => 6,
        Route::Other => 7,
    }
}

pub async fn record_metrics(
    State(metrics): State<DashboardMetrics>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let matched = request
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let route = Route::from_matched(&method, &matched);
    let response = next.run(request).await;
    let outcome = if response.status().is_server_error()
        || response.status().is_client_error()
    {
        Outcome::Err
    } else {
        Outcome::Ok
    };
    metrics.inc_route(route, outcome);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_default_to_zero_in_render_output() {
        let m = DashboardMetrics::new();
        let txt = m.render();
        assert!(txt.contains("spendguard_dashboard_route_calls_total{route=\"api_budgets\",outcome=\"ok\"} 0"));
        assert!(txt.contains("spendguard_dashboard_route_calls_total{route=\"api_audit_export\",outcome=\"err\"} 0"));
    }

    #[test]
    fn increments_show_in_render_output() {
        let m = DashboardMetrics::new();
        m.inc_route(Route::ApiBudgets, Outcome::Ok);
        m.inc_route(Route::ApiBudgets, Outcome::Ok);
        m.inc_route(Route::ApiAuditExport, Outcome::Err);
        let txt = m.render();
        assert!(txt.contains("route=\"api_budgets\",outcome=\"ok\"} 2"));
        assert!(txt.contains("route=\"api_audit_export\",outcome=\"err\"} 1"));
    }

    #[test]
    fn render_includes_help_and_type_lines() {
        let m = DashboardMetrics::new();
        let txt = m.render();
        assert!(txt.contains("# HELP spendguard_dashboard_route_calls_total"));
        assert!(txt.contains("# TYPE spendguard_dashboard_route_calls_total counter"));
    }

    #[test]
    fn shared_state_is_thread_safe() {
        let m = DashboardMetrics::new();
        let m2 = m.clone();
        m.inc_route(Route::Healthz, Outcome::Ok);
        m2.inc_route(Route::Healthz, Outcome::Ok);
        let txt = m.render();
        assert!(txt.contains("route=\"healthz\",outcome=\"ok\"} 2"));
    }

    #[test]
    fn route_from_matched_maps_known_pairs() {
        assert_eq!(Route::from_matched(&Method::GET, "/"), Route::Index);
        assert_eq!(Route::from_matched(&Method::GET, "/api/budgets"), Route::ApiBudgets);
        assert_eq!(Route::from_matched(&Method::GET, "/api/audit/export"), Route::ApiAuditExport);
        assert_eq!(Route::from_matched(&Method::POST, "/anywhere"), Route::Other);
    }
}
