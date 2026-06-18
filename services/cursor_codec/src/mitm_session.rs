//! Per-connection MITM session state machine.
//!
//! D17 SLICE 6 ([`design.md`](../../docs/specs/coverage/D17_cursor_mitm/design.md)
//! §5 architecture + [`implementation.md`](../../docs/specs/coverage/D17_cursor_mitm/implementation.md)
//! §3 [`CodecPipeline`]):
//! the SLICE 6 deliverable is the wire integration that ties together
//! framing decode (SLICE 2) + envelope decode (SLICE 3) + translation
//! (SLICE 5) + the sidecar UDS gRPC client. This module is the per-
//! connection state machine.
//!
//! ## State machine
//!
//! ```text
//!  +-------+ accept              +----------+ translate
//!  | Idle  |--- Cursor TCP ----->|RequestDec|----+
//!  +-------+                     +----------+    |
//!                                                v
//!                              +-----------------+----+ reserve via sidecar
//!                              | RequestDecisionReady |
//!                              +-----------------+----+
//!                                                |
//!                            DENY                | CONTINUE / Sidecar OK
//!                +-------------+                 |
//!                | BlockClient |<----------------+ (forward + reservation)
//!                +-------------+                 v
//!                                  +-------------+----+ forward to upstream
//!                                  | UpstreamForward  |
//!                                  +-------------+----+
//!                                                |
//!                                  +-------------+----+ translate response chunks
//!                                  | StreamResponse   |--- back to Cursor client
//!                                  +-------------+----+
//!                                                |
//!                                                | end-of-stream
//!                                                v
//!                                  +-------------+----+ commit with actuals
//!                                  | Commit / Release |
//!                                  +-------------+----+
//! ```
//!
//! ## Why traits for `Upstream` and `Sidecar`
//!
//! Production wiring binds the [`UpstreamConnector`] to a tonic UDS
//! `Client` (the real Cursor backend OR an OpenAI-compatible reflector
//! for the demo) and the [`SidecarLane`] to a real sidecar UDS gRPC
//! channel via [`crate::sidecar_client::SidecarHandle`]. Tests bind to
//! [`CountedUpstream`] and [`InMemorySidecar`] so the SLICE 6 reserve →
//! forward → commit / release cycle is exercised in-process with no
//! real OS sockets.
//!
//! ## Lifecycle hooks
//!
//! [`MitmSession::run`] is the entry point for one Cursor wire request
//! / response pair. It calls [`crate::assert_experimental_banner_emitted`]
//! before any other work — per
//! [`review-standards.md`](../../docs/specs/coverage/D17_cursor_mitm/review-standards.md)
//! §1 (E2) the banner MUST fire on every public entry into the codec.

use std::sync::Arc;
use std::sync::Mutex;

use bytes::Bytes;
use prost::Message;
use thiserror::Error;

use crate::cursor_proto::CursorChatRequest;
use crate::envelope::DecodeError;
use crate::openai_models::{OpenAiChatRequest, OpenAiChatResponseChunk};
use crate::reencode::reencode_frame_with_payload;
use crate::translate::cursor_request_to_openai;

/// Sidecar decision verdict at the LLM_CALL_PRE trigger boundary.
///
/// Maps the proto3 `DecisionResponse::Decision` enum down to the four
/// outcomes the MITM session needs to act on. `Continue` is the
/// happy path; everything else short-circuits the upstream forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarDecision {
    /// Reservation succeeded; forward to upstream.
    ///
    /// `reservation_id` is the sidecar-minted UUIDv7 we use to commit
    /// or release later. `decision_id` is what the audit chain
    /// correlates against.
    Continue {
        /// Sidecar reservation id (UUIDv7).
        reservation_id: String,
        /// Decision id for audit correlation.
        decision_id: String,
    },
    /// Hard block — return an error frame to the client. Maps to the
    /// proto3 `STOP` / `STOP_RUN_PROJECTION` decisions per design §7.
    Stop {
        /// Decision id so the client can log it.
        decision_id: String,
        /// Reason codes the client may surface.
        reason_codes: Vec<String>,
    },
    /// Approval pending. Not supported in SLICE 6 — surfaces as a
    /// codec-level error rather than queuing.
    RequireApproval {
        /// Decision id for follow-up.
        decision_id: String,
    },
    /// Skip — short-circuit but no error.
    Skip {
        /// Decision id for audit.
        decision_id: String,
    },
}

