//! D18 SLICE 78 — per-connection MITM forward state machine.
//!
//! Mirrors D17's `cursor_codec::mitm_session`: ties together framing
//! decode + envelope decode + translation + sidecar reserve-commit-
//! release lifecycle. The codec exposes a [`SidecarLane`] trait so
//! production wiring binds the real sidecar gRPC UDS client and
//! tests bind to [`InMemorySidecar`].
//!
//! ## State machine
//!
//! ```text
//!  +-------+ accept                +----------+ decode
//!  | Idle  |--- Cascade gRPC ----->| RequestPb|----+
//!  +-------+                       +----------+    |
//!                                                  v
//!                              +-------------------+----+ translate to OpenAI
//!                              | OpenAiCanonicalRequest |
//!                              +-------------------+----+
//!                                                  |
//!                                       Reserve via sidecar
//!                                                  |
//!                            STOP                  | CONTINUE
//!                +-------------+                   v
//!                | BlockClient |<------+----+ forward upstream byte-perfect
//!                +-------------+       |    |
//!                                      |    v
//!                                +-----+----+ decode response deltas
//!                                | StreamRsp |--- back to Windsurf client
//!                                +-----+----+
//!                                      |
//!                                      | terminal delta with usage
//!                                      v
//!                                +-----+----+ commit with actuals
//!                                | Commit/Rel |
//!                                +-----+----+
//! ```

use std::sync::Arc;
use std::sync::Mutex;

use thiserror::Error;

use crate::envelope::decode_request_body;
use crate::error::WindsurfCodecError;
use crate::openai_models::{OpenAiChatRequest, OpenAiChatResponseChunk};
use crate::translate::cascade_request_to_openai;

/// Sidecar decision verdict at the LLM_CALL_PRE trigger boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarDecision {
    /// Reservation succeeded; forward to upstream.
    Continue {
        /// Sidecar reservation id (UUIDv7).
        reservation_id: String,
        /// Decision id for audit correlation.
        decision_id: String,
    },
    /// Hard block — return an error frame to the client.
    Stop {
        /// Decision id so the client can log it.
        decision_id: String,
        /// Reason codes the client may surface.
        reason_codes: Vec<String>,
    },
    /// Approval pending. Not supported in SLICE 78.
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
#[allow(async_fn_in_trait)]
pub trait SidecarLane: Send + Sync {
    /// Reserve a budget for the translated OpenAI request.
    async fn reserve(&self, req: &OpenAiChatRequest) -> Result<SidecarDecision, SessionError>;

    /// Commit the reservation with the actual output-token count.
    async fn commit(
        &self,
        reservation_id: &str,
        decision_id: &str,
        actual_output_tokens: u32,
    ) -> Result<(), SessionError>;

    /// Release the reservation explicitly.
    async fn release(
        &self,
        reservation_id: &str,
        decision_id: &str,
        reason: ReleaseReason,
    ) -> Result<(), SessionError>;
}

/// Why a reservation was released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    /// Upstream call errored.
    ProviderError,
    /// Codec-side error (envelope decode failed, etc).
    RuntimeError,
    /// Windsurf IDE client cancelled mid-stream.
    ClientTimeout,
    /// Codec-level abort (translator rejected the request).
    RunAborted,
    /// Unknown wire version — codec couldn't decode at all.
    UnsupportedWireVersion,
}

impl ReleaseReason {
    /// String form for ASP Draft-01 `reason_codes` wire field.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ProviderError => "provider_error",
            Self::RuntimeError => "runtime_error",
            Self::ClientTimeout => "client_timeout",
            Self::RunAborted => "run_aborted",
            Self::UnsupportedWireVersion => "windsurf_wire_version_unsupported",
        }
    }
}

/// Upstream forwarding surface.
#[allow(async_fn_in_trait)]
pub trait UpstreamConnector: Send + Sync {
    /// Forward the translated OpenAI request and return a Vec of
    /// response chunks.
    async fn forward(
        &self,
        req: &OpenAiChatRequest,
    ) -> Result<Vec<OpenAiChatResponseChunk>, SessionError>;
}

/// Session-level error surface.
#[derive(Debug, Error)]
pub enum SessionError {
    /// Cascade envelope decode failed.
    #[error("cascade envelope decode failed: {0}")]
    EnvelopeDecode(#[from] WindsurfCodecError),

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

/// The result of running one MITM forward session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResult {
    /// The reservation id that was committed or released. `None` on
    /// the deny path where no reservation was minted.
    pub reservation_id: Option<String>,
    /// Whether a commit happened (`true`) or a release happened
    /// (`false`). The deny path leaves this as `None`.
    pub committed: Option<bool>,
    /// The model the Cascade request advertised. Useful for the
    /// audit chain.
    pub model_name: String,
    /// The actual output-token count we observed (0 when terminal
    /// usage stamp was missing).
    pub actual_output_tokens: u32,
}

/// The MITM forward state machine.
pub struct MitmForward<S: SidecarLane, U: UpstreamConnector> {
    sidecar: Arc<S>,
    upstream: Arc<U>,
}

impl<S: SidecarLane, U: UpstreamConnector> MitmForward<S, U> {
    /// Build a new forward session.
    pub fn new(sidecar: Arc<S>, upstream: Arc<U>) -> Self {
        Self { sidecar, upstream }
    }

