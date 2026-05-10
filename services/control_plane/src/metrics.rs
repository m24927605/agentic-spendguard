//! Round-2 #11: Prometheus metrics for control plane (REST API).
//!
//! Mirrors the ledger / sidecar pattern — no `prometheus` crate, raw
//! `AtomicU64` + manual text-format render. Increments live in an
//! axum middleware layer (`record_metrics`) that maps the matched
//! route + HTTP method to a handler enum and records ok/err based on
//! the response status code.
//!
//! Surfaced counters:
//!
//!   * `spendguard_control_plane_route_calls_total{route, outcome}`
//!     — REST endpoint invocation count, broken out by 2xx/3xx (ok)
//!     vs 4xx/5xx (err).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{
    extract::{MatchedPath, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    CreateTenant,
    GetTenant,
    TombstoneTenant,
    CreateBudget,
    ListApprovals,
    GetApproval,
    ResolveApproval,
    Healthz,
    /// Catch-all for routes not yet enumerated; surfaces churn before
    /// we forget to bump the enum.
    Other,
}

impl Route {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateTenant => "create_tenant",
            Self::GetTenant => "get_tenant",
            Self::TombstoneTenant => "tombstone_tenant",
            Self::CreateBudget => "create_budget",
            Self::ListApprovals => "list_approvals",
            Self::GetApproval => "get_approval",
            Self::ResolveApproval => "resolve_approval",
            Self::Healthz => "healthz",
            Self::Other => "other",
        }
    }

    /// Map an axum `MatchedPath` + HTTP method to a `Route` enum
    /// variant. Unknown combinations fall back to `Other`.
    pub fn from_matched(method: &Method, path: &str) -> Self {
        match (method, path) {
            (&Method::POST, "/v1/tenants") => Self::CreateTenant,
            (&Method::GET, "/v1/tenants/:id") => Self::GetTenant,
            (&Method::DELETE, "/v1/tenants/:id") => Self::TombstoneTenant,
            (&Method::POST, "/v1/tenants/:id/budgets") => Self::CreateBudget,
            (&Method::GET, "/v1/approvals") => Self::ListApprovals,
            (&Method::GET, "/v1/approvals/:id") => Self::GetApproval,
            (&Method::POST, "/v1/approvals/:id/resolve") => Self::ResolveApproval,
            (&Method::GET, "/healthz") => Self::Healthz,
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

const ROUTE_COUNT: usize = 9;

#[derive(Default)]
pub struct ControlPlaneMetricsInner {
    counts: [[AtomicU64; 2]; ROUTE_COUNT],
}

#[derive(Clone, Default)]
pub struct ControlPlaneMetrics {
    pub inner: Arc<ControlPlaneMetricsInner>,
}

impl ControlPlaneMetrics {
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

    /// Render the Prometheus text-format payload.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str("# HELP spendguard_control_plane_route_calls_total Control plane REST endpoint invocations broken out by outcome.\n");
        out.push_str("# TYPE spendguard_control_plane_route_calls_total counter\n");
        for r in ALL_ROUTES {
            for outcome in [Outcome::Ok, Outcome::Err] {
                let r_idx = route_index(*r);
                let o_idx = match outcome {
                    Outcome::Ok => 0,
                    Outcome::Err => 1,
                };
                let v = self.inner.counts[r_idx][o_idx].load(Ordering::Relaxed);
                out.push_str(&format!(
                    "spendguard_control_plane_route_calls_total{{route=\"{}\",outcome=\"{}\"}} {}\n",
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
    Route::CreateTenant,
    Route::GetTenant,
    Route::TombstoneTenant,
    Route::CreateBudget,
    Route::ListApprovals,
    Route::GetApproval,
    Route::ResolveApproval,
    Route::Healthz,
    Route::Other,
];

fn route_index(r: Route) -> usize {
    match r {
        Route::CreateTenant => 0,
        Route::GetTenant => 1,
        Route::TombstoneTenant => 2,
        Route::CreateBudget => 3,
        Route::ListApprovals => 4,
        Route::GetApproval => 5,
        Route::ResolveApproval => 6,
        Route::Healthz => 7,
        Route::Other => 8,
    }
}

/// Axum middleware: extract the matched route + method, run the
/// downstream handler, then increment ok/err based on response status.
pub async fn record_metrics(
    State(metrics): State<ControlPlaneMetrics>,
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
    let _ = StatusCode::OK; // keep StatusCode import alive for clarity.
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_default_to_zero_in_render_output() {
        let m = ControlPlaneMetrics::new();
        let txt = m.render();
        assert!(txt.contains("spendguard_control_plane_route_calls_total{route=\"create_tenant\",outcome=\"ok\"} 0"));
        assert!(txt.contains("spendguard_control_plane_route_calls_total{route=\"resolve_approval\",outcome=\"err\"} 0"));
    }

    #[test]
    fn increments_show_in_render_output() {
        let m = ControlPlaneMetrics::new();
        m.inc_route(Route::CreateTenant, Outcome::Ok);
        m.inc_route(Route::CreateTenant, Outcome::Ok);
        m.inc_route(Route::ResolveApproval, Outcome::Err);
        let txt = m.render();
        assert!(txt.contains("route=\"create_tenant\",outcome=\"ok\"} 2"));
        assert!(txt.contains("route=\"resolve_approval\",outcome=\"err\"} 1"));
    }

    #[test]
    fn render_includes_help_and_type_lines() {
        let m = ControlPlaneMetrics::new();
        let txt = m.render();
        assert!(txt.contains("# HELP spendguard_control_plane_route_calls_total"));
        assert!(txt.contains("# TYPE spendguard_control_plane_route_calls_total counter"));
    }

    #[test]
    fn shared_state_is_thread_safe() {
        let m = ControlPlaneMetrics::new();
        let m2 = m.clone();
        m.inc_route(Route::Healthz, Outcome::Ok);
        m2.inc_route(Route::Healthz, Outcome::Ok);
        let txt = m.render();
        assert!(txt.contains("route=\"healthz\",outcome=\"ok\"} 2"));
    }

    #[test]
    fn route_from_matched_maps_known_pairs() {
        assert_eq!(
            Route::from_matched(&Method::POST, "/v1/tenants"),
            Route::CreateTenant
        );
        assert_eq!(
            Route::from_matched(&Method::GET, "/v1/tenants/:id"),
            Route::GetTenant
        );
        assert_eq!(
            Route::from_matched(&Method::DELETE, "/v1/tenants/:id"),
            Route::TombstoneTenant
        );
        assert_eq!(
            Route::from_matched(&Method::POST, "/v1/approvals/:id/resolve"),
            Route::ResolveApproval
        );
        assert_eq!(
            Route::from_matched(&Method::GET, "/healthz"),
            Route::Healthz
        );
        assert_eq!(
            Route::from_matched(&Method::PUT, "/somewhere/else"),
            Route::Other
        );
    }
}