/// Sidecar-side ledger surface. Production binds to the tonic UDS
/// gRPC client; tests bind to [`InMemorySidecar`].
///
/// We deliberately do NOT expose the full sidecar gRPC surface here:
/// the codec only needs the reserve / commit / release contract. The
/// implementation may dispatch through `RequestDecision` +
/// `EmitTraceEvents` + `ConfirmPublishOutcome` (production) or
/// directly maintain an in-memory ledger (tests).
#[allow(async_fn_in_trait)]
pub trait SidecarLane: Send + Sync {
    /// Reserve a budget for the translated OpenAI request.
    ///
    /// Returns [`SidecarDecision::Continue`] on success; the codec
    /// short-circuits the upstream forward on any other variant.
    async fn reserve(&self, req: &OpenAiChatRequest) -> Result<SidecarDecision, SessionError>;

    /// Commit the reservation with the actual output-token count.
    ///
    /// Called from the response path at end-of-stream. Production
    /// emits an `LLM_CALL_POST(SUCCESS)` trace event followed by
    /// `ConfirmPublishOutcome(APPLIED)`. Tests record the commit in
    /// the in-memory ledger.
    async fn commit(
        &self,
        reservation_id: &str,
        decision_id: &str,
        actual_output_tokens: u32,
    ) -> Result<(), SessionError>;

    /// Release the reservation explicitly.
    ///
    /// Called from the error path (translation failed, upstream
    /// errored, end-of-stream never reached). Production emits an
    /// `LLM_CALL_POST(PROVIDER_ERROR)` trace event followed by
    /// `ConfirmPublishOutcome(APPLY_FAILED)`. Tests record the
    /// release in the in-memory ledger.
    async fn release(
        &self,
        reservation_id: &str,
        decision_id: &str,
        reason: ReleaseReason,
    ) -> Result<(), SessionError>;
}

/// Why a reservation was released. Maps to the proto3
/// `ReleaseReservationRequest::reason_codes` enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    /// Upstream call errored.
    ProviderError,
    /// Codec-side error (envelope decode failed, etc).
    RuntimeError,
    /// Cursor IDE client cancelled mid-stream.
    ClientTimeout,
    /// Codec-level abort (translator rejected the request).
    RunAborted,
}

impl ReleaseReason {
    /// String form for ASP Draft-01 `reason_codes` wire field.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ProviderError => "provider_error",
            Self::RuntimeError => "runtime_error",
            Self::ClientTimeout => "client_timeout",
            Self::RunAborted => "run_aborted",
        }
    }
}

/// Upstream forwarding surface. Production binds to a tonic client
/// dialing the real Cursor backend OR an OpenAI-compatible reflector
/// (demo path); tests bind to [`CountedUpstream`].
#[allow(async_fn_in_trait)]
pub trait UpstreamConnector: Send + Sync {
    /// Forward the translated OpenAI request and return a Vec of
    /// response chunks. The SLICE 6 contract is that the upstream
    /// adapter handles its own framing — the MITM session sees the
    /// canonical OpenAI shape.
    async fn forward(
        &self,
        req: &OpenAiChatRequest,
    ) -> Result<Vec<OpenAiChatResponseChunk>, SessionError>;
}

/// Session-level error surface. All variants map to a specific
/// release reason so the SLICE 6 cleanup path is unambiguous.
#[derive(Debug, Error)]
pub enum SessionError {
    /// Cursor envelope decode failed.
    #[error("cursor envelope decode failed: {0}")]
    EnvelopeDecode(#[from] DecodeError),

    /// Sidecar reserve / commit / release failed.
    #[error("sidecar lane error: {0}")]
    Sidecar(String),

    /// Upstream call errored.
    #[error("upstream error: {0}")]
    Upstream(String),

    /// Sidecar returned STOP at the decision boundary.
    #[error("sidecar decision blocked: decision_id={decision_id} reasons={reason_codes:?}")]
    Blocked {
        /// Decision id for audit correlation.
        decision_id: String,
        /// Reason codes the sidecar attached.
        reason_codes: Vec<String>,
    },

    /// Sidecar returned an unsupported decision variant.
    #[error("sidecar returned unsupported decision: {0}")]
    UnsupportedDecision(String),
}

/// The result of running one MITM session — what to send back to the
/// Cursor client on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResult {
    /// Re-encoded Cursor wire bytes (frames concatenated). The client
    /// receives these byte-for-byte.
    pub response_bytes: Bytes,
    /// The reservation id that was committed or released. `None` on
    /// the deny path where no reservation was minted.
    pub reservation_id: Option<String>,
    /// Whether a commit happened (`true`) or a release happened
    /// (`false`). The deny path leaves this as `None`.
    pub committed: Option<bool>,
}