    /// Run one round: decode the Cascade request body (already
    /// stripped of the 5-byte gRPC-Web prefix by the framing reader),
    /// translate, reserve, forward upstream, translate the response,
    /// commit / release.
    ///
    /// SLICE 78 contract:
    ///
    /// * Decode error → no reservation; return error.
    /// * Sidecar STOP → no reservation; return error.
    /// * Upstream error → reservation minted; release with
    ///   [`ReleaseReason::ProviderError`].
    /// * Terminal usage OK → reservation minted; commit with
    ///   actuals extracted from the terminal delta.
    pub async fn run(&self, cascade_request_body: &[u8]) -> Result<SessionResult, SessionError> {
        crate::assert_experimental_banner_emitted();

        // 1. Decode the Cascade request envelope.
        let cascade_req = decode_request_body(cascade_request_body)?;
        let model_name = cascade_req.model_name.clone();

        // 2. Translate to canonical OpenAI shape.
        let openai_req = cascade_request_to_openai(&cascade_req);

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

        // 4. Forward to upstream. On error, release.
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

        // 5. Extract actual output tokens for commit (the maximum
        //    observed usage.completion_tokens across the streamed
        //    chunks — Cascade and OpenAI both stamp on the terminal
        //    delta).
        let actual_output_tokens = chunks
            .iter()
            .map(|c| c.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0))
            .max()
            .unwrap_or(0);

        // 6. Commit.
        self.sidecar
            .commit(&reservation_id, &decision_id, actual_output_tokens)
            .await?;

        Ok(SessionResult {
            reservation_id: Some(reservation_id),
            committed: Some(true),
            model_name,
            actual_output_tokens,
        })
    }
}

// ============================================================================
// Test surfaces
// ============================================================================

/// In-memory sidecar lane. Records every reserve / commit / release
/// call so the SLICE 78 tests can assert the cycle ran end-to-end.
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
    /// Snapshot of all commit calls.
    pub fn commit_calls(&self) -> Vec<(String, String, u32)> {
        self.inner.lock().unwrap().commit_calls.clone()
    }
    /// Snapshot of all release calls.
    pub fn release_calls(&self) -> Vec<(String, String, ReleaseReason)> {
        self.inner.lock().unwrap().release_calls.clone()
    }
}

#[cfg(feature = "mitm")]
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

#[cfg(not(feature = "mitm"))]
impl SidecarLane for InMemorySidecar {
    async fn reserve(&self, req: &OpenAiChatRequest) -> Result<SidecarDecision, SessionError> {
        let mut g = self.inner.lock().unwrap();
        g.reserve_calls.push(req.clone());
        let d = g
            .next_decision
            .take()
            .unwrap_or_else(|| SidecarDecision::Continue {
                reservation_id: "test-reservation".to_string(),
                decision_id: "test-decision".to_string(),
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
    /// Build an upstream that returns the given chunks.
    pub fn returning(chunks: Vec<OpenAiChatResponseChunk>) -> Self {
        Self {
            inner: Mutex::new(CountedUpstreamInner {
                forwards: vec![],
                next_response: Ok(chunks),
            }),
        }
    }

    /// Build an upstream that errors.
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
    use crate::openai_models::{OpenAiChunkChoice, OpenAiChunkDelta, OpenAiUsage};
    use crate::windsurf_proto::{CascadeMessage, CascadeRequest};
    use prost::Message;

    fn cascade_request_bytes(model_name: &str, content: &str) -> Vec<u8> {
        let req = CascadeRequest {
            messages: vec![CascadeMessage {
                role: "user".to_string(),
                content: content.to_string(),
            }],
            model_name: model_name.to_string(),
            max_tokens: Some(64),
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        };
        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        buf
    }

    fn streaming_response(total_tokens: u32) -> Vec<OpenAiChatResponseChunk> {
        vec![
            OpenAiChatResponseChunk {
                model: "gpt-4o".to_string(),
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
                model: "gpt-4o".to_string(),
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

    /// (1) Full reserve + commit cycle on happy path.
    #[tokio::test]
    async fn reserve_then_commit_on_success() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(17)));
        let forward = MitmForward::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cascade_request_bytes("gpt-4o", "hello");
        let result = forward.run(&bytes).await.unwrap();

        assert_eq!(sidecar.reserve_count(), 1);
        assert_eq!(sidecar.commit_count(), 1);
        assert_eq!(sidecar.release_count(), 0);
        assert_eq!(upstream.forward_count(), 1);
        assert_eq!(result.committed, Some(true));
        assert_eq!(result.model_name, "gpt-4o");
        assert_eq!(result.actual_output_tokens, 17);
    }

    /// (2) STOP blocks the forward.
    #[tokio::test]
    async fn stop_blocks_forward() {
        let sidecar = Arc::new(InMemorySidecar::new());
        sidecar.program_next_decision(SidecarDecision::Stop {
            decision_id: "decision-deny".to_string(),
            reason_codes: vec!["BUDGET_EXHAUSTED".to_string()],
        });
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(0)));
        let forward = MitmForward::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cascade_request_bytes("gpt-4o", "hello");
        let err = forward.run(&bytes).await.unwrap_err();

        assert!(matches!(err, SessionError::Blocked { .. }), "{err:?}");
        assert_eq!(upstream.forward_count(), 0);
        assert_eq!(sidecar.commit_count(), 0);
    }

    /// (3) Upstream error releases reservation with ProviderError.
    #[tokio::test]
    async fn upstream_error_releases() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::erroring("connection refused"));
        let forward = MitmForward::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cascade_request_bytes("gpt-4o", "hello");
        let err = forward.run(&bytes).await.unwrap_err();

