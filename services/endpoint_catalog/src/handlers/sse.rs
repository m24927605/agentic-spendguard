//! Server-Sent Events invalidation channel (Stage 2 §8.2.4 + Codex round-1 D7).
//!
//! GET /v1/catalog/events
//!
//! SSE is BEST-EFFORT — it is an invalidation hint, NOT the correctness path.
//! Sidecars MUST still poll the manifest endpoint within
//! `manifest_validity_seconds` regardless of SSE state.
//!
//! When the publisher (the catalog-publish CLI) writes a new manifest,
//! it POSTs `/v1/internal/notify-catalog-change` with a Bearer token; the
//! handler in `server.rs` then broadcasts via `invalidation_tx` to all
//! connected SSE clients.

use std::time::Duration;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;

use crate::server::AppState;

pub async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    let rx = state.invalidation_tx.subscribe();
    let stream = BroadcastStream::new(rx).map(|res| match res {
        Ok(payload) => Ok(Event::default()
            .event("catalog_invalidate")
            .data(payload)),
        Err(_) => {
            // Lagged or closed — emit a hint event so client refreshes
            // explicitly.
            Ok(Event::default()
                .event("catalog_invalidate")
                .data("{\"reason\":\"stream_lagged\"}"))
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(state.cfg.sse_keepalive_seconds))
            .text("keep-alive"),
    )
}