/// The MITM session state machine.
///
/// One instance per Cursor wire request / response pair.
pub struct MitmSession<S: SidecarLane, U: UpstreamConnector> {
    sidecar: Arc<S>,
    upstream: Arc<U>,
}

impl<S: SidecarLane, U: UpstreamConnector> MitmSession<S, U> {
    /// Build a new session bound to the given sidecar and upstream
    /// surfaces.
    pub fn new(sidecar: Arc<S>, upstream: Arc<U>) -> Self {
        Self { sidecar, upstream }
    }

    /// Run one round: decode the Cursor wire bytes, translate, reserve,
    /// forward upstream, translate the response, re-encode the Cursor
    /// wire, commit / release.
    ///
    /// This is the SLICE 6 happy-path + error-path contract:
    ///
    /// * DecodeError → no reservation was minted yet; return error,
    ///   no sidecar call.
    /// * Sidecar STOP → no reservation was minted; return error.
    /// * Upstream error → reservation was minted; release with
    ///   [`ReleaseReason::ProviderError`].
    /// * End-of-stream OK → reservation was minted; commit with the
    ///   actuals extracted from the terminal chunk.
    pub async fn run(&self, cursor_request_bytes: &[u8]) -> Result<SessionResult, SessionError> {
        crate::assert_experimental_banner_emitted();

        // 1. Decode the Cursor request envelope. No sidecar call yet.
        let cursor_req = CursorChatRequest::decode(cursor_request_bytes)
            .map_err(|e| SessionError::EnvelopeDecode(DecodeError::Prost(e)))?;
        if cursor_req.model.is_empty() {
            return Err(SessionError::EnvelopeDecode(DecodeError::MissingField {
                field: "model",
            }));
        }

        // 2. Translate to canonical OpenAI shape.
        let openai_req = cursor_request_to_openai(&cursor_req);

        // 3. Sidecar reserve.
        let decision = self.sidecar.reserve(&openai_req).await?;
        let (reservation_id, decision_id) = match decision {
            SidecarDecision::Continue {
                reservation_id,
                decision_id,
            } => (reservation_id, decision_id),
            SidecarDecision::Stop {
                decision_id,
                reason_codes,
            } => {
                return Err(SessionError::Blocked {
                    decision_id,
                    reason_codes,
                });
            }
            SidecarDecision::RequireApproval { decision_id } => {
                return Err(SessionError::UnsupportedDecision(format!(
                    "REQUIRE_APPROVAL (decision_id={decision_id})"
                )));
            }
            SidecarDecision::Skip { decision_id } => {
                return Err(SessionError::UnsupportedDecision(format!(
                    "SKIP (decision_id={decision_id})"
                )));
            }
        };

        // 4. Forward to upstream. If upstream errors, release.
        let chunks = match self.upstream.forward(&openai_req).await {
            Ok(chunks) => chunks,
            Err(e) => {
                let _ = self
                    .sidecar
                    .release(&reservation_id, &decision_id, ReleaseReason::ProviderError)
                    .await;
                return Err(e);
            }
        };

        // 5. Translate response chunks back to Cursor wire and re-
        //    encode. The codec sees a per-chunk list; we wrap each in
        //    a Connect-RPC frame and concatenate.
        let response_bytes = match build_cursor_response_wire(&chunks) {
            Ok(b) => b,
            Err(e) => {
                let _ = self
                    .sidecar
                    .release(&reservation_id, &decision_id, ReleaseReason::RuntimeError)
                    .await;
                return Err(e);
            }
        };

        // 6. Extract actual output tokens for commit.
        let actual_output_tokens = commit_output_tokens(&chunks);

        // 7. Commit.
        self.sidecar
            .commit(&reservation_id, &decision_id, actual_output_tokens)
            .await?;

        Ok(SessionResult {
            response_bytes,
            reservation_id: Some(reservation_id),
            committed: Some(true),
        })
    }
}

