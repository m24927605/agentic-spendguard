//! Round-2 #11: Prometheus metrics for webhook receiver.
//!
//! axum middleware mirrors control_plane / dashboard: extract the
//! matched route + HTTP method, run the handler, then increment
//! ok/err based on response status.
//!
//! Surfaced counters:
//!
//!   * `spendguard_webhook_receiver_route_calls_total{route, outcome}`

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
    Webhook,
    Healthz,
    Other,
}

impl Route {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Healthz => "healthz",
            Self::Other => "other",
        }
    }

    pub fn from_matched(method: &Method, path: &str) -> Self {
        match (method, path) {
            (&Method::POST, "/v1/webhook/:provider") => Self::Webhook,
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

const ROUTE_COUNT: usize = 3;

#[derive(Default)]
pub struct WebhookReceiverMetricsInner {
    counts: [[AtomicU64; 2]; ROUTE_COUNT],
}

#[derive(Clone, Default)]
pub struct WebhookReceiverMetrics {
    pub inner: Arc<WebhookReceiverMetricsInner>,
}

impl WebhookReceiverMetrics {
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
        let mut out = String::with_capacity(512);
        out.push_str("# HELP spendguard_webhook_receiver_route_calls_total Webhook receiver route invocations broken out by outcome.\n");
        out.push_str("# TYPE spendguard_webhook_receiver_route_calls_total counter\n");
        for r in ALL_ROUTES {
            for outcome in [Outcome::Ok, Outcome::Err] {
                let r_idx = route_index(*r);
                let o_idx = match outcome {
                    Outcome::Ok => 0,
                    Outcome::Err => 1,
                };
                let v = self.inner.counts[r_idx][o_idx].load(Ordering::Relaxed);
                out.push_str(&format!(
                    "spendguard_webhook_receiver_route_calls_total{{route=\"{}\",outcome=\"{}\"}} {}\n",
                    r.as_str(),
                    outcome.as_str(),
                    v,
                ));
            }
        }
        out
    }
}

const ALL_ROUTES: &[Route] = &[Route::Webhook, Route::Healthz, Route::Other];

fn route_index(r: Route) -> usize {
    match r {
        Route::Webhook => 0,
        Route::Healthz => 1,
        Route::Other => 2,
    }
}

pub async fn record_metrics(
    State(metrics): State<WebhookReceiverMetrics>,
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
    fn counters_default_to_zero() {
        let m = WebhookReceiverMetrics::new();
        let txt = m.render();
        assert!(txt.contains("route=\"webhook\",outcome=\"ok\"} 0"));
        assert!(txt.contains("route=\"healthz\",outcome=\"err\"} 0"));
    }

    #[test]
    fn increments_show() {
        let m = WebhookReceiverMetrics::new();
        m.inc_route(Route::Webhook, Outcome::Ok);
        m.inc_route(Route::Webhook, Outcome::Ok);
        m.inc_route(Route::Webhook, Outcome::Err);
        let txt = m.render();
        assert!(txt.contains("route=\"webhook\",outcome=\"ok\"} 2"));
        assert!(txt.contains("route=\"webhook\",outcome=\"err\"} 1"));
    }

    #[test]
    fn render_help_and_type() {
        let m = WebhookReceiverMetrics::new();
        let txt = m.render();
        assert!(txt.contains("# HELP spendguard_webhook_receiver_route_calls_total"));
        assert!(txt.contains("# TYPE spendguard_webhook_receiver_route_calls_total counter"));
    }

    #[test]
    fn route_from_matched() {
        assert_eq!(
            Route::from_matched(&Method::POST, "/v1/webhook/:provider"),
            Route::Webhook
        );
        assert_eq!(Route::from_matched(&Method::GET, "/healthz"), Route::Healthz);
        assert_eq!(Route::from_matched(&Method::DELETE, "/x"), Route::Other);
    }

    #[test]
    fn shared_state_thread_safe() {
        let m = WebhookReceiverMetrics::new();
        let m2 = m.clone();
        m.inc_route(Route::Healthz, Outcome::Ok);
        m2.inc_route(Route::Healthz, Outcome::Ok);
        let txt = m.render();
        assert!(txt.contains("route=\"healthz\",outcome=\"ok\"} 2"));
    }
}