        assert!(matches!(err, SessionError::Upstream(_)), "{err:?}");
        assert_eq!(sidecar.release_count(), 1);
        let releases = sidecar.release_calls();
        assert_eq!(releases[0].2, ReleaseReason::ProviderError);
    }

    /// (4) Envelope decode error → no sidecar call.
    #[tokio::test]
    async fn envelope_decode_error_skips_sidecar() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(0)));
        let forward = MitmForward::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bad = [0xffu8; 16];
        let err = forward.run(&bad).await.unwrap_err();
        assert!(matches!(err, SessionError::EnvelopeDecode(_)), "{err:?}");
        assert_eq!(sidecar.reserve_count(), 0);
    }

    /// (5) Empty model_name rejected pre-sidecar.
    #[tokio::test]
    async fn empty_model_rejected_pre_sidecar() {
        let cascade_req = CascadeRequest {
            messages: vec![CascadeMessage {
                role: "user".to_string(),
                content: "x".to_string(),
            }],
            model_name: String::new(),
            max_tokens: None,
            tool_declarations: vec![],
            workspace_id: None,
            cascade_wire_version: Some("cascade.v2.0".to_string()),
        };
        let mut bytes = Vec::new();
        cascade_req.encode(&mut bytes).unwrap();

        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(vec![]));
        let forward = MitmForward::new(Arc::clone(&sidecar), Arc::clone(&upstream));
        let err = forward.run(&bytes).await.unwrap_err();
        assert!(matches!(err, SessionError::EnvelopeDecode(_)));
        assert_eq!(sidecar.reserve_count(), 0);
    }

    /// (6) RequireApproval is unsupported.
    #[tokio::test]
    async fn require_approval_unsupported() {
        let sidecar = Arc::new(InMemorySidecar::new());
        sidecar.program_next_decision(SidecarDecision::RequireApproval {
            decision_id: "decision-approval".to_string(),
        });
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(0)));
        let forward = MitmForward::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cascade_request_bytes("gpt-4o", "hello");
        let err = forward.run(&bytes).await.unwrap_err();
        assert!(matches!(err, SessionError::UnsupportedDecision(_)));
        assert_eq!(upstream.forward_count(), 0);
    }

    /// (7) Translated request matches Cascade envelope.
    #[tokio::test]
    async fn translated_request_matches_envelope() {
        let sidecar = Arc::new(InMemorySidecar::new());
        let upstream = Arc::new(CountedUpstream::returning(streaming_response(3)));
        let forward = MitmForward::new(Arc::clone(&sidecar), Arc::clone(&upstream));

        let bytes = cascade_request_bytes("claude-3.5-sonnet", "be brief");
        let _ = forward.run(&bytes).await.unwrap();

        let forwarded = upstream.forwarded();
        assert_eq!(forwarded[0].model, "claude-3.5-sonnet");
        assert_eq!(forwarded[0].messages[0].content, "be brief");
        assert_eq!(forwarded[0].stream, Some(true));
    }

    /// (8) Release reason "windsurf_wire_version_unsupported".
    #[test]
    fn release_reason_unsupported_wire_version_string() {
        assert_eq!(
            ReleaseReason::UnsupportedWireVersion.as_str(),
            "windsurf_wire_version_unsupported"
        );
    }
}