/// Build a Cursor wire response from translated chunks.
///
/// Each chunk gets its own `[flag=0x00][len BE][payload]` frame; the
/// terminal frame is `[flag=FLAG_END_OF_STREAM][len=0][]`.
fn build_cursor_response_wire(chunks: &[OpenAiChatResponseChunk]) -> Result<Bytes, SessionError> {
    use crate::framing::Frame as F;
    use crate::framing::FLAG_END_OF_STREAM;

    let mut out = Vec::new();
    for chunk in chunks {
        let cursor_chunk = crate::translate::openai_chunk_to_cursor(chunk);
        let mut payload = Vec::new();
        cursor_chunk
            .encode(&mut payload)
            .map_err(|e| SessionError::Sidecar(format!("encode chunk: {e}")))?;
        let frame = F {
            flags: 0x00,
            payload: Bytes::from(payload.clone()),
        };
        out.extend_from_slice(&reencode_frame_with_payload(&frame, Bytes::from(payload)));
    }
    // Terminal EOS frame.
    let eos = F {
        flags: FLAG_END_OF_STREAM,
        payload: Bytes::new(),
    };
    out.extend_from_slice(&reencode_frame_with_payload(&eos, Bytes::new()));
    Ok(Bytes::from(out))
}

/// Derive the output-token count to commit from the response chunks.
///
/// ## Why this is not just `max(completion_tokens)`
///
/// The translator never sets `stream_options.include_usage` on the
/// forwarded request, so most backends stream WITHOUT a terminal usage
/// block. A naive `max(completion_tokens).unwrap_or(0)` then commits
/// `0` for every successful streaming call — silently under-metering
/// spend (a fail-OPEN data-integrity bug: real money leaks past the
/// guardrail because the ledger believes nothing was consumed).
///
/// The fix distinguishes the two ways the count can be zero:
///
/// 1. **At least one chunk carried a usage block.** The provider told
///    us the truth — trust it. We take the maximum `completion_tokens`
///    across the usage blocks (OpenAI reports a single cumulative count
///    on the terminal chunk; `max` is robust if a backend reports
///    per-chunk deltas). A genuine `0` here (empty / refused / filtered
///    completion) is PRESERVED — we do NOT floor it, because flooring a
///    truthful zero would over-bill.
///
/// 2. **No chunk carried any usage block.** We have no provider-reported
///    actuals, so committing `0` would under-meter. Instead we fall back
///    to estimating from the streamed delta content (the fallback
///    [`crate::translate::extract_openai_output_tokens`] documents). The
///    estimate is fail-closed: `ceil(chars / 4)` (the workspace's
///    chars-per-token proxy), rounding partial tokens UP and yielding at
///    least `1` whenever any content was actually streamed. A stream
///    that produced no content at all still commits `0` — the legitimate
///    empty-completion case.
fn commit_output_tokens(chunks: &[OpenAiChatResponseChunk]) -> u32 {
    // Provider-reported path: trust the usage block(s) if any are present,
    // preserving a truthful zero.
    let mut saw_usage = false;
    let mut reported = 0u32;
    for c in chunks {
        if let Some(usage) = c.usage.as_ref() {
            saw_usage = true;
            reported = reported.max(usage.completion_tokens);
        }
    }
    if saw_usage {
        return reported;
    }

    // Absence-of-usage fallback: estimate from streamed delta content so
    // a usage-less stream is never silently committed as zero spend.
    let streamed_chars: usize = chunks
        .iter()
        .flat_map(|c| c.choices.iter())
        .filter_map(|choice| choice.delta.content.as_deref())
        .map(|content| content.chars().count())
        .sum();

    // `div_ceil` rounds partial tokens UP (fail-closed). Saturating cast
    // guards the (practically unreachable) >u32::MAX-token case.
    u32::try_from(streamed_chars.div_ceil(4)).unwrap_or(u32::MAX)
}

// ============================================================================
// Test surfaces
// ============================================================================

