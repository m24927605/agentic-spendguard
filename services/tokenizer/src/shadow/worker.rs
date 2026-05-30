//! Phase A placeholder — full implementation in Phase E.
//!
//! Async shadow loop spawned at boot via `tokio::spawn` from
//! `services/tokenizer/src/main.rs`. Listens on a `mpsc::Receiver` fed
//! by the gRPC `Tokenize` handler after Tier 2 returns to caller (the
//! send is best-effort + non-blocking so Tier 2 latency is unaffected).
//!
//! Per spec §4 — the shadow path is strictly async; the worker SHALL
//! NOT block the gRPC response path.
//!
//! See `tokenizer-service-spec-v1alpha1.md` §4 + `worker` stub in §4.1.

use spendguard_tokenizer::encoders::EncoderKind;
use tokio::sync::mpsc;

/// One sampled tokenize event headed to the shadow worker. Carries
/// enough context to (a) decide whether to actually sample (Phase B
/// rate gating), (b) call the provider Tier 1 endpoint (Phase C), (c)
/// compute drift vs Tier 2 result (Phase E), (d) persist to
/// `tokenizer_t1_samples` (Phase E).
#[derive(Debug, Clone)]
pub struct ShadowEvent {
    pub tenant_id: String,
    pub model: String,
    pub encoder_kind: EncoderKind,
    pub t2_input_tokens: i64,
    pub t2_tokenizer_version_id: String,
    /// Raw text the caller tokenized. We carry it across the channel
    /// (rather than only the count) because the provider count_tokens
    /// APIs need the original text. Bounded by spec §10.1 1 MiB cap
    /// upstream so memory pressure on the channel is bounded.
    pub raw_text: String,
}

/// Handle the shadow worker returns to main.rs for graceful shutdown +
/// best-effort event submission.
#[derive(Debug, Clone)]
pub struct ShadowWorkerHandle {
    sender: mpsc::Sender<ShadowEvent>,
}

impl ShadowWorkerHandle {
    /// Non-blocking try-send. Phase A returns the result so callers can
    /// distinguish "channel full" from "channel closed"; the gRPC
    /// server handler ignores both (Tier 2 hot path is not allowed to
    /// be perturbed by the shadow path per spec §1.3 invariant).
    pub fn try_send(&self, event: ShadowEvent) -> Result<(), mpsc::error::TrySendError<ShadowEvent>> {
        self.sender.try_send(event)
    }
}

/// Phase A skeleton — Phase E ships the real shadow loop. The boot
/// path can call this today; the returned handle is otherwise inert
/// (channel receiver drops the events).
pub fn spawn_shadow_worker(buffer: usize) -> ShadowWorkerHandle {
    let (tx, mut rx) = mpsc::channel::<ShadowEvent>(buffer);
    // Drain receiver so try_send returns Ok until full; Phase E
    // replaces with the real loop.
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            // Drop. Phase E processes the event.
        }
    });
    ShadowWorkerHandle { sender: tx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn try_send_smoke() {
        let handle = spawn_shadow_worker(8);
        let ev = ShadowEvent {
            tenant_id: "t".into(),
            model: "gpt-4o".into(),
            encoder_kind: EncoderKind::OpenAi,
            t2_input_tokens: 10,
            t2_tokenizer_version_id: "01918000-0000-7c10-8c10-000000000001".into(),
            raw_text: "hi".into(),
        };
        // First send should succeed.
        handle.try_send(ev).expect("phase A drain receiver");
    }
}