/// In-memory sidecar lane. Records every reserve / commit / release
/// call so the SLICE 6 tests can assert the cycle ran end-to-end.
///
/// `next_decision` lets a test pre-program what the next reserve call
/// returns. Default is `Continue` with a fresh UUIDv7 reservation.
pub struct InMemorySidecar {
    inner: Mutex<InMemorySidecarInner>,
}

struct InMemorySidecarInner {
    next_decision: Option<SidecarDecision>,
    reserve_calls: Vec<OpenAiChatRequest>,
    commit_calls: Vec<(String, String, u32)>,
    release_calls: Vec<(String, String, ReleaseReason)>,
}

impl Default for InMemorySidecar {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySidecar {
    /// Build a fresh sidecar stub that always reserves.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(InMemorySidecarInner {
                next_decision: None,
                reserve_calls: vec![],
                commit_calls: vec![],
                release_calls: vec![],
            }),
        }
    }

    /// Program the next reserve call to return the given decision.
    pub fn program_next_decision(&self, decision: SidecarDecision) {
        let mut g = self.inner.lock().unwrap();
        g.next_decision = Some(decision);
    }

    /// Number of reserve calls recorded.
    pub fn reserve_count(&self) -> usize {
        self.inner.lock().unwrap().reserve_calls.len()
    }
    /// Number of commit calls recorded.
    pub fn commit_count(&self) -> usize {
        self.inner.lock().unwrap().commit_calls.len()
    }
    /// Number of release calls recorded.
    pub fn release_count(&self) -> usize {
        self.inner.lock().unwrap().release_calls.len()
    }
    /// Get a snapshot of all commit calls.
    pub fn commit_calls(&self) -> Vec<(String, String, u32)> {
        self.inner.lock().unwrap().commit_calls.clone()
    }
    /// Get a snapshot of all release calls.
    pub fn release_calls(&self) -> Vec<(String, String, ReleaseReason)> {
        self.inner.lock().unwrap().release_calls.clone()
    }
}

impl SidecarLane for InMemorySidecar {
    async fn reserve(&self, req: &OpenAiChatRequest) -> Result<SidecarDecision, SessionError> {
        let mut g = self.inner.lock().unwrap();
        g.reserve_calls.push(req.clone());
        let d = g
            .next_decision
            .take()
            .unwrap_or_else(|| SidecarDecision::Continue {
                reservation_id: uuid::Uuid::now_v7().to_string(),
                decision_id: uuid::Uuid::now_v7().to_string(),
            });
        Ok(d)
    }

    async fn commit(
        &self,
        reservation_id: &str,
        decision_id: &str,
        actual_output_tokens: u32,
    ) -> Result<(), SessionError> {
        let mut g = self.inner.lock().unwrap();
        g.commit_calls.push((
            reservation_id.to_string(),
            decision_id.to_string(),
            actual_output_tokens,
        ));
        Ok(())
    }

    async fn release(
        &self,
        reservation_id: &str,
        decision_id: &str,
        reason: ReleaseReason,
    ) -> Result<(), SessionError> {
        let mut g = self.inner.lock().unwrap();
        g.release_calls
            .push((reservation_id.to_string(), decision_id.to_string(), reason));
        Ok(())
    }
}

/// Counted upstream stub. Records every forward call and returns a
/// pre-programmed list of OpenAI response chunks.
pub struct CountedUpstream {
    inner: Mutex<CountedUpstreamInner>,
}

struct CountedUpstreamInner {
    forwards: Vec<OpenAiChatRequest>,
    next_response: Result<Vec<OpenAiChatResponseChunk>, String>,
}

impl CountedUpstream {
    /// Build an upstream that returns the given chunks on the next forward.
    pub fn returning(chunks: Vec<OpenAiChatResponseChunk>) -> Self {
        Self {
            inner: Mutex::new(CountedUpstreamInner {
                forwards: vec![],
                next_response: Ok(chunks),
            }),
        }
    }

    /// Build an upstream that errors on the next forward.
    pub fn erroring(msg: impl Into<String>) -> Self {
        Self {
            inner: Mutex::new(CountedUpstreamInner {
                forwards: vec![],
                next_response: Err(msg.into()),
            }),
        }
    }

    /// Number of forward calls recorded.
    pub fn forward_count(&self) -> usize {
        self.inner.lock().unwrap().forwards.len()
    }

    /// Snapshot of all forward requests.
    pub fn forwarded(&self) -> Vec<OpenAiChatRequest> {
        self.inner.lock().unwrap().forwards.clone()
    }
}

impl UpstreamConnector for CountedUpstream {
    async fn forward(
        &self,
        req: &OpenAiChatRequest,
    ) -> Result<Vec<OpenAiChatResponseChunk>, SessionError> {
        let mut g = self.inner.lock().unwrap();
        g.forwards.push(req.clone());
        match &g.next_response {
            Ok(chunks) => Ok(chunks.clone()),
            Err(msg) => Err(SessionError::Upstream(msg.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor_proto::{CursorChatResponseChunk, Message as CursorMessage};
    use crate::openai_models::{OpenAiChunkChoice, OpenAiChunkDelta, OpenAiUsage};

    fn cursor_request_bytes(model: &str, content: &str) -> Vec<u8> {
        let req = CursorChatRequest {
            messages: vec![CursorMessage {
                role: "user".to_string(),
                content: content.to_string(),
            }],
            model: model.to_string(),
            system: None,
            max_tokens: Some(64),
            temperature: Some(0.2),
        };
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        buf
    }

    fn streaming_response(total_tokens: u32) -> Vec<OpenAiChatResponseChunk> {
        vec![
            OpenAiChatResponseChunk {
                model: "gpt-4o-mini".to_string(),
                choices: vec![OpenAiChunkChoice {
                    index: 0,
                    delta: OpenAiChunkDelta {
                        role: Some("assistant".to_string()),
                        content: Some("Hi".to_string()),
                    },
                    finish_reason: None,
                }],
                usage: None,
                extra: Default::default(),
            },
            OpenAiChatResponseChunk {
                model: "gpt-4o-mini".to_string(),
                choices: vec![OpenAiChunkChoice {
                    index: 0,
                    delta: OpenAiChunkDelta {
                        role: None,
                        content: Some(" there".to_string()),
                    },
                    finish_reason: None,
                }],
                usage: None,
                extra: Default::default(),
            },
            OpenAiChatResponseChunk {
                model: "gpt-4o-mini".to_string(),
                choices: vec![OpenAiChunkChoice {
                    index: 0,
                    delta: OpenAiChunkDelta {
                        role: None,
                        content: None,
                    },
                    finish_reason: Some("stop".to_string()),
                }],
                usage: Some(OpenAiUsage {
                    prompt_tokens: 8,
                    completion_tokens: total_tokens,
                    total_tokens: 8 + total_tokens,
                }),
                extra: Default::default(),
            },
        ]
    }

    /// A streaming response with NO usage block on any chunk — the
    /// common case, because the translator never sets
    /// `stream_options.include_usage`. `content` is split into two
    /// streamed deltas so the absence-of-usage fallback has data to
    /// estimate from.
    fn streaming_response_no_usage(content: &str) -> Vec<OpenAiChatResponseChunk> {
        let (a, b) = content.split_at(content.len() / 2);
        vec![
            OpenAiChatResponseChunk {
                model: "gpt-4o-mini".to_string(),
                choices: vec![OpenAiChunkChoice {
                    index: 0,
                    delta: OpenAiChunkDelta {
                        role: Some("assistant".to_string()),
                        content: Some(a.to_string()),
                    },
                    finish_reason: None,
                }],
                usage: None,
                extra: Default::default(),
            },
            OpenAiChatResponseChunk {
                model: "gpt-4o-mini".to_string(),
                choices: vec![OpenAiChunkChoice {
                    index: 0,
                    delta: OpenAiChunkDelta {
                        role: None,
                        content: Some(b.to_string()),
                    },
                    finish_reason: Some("stop".to_string()),
                }],
                usage: None,
                extra: Default::default(),
            },
        ]
    }

    /// (1) Full reserve + commit cycle on the happy path.
    #[tokio::test]
    async fn reserve_then_commit_on_success() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(17)));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cursor_request_bytes("gpt-4o-mini", "hello");
        let result = session.run(&bytes).await.unwrap();

        assert_eq!(sidecar.reserve_count(), 1);
        assert_eq!(sidecar.commit_count(), 1);
        assert_eq!(sidecar.release_count(), 0);
        assert_eq!(upstream.forward_count(), 1);
        assert_eq!(result.committed, Some(true));
        let commits = sidecar.commit_calls();
        assert_eq!(commits[0].2, 17, "commit actual_output_tokens should be 17");
    }

    /// (2) DENY blocks the forward; no reservation is committed.
    #[tokio::test]
    async fn deny_blocks_forward() {
        let sidecar = Arc::new(InMemorySidecar::new());
        sidecar.program_next_decision(SidecarDecision::Stop {
            decision_id: "decision-deny".to_string(),
            reason_codes: vec!["BUDGET_EXHAUSTED".to_string()],
        });
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(0)));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cursor_request_bytes("gpt-4o-mini", "hello");
        let err = session.run(&bytes).await.unwrap_err();

        assert!(
            matches!(err, SessionError::Blocked { .. }),
            "expected Blocked, got: {err:?}"
        );
        assert_eq!(
            upstream.forward_count(),
            0,
            "upstream MUST NOT be hit on DENY"
        );
        assert_eq!(sidecar.commit_count(), 0);
        assert_eq!(sidecar.release_count(), 0);
    }

    /// (3) Upstream error releases the reservation with ProviderError.
    #[tokio::test]
    async fn upstream_error_releases_reservation() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::erroring("connection refused"));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cursor_request_bytes("gpt-4o-mini", "hello");
        let err = session.run(&bytes).await.unwrap_err();

        assert!(matches!(err, SessionError::Upstream(_)), "got: {err:?}");
        assert_eq!(sidecar.reserve_count(), 1);
        assert_eq!(sidecar.commit_count(), 0);
        assert_eq!(sidecar.release_count(), 1);
        let releases = sidecar.release_calls();
        assert_eq!(releases[0].2, ReleaseReason::ProviderError);
    }

    /// (4) Envelope decode error → no sidecar call.
    #[tokio::test]
    async fn envelope_decode_error_skips_sidecar() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(0)));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bad_bytes = [0xffu8; 16];
        let err = session.run(&bad_bytes).await.unwrap_err();
        assert!(
            matches!(err, SessionError::EnvelopeDecode(_)),
            "got: {err:?}"
        );
        assert_eq!(sidecar.reserve_count(), 0);
        assert_eq!(sidecar.commit_count(), 0);
        assert_eq!(sidecar.release_count(), 0);
        assert_eq!(upstream.forward_count(), 0);
    }

    /// (5) Empty-model envelope: rejected before sidecar.
    #[tokio::test]
    async fn empty_model_rejected_pre_sidecar() {
        let cursor_req = CursorChatRequest {
            messages: vec![CursorMessage {
                role: "user".to_string(),
                content: "x".to_string(),
            }],
            model: String::new(),
            system: None,
            max_tokens: None,
            temperature: None,
        };
        let mut bytes = Vec::new();
        cursor_req.encode(&mut bytes).unwrap();

        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(vec![]));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));
        let err = session.run(&bytes).await.unwrap_err();
        match err {
            SessionError::EnvelopeDecode(DecodeError::MissingField { field }) => {
                assert_eq!(field, "model");
            }
            other => panic!("expected MissingField(model), got: {other:?}"),
        }
        assert_eq!(sidecar.reserve_count(), 0);
    }

    /// (6) RequireApproval → unsupported decision; no upstream call.
    #[tokio::test]
    async fn require_approval_unsupported() {
        let sidecar = Arc::new(InMemorySidecar::new());
        sidecar.program_next_decision(SidecarDecision::RequireApproval {
            decision_id: "decision-approval".to_string(),
        });
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(0)));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cursor_request_bytes("gpt-4o-mini", "hello");
        let err = session.run(&bytes).await.unwrap_err();
        assert!(
            matches!(err, SessionError::UnsupportedDecision(_)),
            "got: {err:?}"
        );
        assert_eq!(upstream.forward_count(), 0);
    }

    /// (7) Response wire bytes round-trip through the framing reader.
    #[tokio::test]
    async fn response_wire_bytes_decode_back_to_chunks() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(5)));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cursor_request_bytes("gpt-4o-mini", "hello");
        let result = session.run(&bytes).await.unwrap();

        // Decode the response bytes back through the framing reader and
        // assert we get N data frames + 1 EOS frame.
        let mut reader = crate::framing::ConnectRpcReader::new(std::io::Cursor::new(
            result.response_bytes.to_vec(),
        ));
        let mut data_frames = 0;
        let mut eos_frames = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            if frame.is_end_of_stream() {
                eos_frames += 1;
            } else {
                data_frames += 1;
                // Each data frame decodes to a CursorChatResponseChunk.
                let _ = CursorChatResponseChunk::decode(&frame.payload[..]).unwrap();
            }
        }
        assert_eq!(data_frames, 3);
        assert_eq!(eos_frames, 1);
    }

    /// (8) The forwarded OpenAI request matches the translated Cursor envelope.
    #[tokio::test]
    async fn forwarded_request_matches_translated_envelope() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(3)));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cursor_request_bytes("claude-3.5-sonnet", "be brief");
        let _ = session.run(&bytes).await.unwrap();

        let forwarded = upstream.forwarded();
        assert_eq!(forwarded.len(), 1);
        assert_eq!(forwarded[0].model, "claude-3.5-sonnet");
        assert_eq!(forwarded[0].messages.len(), 1);
        assert_eq!(forwarded[0].messages[0].role, "user");
        assert_eq!(forwarded[0].messages[0].content, "be brief");
        assert_eq!(forwarded[0].stream, Some(true));
    }

    /// (9) Skip decision short-circuits the upstream.
    #[tokio::test]
    async fn skip_short_circuits_upstream() {
        let sidecar = Arc::new(InMemorySidecar::new());
        sidecar.program_next_decision(SidecarDecision::Skip {
            decision_id: "decision-skip".to_string(),
        });
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(0)));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cursor_request_bytes("gpt-4o-mini", "hello");
        let err = session.run(&bytes).await.unwrap_err();
        assert!(
            matches!(err, SessionError::UnsupportedDecision(_)),
            "got: {err:?}"
        );
        assert_eq!(upstream.forward_count(), 0);
        assert_eq!(sidecar.commit_count(), 0);
    }

    /// (10) Usage-present path is unchanged: commit the reported count.
    #[test]
    fn commit_tokens_trusts_reported_usage() {
        let chunks = streaming_response(17);
        assert_eq!(commit_output_tokens(&chunks), 17);
    }

    /// (11) A truthful zero (empty / refused completion, usage block
    /// present) is PRESERVED — we must not floor it, that would over-bill.
    #[test]
    fn commit_tokens_preserves_genuine_zero_when_usage_present() {
        let chunks = streaming_response(0);
        assert_eq!(
            commit_output_tokens(&chunks),
            0,
            "a provider-reported zero must not be floored"
        );
    }

    /// (12) The fail-OPEN bug: a usage-less stream that actually carried
    /// content must NOT commit zero. It falls back to a ceil(chars/4)
    /// estimate so spend is metered.
    #[test]
    fn commit_tokens_estimates_when_no_usage_block() {
        // 20 streamed chars → ceil(20/4) = 5 estimated output tokens.
        let chunks = streaming_response_no_usage("abcdefghijklmnopqrst");
        assert_eq!(commit_output_tokens(&chunks), 5);
    }

    /// (13) A usage-less stream with NO content at all is a legitimate
    /// empty completion → commit zero (do not synthesize spend).
    #[test]
    fn commit_tokens_zero_when_no_usage_and_no_content() {
        let chunks = streaming_response_no_usage("");
        assert_eq!(commit_output_tokens(&chunks), 0);
    }

    /// (14) End-to-end: a successful streaming call without a usage block
    /// commits a NONZERO estimate, not silent zero. This is the
    /// regression guard for the under-metering fail-open.
    #[tokio::test]
    async fn streaming_without_usage_commits_nonzero_estimate() {
        let sidecar = Arc::new(InMemorySidecar::new());
        // 12 streamed chars → ceil(12/4) = 3.
        let upstream = Arc::new(CountedUpstream::returning(streaming_response_no_usage(
            "hello world!",
        )));
        let session = MitmSession::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cursor_request_bytes("gpt-4o-mini", "hello");
        let result = session.run(&bytes).await.unwrap();

        assert_eq!(result.committed, Some(true));
        assert_eq!(sidecar.commit_count(), 1);
        let commits = sidecar.commit_calls();
        assert_eq!(
            commits[0].2, 3,
            "usage-less stream must commit the ceil(chars/4) estimate, not 0"
        );
    }
}
