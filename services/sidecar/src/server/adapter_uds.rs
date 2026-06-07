//! Adapter UDS gRPC server (Sidecar §3 + §5; Stage 2 §11).
//!
//! Transport: gRPC over Unix Domain Socket. Auth: SO_PEERCRED (peer uid
//! match). POC accepts any local connection; production verifies that
//! `peer_uid == expected_app_uid` and `peer_pid` is in the same pod's
//! process group.
//!
//! Implemented RPCs:
//!   * Handshake — validates protocol version + capability advertisement.
//!   * RequestDecision — runs Contract §6 stages 1-4 via decision::transaction.
//!   * ConfirmPublishOutcome — POC ack-only (Codex round 1 P2.8: a single
//!     audit_outcome writer lives in EmitTraceEvents → CommitEstimated).
//!   * EmitTraceEvents — Phase 2B Step 7: routes LLM_CALL_POST.SUCCESS with
//!     `estimated_amount_atomic` to `decision::transaction::run_commit_estimated`
//!     (Stage 7 commit lane). ProviderReport + Release deferred (B2/A3).
//!   * StreamDrainSignal — emits drain phase events when sidecar marks draining.
//!
//! Round-2 #11: every gRPC method records a (handler, outcome) bucket
//! into `metrics: SidecarMetrics`. The trait impl methods are tiny
//! wrappers around `*_inner` impls on `AdapterUds` so the inner bodies
//! keep their existing early-return shape unchanged.
//!
//! Deferred to vertical slice expansion:
//!   * Sub-agent budget grant lifecycle
//!   * Sidecar announcement signature (HandshakeResponse.announcement_signature)

use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::{
    config::Config,
    decision::transaction::{self, DecisionContext},
    domain::{error::DomainError, state::SidecarState},
    metrics::{Handler, Outcome, SidecarMetrics},
    proto::sidecar_adapter::v1::{
        drain_signal::Phase as DrainPhase, sidecar_adapter_server::SidecarAdapter,
        ConsumeBudgetGrantRequest, ConsumeBudgetGrantResponse, DecisionRequest, DecisionResponse,
        DrainSignal, DrainSubscribeRequest, HandshakeRequest, HandshakeResponse,
        IssueBudgetGrantRequest, IssueBudgetGrantResponse, PublishOutcomeRequest,
        PublishOutcomeResponse, RevokeBudgetGrantRequest, RevokeBudgetGrantResponse, TraceEvent,
        TraceEventAck,
    },
};

pub struct AdapterUds {
    pub state: SidecarState,
    pub cfg: Config,
    /// Round-2 #11: handler call counters surfaced over /metrics.
    pub metrics: SidecarMetrics,
}

/// Round-2 #11: increment the (handler, outcome) bucket based on the
/// `Result<T, Status>` returned by the wrapped handler.
fn record_outcome<T>(metrics: &SidecarMetrics, handler: Handler, r: &Result<T, Status>) {
    let outcome = if r.is_ok() { Outcome::Ok } else { Outcome::Err };
    metrics.inc_handler(handler, outcome);
}

#[tonic::async_trait]
impl SidecarAdapter for AdapterUds {
    async fn handshake(
        &self,
        req: Request<HandshakeRequest>,
    ) -> Result<Response<HandshakeResponse>, Status> {
        let result = self.handshake_inner(req).await;
        record_outcome(&self.metrics, Handler::Handshake, &result);
        result
    }

    async fn request_decision(
        &self,
        req: Request<DecisionRequest>,
    ) -> Result<Response<DecisionResponse>, Status> {
        let result = self.request_decision_inner(req).await;
        record_outcome(&self.metrics, Handler::RequestDecision, &result);
        result
    }

    async fn confirm_publish_outcome(
        &self,
        req: Request<PublishOutcomeRequest>,
    ) -> Result<Response<PublishOutcomeResponse>, Status> {
        let result = self.confirm_publish_outcome_inner(req).await;
        record_outcome(&self.metrics, Handler::ConfirmPublishOutcome, &result);
        result
    }

    type EmitTraceEventsStream = ReceiverStream<Result<TraceEventAck, Status>>;

    async fn emit_trace_events(
        &self,
        stream: Request<tonic::Streaming<TraceEvent>>,
    ) -> Result<Response<Self::EmitTraceEventsStream>, Status> {
        let result = self.emit_trace_events_inner(stream).await;
        record_outcome(&self.metrics, Handler::EmitTraceEvents, &result);
        result
    }

    async fn issue_budget_grant(
        &self,
        _req: Request<IssueBudgetGrantRequest>,
    ) -> Result<Response<IssueBudgetGrantResponse>, Status> {
        self.metrics
            .inc_handler(Handler::IssueBudgetGrant, Outcome::Err);
        Err(Status::unimplemented(
            "IssueBudgetGrant: vertical slice expansion (Phase 1 sub-agent flow)",
        ))
    }
    async fn revoke_budget_grant(
        &self,
        _req: Request<RevokeBudgetGrantRequest>,
    ) -> Result<Response<RevokeBudgetGrantResponse>, Status> {
        self.metrics
            .inc_handler(Handler::RevokeBudgetGrant, Outcome::Err);
        Err(Status::unimplemented(
            "RevokeBudgetGrant: vertical slice expansion",
        ))
    }
    async fn consume_budget_grant(
        &self,
        _req: Request<ConsumeBudgetGrantRequest>,
    ) -> Result<Response<ConsumeBudgetGrantResponse>, Status> {
        self.metrics
            .inc_handler(Handler::ConsumeBudgetGrant, Outcome::Err);
        Err(Status::unimplemented(
            "ConsumeBudgetGrant: vertical slice expansion",
        ))
    }

    type StreamDrainSignalStream = ReceiverStream<Result<DrainSignal, Status>>;

    async fn stream_drain_signal(
        &self,
        req: Request<DrainSubscribeRequest>,
    ) -> Result<Response<Self::StreamDrainSignalStream>, Status> {
        let result = self.stream_drain_signal_inner(req).await;
        record_outcome(&self.metrics, Handler::StreamDrainSignal, &result);
        result
    }

    async fn resume_after_approval(
        &self,
        req: Request<crate::proto::sidecar_adapter::v1::ResumeAfterApprovalRequest>,
    ) -> Result<Response<crate::proto::sidecar_adapter::v1::ResumeAfterApprovalResponse>, Status>
    {
        let result = self.resume_after_approval_inner(req).await;
        record_outcome(&self.metrics, Handler::ResumeAfterApproval, &result);
        result
    }

    async fn release_reservation(
        &self,
        req: Request<crate::proto::sidecar_adapter::v1::ReleaseReservationRequest>,
    ) -> Result<Response<crate::proto::sidecar_adapter::v1::ReleaseReservationResponse>, Status>
    {
        let result = self.release_reservation_inner(req).await;
        record_outcome(&self.metrics, Handler::ReleaseReservation, &result);
        result
    }
}

impl AdapterUds {
    async fn handshake_inner(
        &self,
        req: Request<HandshakeRequest>,
    ) -> Result<Response<HandshakeResponse>, Status> {
        let req = req.into_inner();

        if req.protocol_version != 1 {
            return Err(Status::failed_precondition(format!(
                "protocol_version {} not supported (server speaks 1)",
                req.protocol_version
            )));
        }
        if req.tenant_id_assertion != self.cfg.tenant_id {
            return Err(Status::permission_denied(format!(
                "tenant_id_assertion '{}' does not match sidecar tenant '{}'",
                req.tenant_id_assertion, self.cfg.tenant_id
            )));
        }

        let bundle = self.state.inner.contract_bundle.read().clone();
        let schema = self.state.inner.schema_bundle.read().clone();
        let session_id = uuid::Uuid::now_v7().to_string();

        let response = HandshakeResponse {
            sidecar_version: env!("CARGO_PKG_VERSION").to_string(),
            schema_bundle: schema.map(|s| crate::proto::common::v1::SchemaBundleRef {
                schema_bundle_id: s.bundle_id.to_string(),
                schema_bundle_hash: s.bundle_hash.into(),
                canonical_schema_version: s.canonical_schema_version,
            }),
            contract_bundle: bundle.map(|b| crate::proto::common::v1::ContractBundleRef {
                bundle_id: b.bundle_id.to_string(),
                bundle_hash: b.bundle_hash.into(),
                bundle_signature: vec![].into(),
                signing_key_id: b.signing_key_id,
            }),
            capability_required: 0x40, // L3 (per Sidecar §3.3)
            active_key_epochs: Some(
                crate::proto::sidecar_adapter::v1::handshake_response::KeyEpochs {
                    producer_signing_key_epochs: vec!["epoch-2026-Q2".into()],
                    hmac_tenant_salt_epochs: vec!["epoch-2026-Q2".into()],
                },
            ),
            protocol_version: 1,
            session_id,
            // Announcement signing deferred to vertical slice; POC sidecars
            // emit empty signature. Adapter MUST treat empty as "POC mode".
            signing_key_id: String::new(),
            announcement_signature: vec![].into(),
        };
        Ok(Response::new(response))
    }

    async fn request_decision_inner(
        &self,
        req: Request<DecisionRequest>,
    ) -> Result<Response<DecisionResponse>, Status> {
        let req = req.into_inner();
        if self.state.is_draining() {
            return Err(DomainError::Draining.to_status());
        }

        let ctx = DecisionContext {
            session_id: req.session_id.clone(),
            workload_instance_id: self.cfg.workload_instance_id.clone(),
            tenant_id: self.cfg.tenant_id.clone(),
            region: self.cfg.region.clone(),
        };

        // Idempotency short-circuit (Contract §6). Adapter retries with the
        // same Idempotency.key MUST collapse to the same cached response;
        // otherwise sidecar mints a fresh decision_id per call and the
        // ledger sees a duplicate logical request. The fingerprint prevents
        // a reused key from replaying the previous decision for a different
        // request while the in-memory cache is still hot.
        let idempotency_key = req
            .idempotency
            .as_ref()
            .map(|i| i.key.clone())
            .unwrap_or_default();
        if idempotency_key.is_empty() {
            return Err(DomainError::InvalidRequest(
                "DecisionRequest.idempotency.key required".into(),
            )
            .to_status());
        }
        let request_fingerprint_hex = transaction::idempotency_request_fingerprint_hex(&ctx, &req);
        match self
            .state
            .inner
            .idempotency
            .get(&idempotency_key, &request_fingerprint_hex)
        {
            crate::decision::idempotency::Lookup::Hit(cached) => {
                tracing::debug!(key = %idempotency_key, "idempotent decision cache hit");
                return Ok(Response::new(cached));
            }
            crate::decision::idempotency::Lookup::Conflict {
                existing_fingerprint_hex,
            } => {
                return Err(DomainError::IdempotencyConflict(format!(
                    "DecisionRequest.idempotency.key reused with different request fingerprint (existing={}, current={})",
                    existing_fingerprint_hex, request_fingerprint_hex
                ))
                .to_status());
            }
            crate::decision::idempotency::Lookup::Miss => {}
        }

        crate::fencing::check_active(&self.state).map_err(|e| e.to_status())?;

        let out = transaction::run_through_reserve(&self.cfg, &self.state, &ctx, &req)
            .await
            .map_err(|e| e.to_status())?;

        let response = transaction::build_response(out);
        self.state.inner.idempotency.put(
            idempotency_key,
            request_fingerprint_hex,
            response.clone(),
        );
        Ok(Response::new(response))
    }

    async fn confirm_publish_outcome_inner(
        &self,
        req: Request<PublishOutcomeRequest>,
    ) -> Result<Response<PublishOutcomeResponse>, Status> {
        use crate::decision::transaction::{self, DecisionContext, ReleaseReason};
        use crate::proto::sidecar_adapter::v1::publish_outcome_request::Outcome as PoOutcome;

        let req = req.into_inner();
        let outcome = PoOutcome::try_from(req.outcome).unwrap_or(PoOutcome::Unspecified);

        // Phase 2B Step 7.5: APPLY_FAILED routes to release lane.
        // PublishOutcomeRequest carries only decision_id; sidecar maps
        // via decision_id_to_reservation index. Restart edge: index lost
        // → return typed POC limitation error; reservation TTL-releases.
        if outcome == PoOutcome::ApplyFailed {
            let decision_uuid = match uuid::Uuid::parse_str(&req.decision_id) {
                Ok(u) => u,
                Err(e) => {
                    return Ok(Response::new(PublishOutcomeResponse {
                        audit_outcome_event_id: String::new(),
                        recorded_at: Some(prost_types::Timestamp {
                            seconds: Utc::now().timestamp(),
                            nanos: Utc::now().timestamp_subsec_nanos() as i32,
                        }),
                        error: Some(crate::proto::common::v1::Error {
                            code: crate::proto::common::v1::error::Code::Unspecified as i32,
                            message: format!("decision_id parse: {e}"),
                            details: Default::default(),
                        }),
                    }));
                }
            };
            let reservation_id = self
                .state
                .inner
                .decision_id_to_reservation
                .lock()
                .get(&decision_uuid)
                .cloned();

            match reservation_id {
                Some(rid) => {
                    let dctx = DecisionContext {
                        session_id: req.session_id.clone(),
                        workload_instance_id: self.cfg.workload_instance_id.clone(),
                        tenant_id: self.cfg.tenant_id.clone(),
                        region: self.cfg.region.clone(),
                    };
                    let metadata = if req.adapter_error.is_empty() {
                        None
                    } else {
                        Some(req.adapter_error.as_str())
                    };
                    match transaction::run_release(
                        &self.cfg,
                        &self.state,
                        &dctx,
                        rid,
                        ReleaseReason::RuntimeError,
                        metadata,
                        None,
                        None,
                    )
                    .await
                    {
                        Ok(out) => {
                            info!(
                                decision_id = %req.decision_id,
                                ledger_tx = %out.ledger_transaction_id,
                                "ConfirmPublishOutcome.APPLY_FAILED → Release success"
                            );
                            return Ok(Response::new(PublishOutcomeResponse {
                                audit_outcome_event_id: out.ledger_transaction_id,
                                recorded_at: Some(prost_types::Timestamp {
                                    seconds: Utc::now().timestamp(),
                                    nanos: Utc::now().timestamp_subsec_nanos() as i32,
                                }),
                                error: None,
                            }));
                        }
                        Err(e) => {
                            warn!(error = %e, "Release failed for APPLY_FAILED path");
                            return Ok(Response::new(PublishOutcomeResponse {
                                audit_outcome_event_id: String::new(),
                                recorded_at: Some(prost_types::Timestamp {
                                    seconds: Utc::now().timestamp(),
                                    nanos: Utc::now().timestamp_subsec_nanos() as i32,
                                }),
                                error: Some(e.to_proto()),
                            }));
                        }
                    }
                }
                None => {
                    warn!(
                        decision_id = %req.decision_id,
                        "ConfirmPublishOutcome.APPLY_FAILED but decision_id_to_reservation index miss \
                         (POC limitation: sidecar restart loses map; reservation will TTL-release)"
                    );
                    return Ok(Response::new(PublishOutcomeResponse {
                        audit_outcome_event_id: String::new(),
                        recorded_at: Some(prost_types::Timestamp {
                            seconds: Utc::now().timestamp(),
                            nanos: Utc::now().timestamp_subsec_nanos() as i32,
                        }),
                        error: Some(crate::proto::common::v1::Error {
                            code: crate::proto::common::v1::error::Code::Unspecified as i32,
                            message: "reservation context lost (sidecar restart); reservation will TTL-release"
                                .into(),
                            details: Default::default(),
                        }),
                    }));
                }
            }
        }

        info!(
            decision_id = %req.decision_id,
            outcome = req.outcome,
            "publish outcome (POC: ack only for APPLIED/APPLIED_NOOP/APPROVAL_*)"
        );
        // Non-APPLY_FAILED outcomes: POC ack-only (Codex round 1 P2.8
        // single durable outcome writer = EmitTraceEvents/Release path).
        Ok(Response::new(PublishOutcomeResponse {
            audit_outcome_event_id: String::new(),
            recorded_at: Some(prost_types::Timestamp {
                seconds: Utc::now().timestamp(),
                nanos: Utc::now().timestamp_subsec_nanos() as i32,
            }),
            error: None,
        }))
    }

    async fn emit_trace_events_inner(
        &self,
        stream: Request<tonic::Streaming<TraceEvent>>,
    ) -> Result<Response<<Self as SidecarAdapter>::EmitTraceEventsStream>, Status> {
        let (tx, rx) = mpsc::channel::<Result<TraceEventAck, Status>>(8);
        let mut input = stream.into_inner();
        let state = self.state.clone();
        let cfg = self.cfg.clone();

        tokio::spawn(async move {
            use crate::decision::transaction::{self, DecisionContext, ReleaseReason};
            use crate::proto::sidecar_adapter::v1::{
                llm_call_post_payload::Outcome as LlmOutcome, trace_event::EventKind,
                trace_event_ack::Status as AckStatus,
            };
            while let Some(ev_res) = input.message().await.transpose() {
                let ev = match ev_res {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(error = %e, "EmitTraceEvents stream error");
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                };
                let event_id = uuid::Uuid::now_v7().to_string();
                let kind = EventKind::try_from(ev.kind).unwrap_or(EventKind::Unspecified);

                if kind != EventKind::LlmCallPost {
                    let _ = tx
                        .send(Ok(TraceEventAck {
                            event_id,
                            status: AckStatus::Accepted as i32,
                            error: None,
                        }))
                        .await;
                    continue;
                }

                let payload = match ev.payload {
                    Some(crate::proto::sidecar_adapter::v1::trace_event::Payload::LlmCallPost(
                        p,
                    )) => p,
                    _ => {
                        let _ = tx
                            .send(Ok(TraceEventAck {
                                event_id,
                                status: AckStatus::Rejected as i32,
                                error: Some(crate::proto::common::v1::Error {
                                    code: crate::proto::common::v1::error::Code::Unspecified as i32,
                                    message: "LLM_CALL_POST missing typed payload".into(),
                                    details: Default::default(),
                                }),
                            }))
                            .await;
                        continue;
                    }
                };

                let dctx = DecisionContext {
                    session_id: ev.session_id.clone(),
                    workload_instance_id: cfg.workload_instance_id.clone(),
                    tenant_id: cfg.tenant_id.clone(),
                    region: cfg.region.clone(),
                };

                // Phase 2B Step 7.5: split routing by outcome BEFORE
                // amount/commit validation (Codex P2.3 fix).
                let outcome =
                    LlmOutcome::try_from(payload.outcome).unwrap_or(LlmOutcome::Unspecified);
                let result: Result<String, crate::domain::error::DomainError> = match outcome {
                    LlmOutcome::Success => {
                        match transaction::run_commit_estimated(&cfg, &state, &dctx, &payload).await
                        {
                            Ok(out) => {
                                info!(
                                    reservation_id = %out.reservation_id,
                                    ledger_tx = %out.ledger_transaction_id,
                                    delta = %out.delta_to_reserved_atomic,
                                    "CommitEstimated success"
                                );
                                Ok(out.ledger_transaction_id)
                            }
                            Err(e) => Err(e),
                        }
                    }
                    LlmOutcome::ProviderError
                    | LlmOutcome::ClientTimeout
                    | LlmOutcome::RunAborted => {
                        let reason = if outcome == LlmOutcome::RunAborted {
                            ReleaseReason::RunAborted
                        } else {
                            ReleaseReason::RuntimeError
                        };
                        let reservation_uuid = match uuid::Uuid::parse_str(&payload.reservation_id)
                        {
                            Ok(u) => u,
                            Err(e) => {
                                let _ = tx
                                    .send(Ok(TraceEventAck {
                                        event_id,
                                        status: AckStatus::Rejected as i32,
                                        error: Some(crate::proto::common::v1::Error {
                                            code: crate::proto::common::v1::error::Code::Unspecified
                                                as i32,
                                            message: format!("reservation_id parse: {e}"),
                                            details: Default::default(),
                                        }),
                                    }))
                                    .await;
                                continue;
                            }
                        };
                        match transaction::run_release(
                            &cfg,
                            &state,
                            &dctx,
                            reservation_uuid,
                            reason,
                            if payload.provider_event_id.is_empty() {
                                None
                            } else {
                                Some(payload.provider_event_id.as_str())
                            },
                            None,
                            None,
                        )
                        .await
                        {
                            Ok(out) => {
                                info!(
                                    reservation_id = %reservation_uuid,
                                    ledger_tx = %out.ledger_transaction_id,
                                    reason = ?reason,
                                    "Release success"
                                );
                                Ok(out.ledger_transaction_id)
                            }
                            Err(e) => Err(e),
                        }
                    }
                    LlmOutcome::Unspecified => {
                        Err(crate::domain::error::DomainError::InvalidRequest(
                            "LLM_CALL_POST outcome=UNSPECIFIED".into(),
                        ))
                    }
                };

                match result {
                    Ok(_) => {
                        let _ = tx
                            .send(Ok(TraceEventAck {
                                event_id,
                                status: AckStatus::Accepted as i32,
                                error: None,
                            }))
                            .await;
                    }
                    Err(e) => {
                        warn!(error = %e, ?outcome, "LLM_CALL_POST routing rejected");
                        let _ = tx
                            .send(Ok(TraceEventAck {
                                event_id,
                                status: AckStatus::Rejected as i32,
                                error: Some(e.to_proto()),
                            }))
                            .await;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn stream_drain_signal_inner(
        &self,
        _req: Request<DrainSubscribeRequest>,
    ) -> Result<Response<<Self as SidecarAdapter>::StreamDrainSignalStream>, Status> {
        let (tx, rx) = mpsc::channel::<Result<DrainSignal, Status>>(4);
        let state = self.state.clone();
        tokio::spawn(async move {
            // Edge-trigger when state.draining transitions to true; POC
            // polls every 200ms.
            let mut last_phase: Option<DrainPhase> = None;
            loop {
                let phase = if state.is_draining() {
                    DrainPhase::DrainInitiated
                } else {
                    DrainPhase::Unspecified
                };
                if Some(phase) != last_phase && phase != DrainPhase::Unspecified {
                    let signal = DrainSignal {
                        phase: phase as i32,
                        deadline: None,
                        drain_trigger: "sigterm".into(),
                    };
                    if tx.send(Ok(signal)).await.is_err() {
                        return;
                    }
                    last_phase = Some(phase);
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });
        Ok(Response::new(ReceiverStream::new(rx)))
    }

    // --------------------------------------------------------------------
    // Phase 5 GA hardening S16 / Round-2 #9 part 2 PR 9c:
    // ResumeAfterApproval — gRPC wiring + state branching.
    // --------------------------------------------------------------------
    //
    // Flow:
    //   1. Call Ledger.GetApprovalForResume(approval_id, tenant_id)
    //   2. Branch on state:
    //      * approved + bundled_ledger_transaction_id non-empty →
    //        idempotent replay, return Continue with that tx id
    //      * approved + bundled_ledger_transaction_id empty →
    //        Pending implementation. The full path requires the
    //        producer-side post_approval_required_decision SP to
    //        capture decision_context_json + requested_effect_json
    //        in a shape this resume handler can rebuild a
    //        ReserveSetRequest from. Until that SP lands, return a
    //        typed [PRODUCER_SP_NOT_WIRED] error so SDK callers see
    //        a clear "still waiting on producer-side wiring" message.
    //      * denied → ResumeAfterApprovalDenied (approver fields
    //        deferred; GetApprovalForResume's response shape will
    //        grow them in a follow-up)
    //      * pending / expired / cancelled / other → typed Error
    async fn resume_after_approval_inner(
        &self,
        req: Request<crate::proto::sidecar_adapter::v1::ResumeAfterApprovalRequest>,
    ) -> Result<Response<crate::proto::sidecar_adapter::v1::ResumeAfterApprovalResponse>, Status>
    {
        use crate::proto::common::v1::error::Code as ProtoCode;
        use crate::proto::ledger::v1::{
            get_approval_for_resume_response::Outcome as GetOutcome, GetApprovalForResumeRequest,
        };
        use crate::proto::sidecar_adapter::v1::{
            decision_response::Decision, resume_after_approval_response::Outcome as ResumeOutcome,
            DecisionResponse, ResumeAfterApprovalDenied, ResumeAfterApprovalResponse,
        };

        let req = req.into_inner();
        tracing::info!(
            tenant = %req.tenant_id,
            decision_id = %req.decision_id,
            approval_id = %req.approval_id,
            "S16/9c: resume_after_approval invoked"
        );

        // Local helper: package a typed error into the response oneof.
        let into_err = |msg: String| {
            Ok(Response::new(ResumeAfterApprovalResponse {
                outcome: Some(ResumeOutcome::Error(crate::proto::common::v1::Error {
                    code: ProtoCode::Unspecified as i32,
                    message: msg,
                    details: Default::default(),
                })),
            }))
        };

        // 1) Fetch the approval row's resume context.
        let get_resp = match self
            .state
            .inner
            .ledger
            .get_approval_for_resume(GetApprovalForResumeRequest {
                approval_id: req.approval_id.clone(),
                tenant_id: req.tenant_id.clone(),
            })
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return into_err(format!("[LEDGER_RPC_FAILED] GetApprovalForResume: {e}"));
            }
        };
        let context = match get_resp.outcome {
            Some(GetOutcome::Context(c)) => c,
            Some(GetOutcome::Error(e)) => {
                return into_err(format!(
                    "[LEDGER_REJECTED] GetApprovalForResume: {}",
                    e.message
                ));
            }
            None => {
                return into_err(
                    "[LEDGER_RESPONSE_EMPTY] GetApprovalForResume returned no oneof".into(),
                );
            }
        };

        // 2) Branch on state.
        match context.state.as_str() {
            "approved" => {
                if !context.bundled_ledger_transaction_id.is_empty() {
                    // Idempotent replay: the approval has already
                    // been bundled; surface the existing tx without
                    // re-running ReserveSet.
                    tracing::info!(
                        approval_id = %req.approval_id,
                        ledger_tx = %context.bundled_ledger_transaction_id,
                        "S16/9c: idempotent replay — approval already bundled"
                    );
                    let decision_resp = DecisionResponse {
                        decision_id: context.decision_id.clone(),
                        audit_decision_event_id: String::new(),
                        decision: Decision::Continue as i32,
                        reason_codes: vec!["resume_idempotent_replay".into()],
                        matched_rule_ids: vec![],
                        mutation_patch_json: String::new(),
                        effect_hash: vec![].into(),
                        ledger_transaction_id: context.bundled_ledger_transaction_id.clone(),
                        reservation_ids: vec![],
                        ttl_expires_at: None,
                        approval_request_id: context.approval_id.clone(),
                        approval_ttl: None,
                        approver_role: String::new(),
                        terminal: false,
                        error: None,
                        // SLICE_02: resume path never emits RUN_*
                        // codes (those come from the projector at
                        // decision time; resume re-evaluates with
                        // post-approval state). Empty string per
                        // proto3 default.
                        run_code_triggered: String::new(),
                        // D13 COV_61: resume path is never the
                        // subscription-meter lane (approvals are a
                        // BYOK-only feature).
                        subscription_meter: None,
                    };
                    Ok(Response::new(ResumeAfterApprovalResponse {
                        outcome: Some(ResumeOutcome::Decision(decision_resp)),
                    }))
                } else {
                    // Round-2 #9 producer SP: parse the JSON payloads
                    // captured at REQUIRE_APPROVAL time, rebuild a
                    // fresh ReserveSetRequest, call Ledger.ReserveSet
                    // under a derived idempotency_key, then
                    // MarkApprovalBundled to atomically link the
                    // approval row → ledger transaction.
                    let parsed = match approval_resume_payload::parse(&context) {
                        Ok(p) => p,
                        Err(e) => {
                            return into_err(format!("[CONTEXT_PARSE_FAILED] {e}"));
                        }
                    };

                    let idem_key = {
                        use sha2::{Digest, Sha256};
                        let mut h = Sha256::new();
                        h.update(b"resume:");
                        h.update(req.approval_id.as_bytes());
                        hex::encode(h.finalize())
                    };

                    let reserve_req = match parsed
                        .into_reserve_set_request(
                            &self.cfg,
                            &self.state,
                            req.session_id.clone(),
                            req.workload_instance_id.clone(),
                            idem_key,
                        )
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            return into_err(format!("[RESUME_BUILD_FAILED] {e}"));
                        }
                    };

                    let reserve_resp = match self.state.inner.ledger.reserve_set(reserve_req).await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            return into_err(format!("[LEDGER_RPC_FAILED] ReserveSet: {e}"));
                        }
                    };

                    use crate::proto::ledger::v1::reserve_set_response::Outcome as ReserveOutcome;
                    let (tx_id, audit_event_id, reservation_ids) = match reserve_resp.outcome {
                        Some(ReserveOutcome::Success(s)) => (
                            s.ledger_transaction_id.clone(),
                            s.audit_decision_event_id.clone(),
                            s.reservations
                                .iter()
                                .map(|r| r.reservation_id.clone())
                                .collect::<Vec<_>>(),
                        ),
                        Some(ReserveOutcome::Replay(r)) => (
                            r.ledger_transaction_id.clone(),
                            r.audit_decision_event_id.clone(),
                            vec![],
                        ),
                        Some(ReserveOutcome::Error(e)) => {
                            return into_err(format!("[RESERVE_REJECTED] {}", e.message));
                        }
                        None => {
                            return into_err(
                                "[LEDGER_RESPONSE_EMPTY] ReserveSet returned no oneof".into(),
                            );
                        }
                    };

                    use crate::proto::ledger::v1::{
                        mark_approval_bundled_response::Outcome as MarkOutcome,
                        MarkApprovalBundledRequest,
                    };
                    let mark_resp = match self
                        .state
                        .inner
                        .ledger
                        .mark_approval_bundled(MarkApprovalBundledRequest {
                            approval_id: req.approval_id.clone(),
                            ledger_transaction_id: tx_id.clone(),
                        })
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            return into_err(format!(
                                "[LEDGER_RPC_FAILED] MarkApprovalBundled: {e}"
                            ));
                        }
                    };
                    if let Some(MarkOutcome::Error(e)) = mark_resp.outcome {
                        return into_err(format!("[BUNDLE_REJECTED] {}", e.message));
                    }

                    let decision_resp = DecisionResponse {
                        decision_id: context.decision_id.clone(),
                        audit_decision_event_id: audit_event_id,
                        decision: Decision::Continue as i32,
                        reason_codes: vec!["resume_approved".into()],
                        matched_rule_ids: vec![],
                        mutation_patch_json: String::new(),
                        effect_hash: vec![].into(),
                        ledger_transaction_id: tx_id,
                        reservation_ids,
                        ttl_expires_at: None,
                        approval_request_id: context.approval_id.clone(),
                        approval_ttl: None,
                        approver_role: String::new(),
                        terminal: false,
                        error: None,
                        // SLICE_02: see resume_idempotent_replay
                        // comment above — resume path does not emit
                        // RUN_* codes.
                        run_code_triggered: String::new(),
                        // D13 COV_61: resume_approved is BYOK-only.
                        subscription_meter: None,
                    };
                    Ok(Response::new(ResumeAfterApprovalResponse {
                        outcome: Some(ResumeOutcome::Decision(decision_resp)),
                    }))
                }
            }
            "denied" => {
                // Round-2 #9 part 2: approver fields (subject, reason,
                // matched_rule_ids) are not yet exposed by
                // GetApprovalForResume. Return Denied with empty
                // fields; SDK callers still raise typed
                // ApprovalDeniedError. A follow-up extends the proto
                // to surface approver_subject / approver_reason / etc.
                Ok(Response::new(ResumeAfterApprovalResponse {
                    outcome: Some(ResumeOutcome::Denied(ResumeAfterApprovalDenied {
                        audit_decision_event_id: String::new(),
                        approver_reason: String::new(),
                        approver_subject: String::new(),
                        matched_rule_ids: vec![],
                    })),
                }))
            }
            other => into_err(format!(
                "[APPROVAL_NON_TERMINAL] approval state={other:?} is not resumable"
            )),
        }
    }

    /// ReleaseReservation — explicit adapter-initiated release of a
    /// held reservation matching Agent Spend Protocol Draft-01 §4.
    ///
    /// Coexists with the implicit release paths (APPLY_FAILED in
    /// ConfirmPublishOutcome, run-aborted in EmitTraceEvents); this
    /// wrapper exposes the same `transaction::run_release` core under a
    /// dedicated RPC for ASP-conformant adapters. Implicit paths keep
    /// working unchanged.
    async fn release_reservation_inner(
        &self,
        req: Request<crate::proto::sidecar_adapter::v1::ReleaseReservationRequest>,
    ) -> Result<Response<crate::proto::sidecar_adapter::v1::ReleaseReservationResponse>, Status>
    {
        use crate::decision::transaction::{self, DecisionContext, ReleaseReason};
        use crate::proto::sidecar_adapter::v1::ReleaseReservationResponse;

        let req = req.into_inner();

        // Tenant assertion: per adapter.proto, the field MAY be empty
        // (defaults to the sidecar's own tenant_id). If non-empty MUST
        // match.
        if !req.tenant_id.is_empty() && req.tenant_id != self.cfg.tenant_id {
            return Err(Status::permission_denied(format!(
                "tenant_id '{}' does not match sidecar tenant '{}'",
                req.tenant_id, self.cfg.tenant_id
            )));
        }

        let reservation_uuid = uuid::Uuid::parse_str(&req.reservation_id).map_err(|e| {
            Status::invalid_argument(format!(
                "reservation_id '{}' is not a valid UUID: {e}",
                req.reservation_id
            ))
        })?;

        // Reason mapping per adapter.proto comment block on
        // ReleaseReservationRequest.reason_codes. Aligned with the
        // EmitTraceEvents implicit path so audit reason values are
        // consistent regardless of which release path the adapter took.
        // Unknown codes default to Explicit so adapter intent is
        // preserved in the audit metadata even when SpendGuard does
        // not recognize the code.
        let reason = req
            .reason_codes
            .iter()
            .find_map(|c| match c.as_str() {
                "provider_error" | "runtime_error" | "client_timeout" => {
                    Some(ReleaseReason::RuntimeError)
                }
                "run_aborted" | "run_cancelled" => Some(ReleaseReason::RunAborted),
                _ => None,
            })
            .unwrap_or(ReleaseReason::Explicit);

        // Do not preflight fencing here. `transaction::run_release`
        // enforces active fencing for first-time mutations after it
        // recovers reservation state, but lets an already-released
        // reservation reach the ledger idempotency-first replay branch.
        // A preflight check here would reject legitimate same-key
        // ReleaseReservation retries after local fencing movement before
        // the ledger can replay the original outcome (GH #86).

        // Honor the request's workload_instance_id when non-empty
        // (per adapter.proto field semantics — fencing parity with
        // reservations created under an adapter-supplied identity,
        // matching the ResumeAfterApproval workload override path).
        let workload_instance_id = if req.workload_instance_id.is_empty() {
            self.cfg.workload_instance_id.clone()
        } else {
            req.workload_instance_id.clone()
        };

        let dctx = DecisionContext {
            session_id: req.session_id.clone(),
            workload_instance_id,
            tenant_id: self.cfg.tenant_id.clone(),
            region: self.cfg.region.clone(),
        };

        // Preserve adapter intent in the audit chain: the joined
        // reason_codes string lands in the audit.outcome CloudEvent's
        // `metadata` field. Empty string when no codes provided.
        let metadata_owned = req.reason_codes.join(",");
        let metadata = if metadata_owned.is_empty() {
            None
        } else {
            Some(metadata_owned.as_str())
        };

        // Forward the adapter's idempotency key to the ledger so the
        // (reservation_id, idempotency_key) dedup contract documented
        // in adapter.proto holds end-to-end. Empty key falls back to
        // run_release's built-in `release:{uuid}:1` (legacy implicit-
        // path behavior — preserves replay semantics for adapters that
        // don't supply a key).
        let idempotency_override = if req.idempotency_key.is_empty() {
            None
        } else {
            Some(req.idempotency_key.as_str())
        };

        // request_body_hash: explicit RPC passes None for v1.
        //
        // Sending a non-empty hash would require matching the ledger's
        // private canonical_request_hash function (tenant +
        // reservation_set + decision + reason — see services/ledger/
        // src/handlers/release.rs); any other hash makes the ledger
        // reject every first-time release as IdempotencyConflict.
        //
        // Known limitation tracked separately: with empty hash, a
        // retry that reuses the same idempotency_key but supplies
        // different reason_codes will REPLAY the first outcome
        // instead of returning REPLAY_CONFLICT. Acceptable for v1
        // since the documented adapter pattern derives a stable
        // idempotency_key per (reservation_id) and varying reason
        // codes for the same key is a programming error.
        match transaction::run_release(
            &self.cfg,
            &self.state,
            &dctx,
            reservation_uuid,
            reason,
            metadata,
            idempotency_override,
            None,
        )
        .await
        {
            Ok(out) => {
                info!(
                    reservation_id = %req.reservation_id,
                    idempotency_key = %req.idempotency_key,
                    ledger_tx = %out.ledger_transaction_id,
                    reason = ?reason,
                    "ReleaseReservation success"
                );
                Ok(Response::new(ReleaseReservationResponse {
                    audit_event_signature: out.audit_event_signature.into(),
                    ledger_transaction_id: out.ledger_transaction_id,
                    released_reservation_ids: out.released_reservation_ids,
                }))
            }
            Err(e) => {
                warn!(
                    reservation_id = %req.reservation_id,
                    idempotency_key = %req.idempotency_key,
                    error = %e,
                    "ReleaseReservation failed"
                );
                // Errors surface via gRPC Status (standard tonic / Draft-01
                // idiom) rather than in the response body — see the
                // error-mapping comment block in adapter.proto on
                // ReleaseReservationResponse.
                Err(e.to_status())
            }
        }
    }
}

/// Round-2 #9 producer SP: shape definitions for `decision_context`
/// + `requested_effect` JSON blobs in `approval_requests`. Producer
/// side (post_approval_required_decision SP via
/// services/sidecar/src/decision/transaction.rs::run_record_denied_decision)
/// MUST write the same shape. The migration 0037 header comment
/// pins the contract on the SQL side.
mod approval_resume_payload {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct DecisionContext {
        pub tenant_id: String,
        pub budget_id: String,
        pub window_instance_id: String,
        pub fencing_scope_id: String,
        pub fencing_epoch: u64,
        pub decision_id: String,
        #[serde(default)]
        pub matched_rule_ids: Vec<String>,
        #[serde(default)]
        pub reason_codes: Vec<String>,
        pub contract_bundle_id: String,
        pub contract_bundle_hash_hex: String,
        #[serde(default)]
        pub schema_bundle_id: String,
        #[serde(default)]
        pub schema_bundle_canonical_version: String,
        // Issue #59 — frozen-at-PRE pricing. Captured at REQUIRE_APPROVAL
        // time in services/sidecar/src/decision/transaction.rs. Reused
        // here (not re-read from the live bundle) so the resume's
        // PricingFreeze matches what the operator approved.
        #[serde(default)]
        pub pricing_version: String,
        #[serde(default)]
        pub price_snapshot_hash_hex: String,
        #[serde(default)]
        pub fx_rate_version: String,
        #[serde(default)]
        pub unit_conversion_version: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct RequestedEffect {
        pub unit_id: String,
        pub unit_kind: String,
        #[serde(default)]
        pub unit_token_kind: String,
        pub amount_atomic: String,
        pub direction: String,
    }

    pub struct Parsed {
        pub decision: DecisionContext,
        pub effect: RequestedEffect,
    }

    pub fn parse(ctx: &crate::proto::ledger::v1::ApprovalResumeContext) -> Result<Parsed, String> {
        let decision: DecisionContext = serde_json::from_slice(&ctx.decision_context_json)
            .map_err(|e| format!("decision_context_json: {e}"))?;
        let effect: RequestedEffect = serde_json::from_slice(&ctx.requested_effect_json)
            .map_err(|e| format!("requested_effect_json: {e}"))?;
        Ok(Parsed { decision, effect })
    }

    /// Resolve the `prediction_policy_used` wire value
    /// (CloudEvent tag 305) for a resume-after-approval
    /// `spendguard.audit.decision` event.
    ///
    /// SLICE_02 round-1 B3 invariant
    /// ----------------------------
    ///
    /// Per `docs/audit-chain-prediction-extension-v1alpha1.md` §2.1 line 122,
    /// `prediction_policy_used` is NOT NULL on every `.decision` event past
    /// the 2027-01-01 cutoff and is CHECKed to be in
    /// (`STRICT_CEILING` | `EMPIRICAL_RUN_CEILING` | `ADAPTIVE_CEILING` |
    /// `SHADOW_ONLY`). The resume-after-approval path mints a fresh
    /// `spendguard.audit.decision` event (same type as CONTINUE / DENY) so
    /// the same column constraint applies; leaving the field at proto3
    /// default would silently violate the CHECK at the mirror persistence
    /// boundary.
    ///
    /// Source of truth: the live bundle's parsed contract. The caller
    /// (`Parsed::into_reserve_set_request`) MUST run the bundle-hash
    /// hot-reload guard BEFORE invoking this helper so the policy value
    /// reflects the operator's approved bundle.
    pub(super) fn resume_audit_decision_policy_field(
        live_bundle: &crate::domain::state::CachedContractBundle,
    ) -> String {
        // v1alpha1 contracts default-fill to STRICT_CEILING at parse time
        // (parse.rs §6.4 default-fill block); v1alpha2 contracts carry
        // the operator-declared value. Either way the enum's `as_str()`
        // yields the exact wire token the CHECK constraint accepts.
        live_bundle.parsed.prediction_policy.as_str().to_string()
    }

    pub(super) fn apply_resume_audit_decision_policy_field(
        cloudevent: &mut crate::proto::common::v1::CloudEvent,
        live_bundle: &crate::domain::state::CachedContractBundle,
    ) {
        cloudevent.prediction_policy_used = resume_audit_decision_policy_field(live_bundle);
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::contract::types::{Contract, PredictionPolicy};
        use crate::domain::state::CachedContractBundle;
        use crate::proto::common::v1::CloudEvent;
        use std::sync::Arc;
        use uuid::Uuid;

        fn _fixture_bundle(policy: PredictionPolicy, api_version: &str) -> CachedContractBundle {
            let parsed = Contract {
                id: Uuid::nil(),
                name: "fixture".into(),
                budgets: vec![],
                rules: vec![],
                prediction_policy: policy,
                api_version: api_version.into(),
            };
            CachedContractBundle {
                bundle_id: Uuid::nil(),
                bundle_hash: vec![0u8; 32],
                signing_key_id: "test-key".into(),
                raw: vec![],
                pricing_version: "v1".into(),
                price_snapshot_hash: vec![0u8; 32],
                fx_rate_version: "v1".into(),
                unit_conversion_version: "v1".into(),
                parsed: Arc::new(parsed),
            }
        }

        #[test]
        fn resume_policy_field_strict_ceiling() {
            // The v1alpha1 default-fill path (parse.rs §6.4) yields
            // STRICT_CEILING for every v1alpha1 contract; resume CloudEvent
            // must emit the literal token the CHECK constraint accepts.
            let b = _fixture_bundle(PredictionPolicy::StrictCeiling, "spendguard.ai/v1alpha1");
            assert_eq!(resume_audit_decision_policy_field(&b), "STRICT_CEILING");
        }

        #[test]
        fn resume_policy_field_empirical_run_ceiling() {
            // v1alpha2 contract with operator-declared
            // EMPIRICAL_RUN_CEILING — the resume CloudEvent must
            // carry the value the operator approved under.
            let b = _fixture_bundle(
                PredictionPolicy::EmpiricalRunCeiling,
                "spendguard.ai/v1alpha2",
            );
            assert_eq!(
                resume_audit_decision_policy_field(&b),
                "EMPIRICAL_RUN_CEILING"
            );
        }

        #[test]
        fn resume_policy_field_adaptive_ceiling() {
            let b = _fixture_bundle(PredictionPolicy::AdaptiveCeiling, "spendguard.ai/v1alpha2");
            assert_eq!(resume_audit_decision_policy_field(&b), "ADAPTIVE_CEILING");
        }

        #[test]
        fn resume_policy_field_shadow_only() {
            let b = _fixture_bundle(PredictionPolicy::ShadowOnly, "spendguard.ai/v1alpha2");
            assert_eq!(resume_audit_decision_policy_field(&b), "SHADOW_ONLY");
        }

        #[test]
        fn resume_policy_field_never_empty_string() {
            // Regression guard for the pre-round-1 behavior where the
            // CloudEvent was built with `..Default::default()` and the
            // `prediction_policy_used` field stayed at proto3-default
            // empty string. The CHECK constraint per
            // audit-chain-prediction-extension-v1alpha1.md §2.1 forbids
            // empty string on `.decision` events past 2027-01-01, so
            // every PredictionPolicy variant must produce a non-empty
            // CHECK-accepted token.
            for policy in [
                PredictionPolicy::StrictCeiling,
                PredictionPolicy::EmpiricalRunCeiling,
                PredictionPolicy::AdaptiveCeiling,
                PredictionPolicy::ShadowOnly,
            ] {
                let b = _fixture_bundle(policy, "spendguard.ai/v1alpha2");
                let s = resume_audit_decision_policy_field(&b);
                assert!(!s.is_empty(), "policy {:?} produced empty string", policy);
                // Cross-check that the result is one of the four CHECK-accepted
                // tokens. Pre-round-1 this would have been "" (proto3 default).
                assert!(
                    matches!(
                        s.as_str(),
                        "STRICT_CEILING"
                            | "EMPIRICAL_RUN_CEILING"
                            | "ADAPTIVE_CEILING"
                            | "SHADOW_ONLY"
                    ),
                    "unexpected policy wire token: {}",
                    s
                );
            }
        }

        #[test]
        fn resume_emit_helper_populates_cloud_event_policy_field() {
            let b = _fixture_bundle(PredictionPolicy::AdaptiveCeiling, "spendguard.ai/v1alpha2");
            let mut ce = CloudEvent {
                r#type: "spendguard.audit.decision".into(),
                ..Default::default()
            };

            apply_resume_audit_decision_policy_field(&mut ce, &b);

            assert_eq!(ce.prediction_policy_used, "ADAPTIVE_CEILING");
        }
    }

    impl Parsed {
        /// Build a fresh ReserveSetRequest from the parsed payloads.
        /// Mints a new decision_id + audit_decision_event_id (the
        /// resume is a logically new transaction; idempotency_key
        /// derived from approval_id collapses retries). Signs a
        /// CloudEvent in place via the sidecar's producer signer.
        pub async fn into_reserve_set_request(
            self,
            cfg: &crate::config::Config,
            state: &crate::domain::state::SidecarState,
            _session_id: String,
            workload_instance_id: String,
            idempotency_key: String,
        ) -> Result<crate::proto::ledger::v1::ReserveSetRequest, String> {
            use crate::proto::common::v1::{
                budget_claim::Direction, unit_ref::Kind as UnitKind, BudgetClaim, CloudEvent,
                ContractBundleRef, Fencing, Idempotency, PricingFreeze, UnitRef,
            };
            use chrono::Utc;
            use num_bigint::BigInt;
            use std::str::FromStr;
            use uuid::Uuid;

            // Parse + validate amount.
            BigInt::from_str(&self.effect.amount_atomic)
                .map_err(|e| format!("amount_atomic parse: {e}"))?;

            let unit_kind = match self.effect.unit_kind.as_str() {
                "MONETARY" => UnitKind::Monetary,
                "TOKEN" => UnitKind::Token,
                "CREDIT" => UnitKind::Credit,
                "NON_MONETARY" => UnitKind::NonMonetary,
                _ => UnitKind::Unspecified,
            };
            let direction = match self.effect.direction.as_str() {
                "CREDIT" => Direction::Credit,
                _ => Direction::Debit,
            };

            let workload_id = if workload_instance_id.is_empty() {
                cfg.workload_instance_id.clone()
            } else {
                workload_instance_id
            };

            let claim = BudgetClaim {
                budget_id: self.decision.budget_id.clone(),
                unit: Some(UnitRef {
                    unit_id: self.effect.unit_id.clone(),
                    kind: unit_kind as i32,
                    currency: String::new(),
                    unit_name: String::new(),
                    token_kind: self.effect.unit_token_kind.clone(),
                    model_family: String::new(),
                    credit_program: String::new(),
                }),
                amount_atomic: self.effect.amount_atomic.clone(),
                direction: direction as i32,
                window_instance_id: self.decision.window_instance_id.clone(),
            };

            // Issue #59 — frozen-at-PRE pricing + bundle-hash hot-reload check.
            //
            // Spec: docs/specs/issue-59-approval-resume-frozen-pricing.md §3.2.
            //
            // 1. Verify the sidecar's currently-installed bundle matches the
            //    bundle the operator approved against. If a hot-reload fired
            //    between approval and resume, the semantic basis for the
            //    operator's approval is stale. Return a typed error so the
            //    SDK can surface ApprovalBundleHotReloadedError and the
            //    caller can re-issue the original DecisionRequest.
            //
            // 2. Reconstruct PricingFreeze from decision_context_json (frozen
            //    at REQUIRE_APPROVAL emit time), NOT from the live bundle.
            //    Preserves the audit-chain invariant that pricing visible to
            //    the approver is the pricing used for the bundled
            //    reservation.
            //
            // SLICE_02 round-1 B3: read the live bundle BEFORE building the
            // resume CloudEvent so we can populate the new
            // `prediction_policy_used` audit column (CloudEvent tag 305).
            // Spec: docs/audit-chain-prediction-extension-v1alpha1.md §2.1
            // line 122 — `prediction_policy_used` is NOT NULL on
            // `.decision` events past the 2027-01-01 cutoff with a
            // CHECK IN ('STRICT_CEILING', 'EMPIRICAL_RUN_CEILING',
            // 'ADAPTIVE_CEILING', 'SHADOW_ONLY') constraint. The
            // resume-after-approval event is a `spendguard.audit.decision`
            // type (same as CONTINUE / DENY lanes) so the same column
            // invariant applies. Leaving the field at proto3 default
            // (empty string) would silently violate the CHECK at the
            // mirror persistence boundary.
            let live_bundle = state
                .inner
                .contract_bundle
                .read()
                .clone()
                .ok_or_else(|| "no contract bundle installed".to_string())?;
            let live_hash_hex = hex::encode(&live_bundle.bundle_hash);
            if !self.decision.contract_bundle_hash_hex.is_empty()
                && self.decision.contract_bundle_hash_hex != live_hash_hex
            {
                return Err(format!(
                    "[BUNDLE_HOT_RELOADED] approval was issued under bundle hash {} but the sidecar's currently-installed bundle is {}; the operator's approval is no longer semantically tied to this bundle. Reissue the original DecisionRequest to get a fresh approval row tied to the new bundle.",
                    self.decision.contract_bundle_hash_hex, live_hash_hex
                ));
            }

            // Mint new audit identifiers for the resume tx; the
            // resume is a logically new write to the ledger.
            let decision_id = Uuid::now_v7();
            let audit_decision_event_id = Uuid::now_v7();
            let producer_sequence = state.next_producer_sequence();

            // Build + sign a fresh CloudEvent (Contract §6.1: every
            // ledger write produces exactly one audit row).
            let payload = serde_json::json!({
                "resume_of_approval_id": self.decision.decision_id,
                "amount_atomic":         self.effect.amount_atomic,
                "budget_id":             self.decision.budget_id,
                "matched_rule_ids":      self.decision.matched_rule_ids,
                "reason_codes":          self.decision.reason_codes,
            });
            let payload_bytes = serde_json::to_vec(&payload)
                .map_err(|e| format!("cloudevent payload encode: {e}"))?;
            let mut cloudevent = CloudEvent {
                specversion: "1.0".into(),
                r#type: "spendguard.audit.decision".into(),
                source: format!("sidecar://{}/{}", cfg.region, workload_id),
                id: audit_decision_event_id.to_string(),
                time: Some(prost_types::Timestamp {
                    seconds: Utc::now().timestamp(),
                    nanos: Utc::now().timestamp_subsec_nanos() as i32,
                }),
                datacontenttype: "application/json".into(),
                data: payload_bytes.into(),
                tenant_id: self.decision.tenant_id.clone(),
                // Cost Advisor P0.5 — resume path enrichment is
                // DEFERRED. The CONTINUE + DENY emissions in
                // services/sidecar/src/decision/transaction.rs already
                // carry run_id / agent_id / model_family / prompt_hash;
                // the resume path mints a NEW decision_id and has no
                // DecisionRequest in scope, so source fields must come
                // from approval_requests.decision_context JSONB.
                // Threading them through requires post_approval_
                // required_decision SP (migration 0037) to persist
                // the original enrichment alongside the proposal —
                // open as a P3.5 follow-on. For now this emission
                // stays sparse; cost_advisor's rules group by original
                // decision_id (not the resume decision_id) so the
                // gap doesn't break run-scope dedup.
                run_id: String::new(),
                decision_id: decision_id.to_string(),
                schema_bundle_id: self.decision.schema_bundle_id.clone(),
                producer_id: format!("sidecar:{}", workload_id),
                producer_sequence,
                producer_signature: vec![].into(),
                signing_key_id: String::new(),
                ..Default::default()
            };
            // SLICE_02 round-1 B3: populate prediction_policy_used (tag 305)
            // from the live bundle's parsed contract — same source-of-truth
            // as the CONTINUE / DENY paths in
            // services/sidecar/src/decision/transaction.rs. The
            // bundle-hash hot-reload guard above ensures the live bundle
            // matches the operator's approved bundle, so the policy value
            // we emit here is the policy the operator approved under.
            apply_resume_audit_decision_policy_field(&mut cloudevent, &live_bundle);
            crate::audit::sign_cloudevent_in_place(&*state.inner.signer, &mut cloudevent)
                .await
                .map_err(|e| format!("sign resume cloudevent: {e}"))?;

            let price_snapshot_hash =
                hex::decode(&self.decision.price_snapshot_hash_hex).map_err(|e| {
                    format!(
                        "decision_context.price_snapshot_hash_hex decode: {e} (value was {:?})",
                        self.decision.price_snapshot_hash_hex
                    )
                })?;
            let pricing = PricingFreeze {
                pricing_version: self.decision.pricing_version.clone(),
                price_snapshot_hash: price_snapshot_hash.into(),
                fx_rate_version: self.decision.fx_rate_version.clone(),
                unit_conversion_version: self.decision.unit_conversion_version.clone(),
            };

            // TTL: reuse sidecar's configured reservation TTL.
            let ttl_expires_at = prost_types::Timestamp {
                seconds: (Utc::now()
                    + chrono::Duration::seconds(state.inner.reservation_ttl_seconds))
                .timestamp(),
                nanos: 0,
            };

            let bundle_hash =
                hex::decode(&self.decision.contract_bundle_hash_hex).unwrap_or_default();

            Ok(crate::proto::ledger::v1::ReserveSetRequest {
                tenant_id: self.decision.tenant_id,
                decision_id: decision_id.to_string(),
                audit_decision_event_id: audit_decision_event_id.to_string(),
                producer_sequence,
                idempotency: Some(Idempotency {
                    key: idempotency_key,
                    request_hash: Vec::new().into(),
                }),
                fencing: Some(Fencing {
                    epoch: self.decision.fencing_epoch,
                    scope_id: self.decision.fencing_scope_id,
                    workload_instance_id: workload_id,
                }),
                claims: vec![claim],
                lock_order_token: None,
                ttl_expires_at: Some(ttl_expires_at),
                audit_event: Some(cloudevent),
                pricing: Some(pricing),
                contract_bundle: Some(ContractBundleRef {
                    bundle_id: self.decision.contract_bundle_id,
                    bundle_hash: bundle_hash.into(),
                    bundle_signature: vec![].into(),
                    signing_key_id: String::new(),
                }),
            })
        }
    }
}

/// Build a tower stack for the UDS-bound gRPC server.
pub fn make_service(state: SidecarState, cfg: Config, metrics: SidecarMetrics) -> AdapterUds {
    AdapterUds {
        state,
        cfg,
        metrics,
    }
}

#[cfg(test)]
mod post_ga_01_release_tests {
    use super::*;
    use crate::{
        clients::{canonical_ingest::CanonicalIngestClient, ledger::LedgerClient},
        decision::idempotency::IdempotencyCache,
        domain::state::SidecarState,
        proto::{
            common::v1::{PricingFreeze, Replay, UnitRef},
            ledger::v1::{
                ledger_server::{Ledger, LedgerServer},
                AcquireFencingLeaseRequest, AcquireFencingLeaseResponse, CommitEstimatedRequest,
                CommitEstimatedResponse, CompensateRequest, CompensateResponse,
                DisputeAdjustmentRequest, DisputeAdjustmentResponse, GetApprovalForResumeRequest,
                GetApprovalForResumeResponse, InvoiceReconcileRequest, InvoiceReconcileResponse,
                MarkApprovalBundledRequest, MarkApprovalBundledResponse, ProviderReportRequest,
                ProviderReportResponse, QueryBudgetStateRequest, QueryBudgetStateResponse,
                QueryDecisionOutcomeRequest, QueryDecisionOutcomeResponse,
                QueryReservationContextRequest, QueryReservationContextResponse,
                RecordDeniedDecisionRequest, RecordDeniedDecisionResponse, RefundCreditRequest,
                RefundCreditResponse, ReleaseRequest, ReleaseResponse, ReleaseSuccess,
                ReplayAuditEvent, ReplayAuditFromCursorRequest, ReservationContext,
                ReserveSetRequest, ReserveSetResponse,
            },
            sidecar_adapter::v1::ReleaseReservationRequest,
        },
    };
    use ed25519_dalek::SigningKey;
    use spendguard_policy::FailPolicyMatrix;
    use spendguard_signing::LocalEd25519Signer;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::transport::{Endpoint, Server};
    use uuid::Uuid;

    #[derive(Clone)]
    struct FakeReleaseLedger {
        tenant_id: String,
        reservation_id: Uuid,
        decision_id: Uuid,
        fencing_scope_id: Uuid,
        release_calls: Arc<AtomicUsize>,
        audit_signatures_seen: Arc<parking_lot::Mutex<Vec<Vec<u8>>>>,
    }

    impl FakeReleaseLedger {
        fn reservation_context(&self) -> ReservationContext {
            let released = self.release_calls.load(Ordering::SeqCst) > 0;
            ReservationContext {
                reservation_id: self.reservation_id.to_string(),
                budget_id: Uuid::new_v4().to_string(),
                window_instance_id: Uuid::new_v4().to_string(),
                unit: Some(UnitRef {
                    unit_id: Uuid::new_v4().to_string(),
                    ..Default::default()
                }),
                original_reserved_amount_atomic: "100".into(),
                pricing: Some(PricingFreeze {
                    pricing_version: "test-pricing".into(),
                    price_snapshot_hash: vec![7; 32].into(),
                    fx_rate_version: "fx-test".into(),
                    unit_conversion_version: "unit-test".into(),
                }),
                fencing_scope_id: self.fencing_scope_id.to_string(),
                fencing_epoch_at_post: 7,
                decision_id: self.decision_id.to_string(),
                ttl_expires_at: Some(prost_types::Timestamp {
                    seconds: (Utc::now() + chrono::Duration::seconds(600)).timestamp(),
                    nanos: 0,
                }),
                current_state: if released { "released" } else { "reserved" }.into(),
                source_ledger_transaction_id: "tx-release-1".into(),
                tenant_id: self.tenant_id.clone(),
            }
        }
    }

    #[tonic::async_trait]
    impl Ledger for FakeReleaseLedger {
        type ReplayAuditFromCursorStream = ReceiverStream<Result<ReplayAuditEvent, Status>>;

        async fn reserve_set(
            &self,
            _req: Request<ReserveSetRequest>,
        ) -> Result<Response<ReserveSetResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn record_denied_decision(
            &self,
            _req: Request<RecordDeniedDecisionRequest>,
        ) -> Result<Response<RecordDeniedDecisionResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn acquire_fencing_lease(
            &self,
            _req: Request<AcquireFencingLeaseRequest>,
        ) -> Result<Response<AcquireFencingLeaseResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn release(
            &self,
            req: Request<ReleaseRequest>,
        ) -> Result<Response<ReleaseResponse>, Status> {
            let req = req.into_inner();
            let signature = req
                .audit_event
                .as_ref()
                .map(|event| event.producer_signature.to_vec())
                .unwrap_or_default();
            self.audit_signatures_seen.lock().push(signature);

            let call = self.release_calls.fetch_add(1, Ordering::SeqCst);
            let outcome = if call == 0 {
                Some(
                    crate::proto::ledger::v1::release_response::Outcome::Success(ReleaseSuccess {
                        ledger_transaction_id: "tx-release-1".into(),
                        released_reservation_ids: vec![self.reservation_id.to_string()],
                        recorded_at: Some(prost_types::Timestamp {
                            seconds: Utc::now().timestamp(),
                            nanos: 0,
                        }),
                    }),
                )
            } else {
                Some(crate::proto::ledger::v1::release_response::Outcome::Replay(
                    Replay {
                        ledger_transaction_id: "tx-release-1".into(),
                        operation_kind: "release".into(),
                        operation_id: self.reservation_id.to_string(),
                        decision_id: self.decision_id.to_string(),
                        ..Default::default()
                    },
                ))
            };

            Ok(Response::new(ReleaseResponse { outcome }))
        }

        async fn commit_estimated(
            &self,
            _req: Request<CommitEstimatedRequest>,
        ) -> Result<Response<CommitEstimatedResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn provider_report(
            &self,
            _req: Request<ProviderReportRequest>,
        ) -> Result<Response<ProviderReportResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn invoice_reconcile(
            &self,
            _req: Request<InvoiceReconcileRequest>,
        ) -> Result<Response<InvoiceReconcileResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn refund_credit(
            &self,
            _req: Request<RefundCreditRequest>,
        ) -> Result<Response<RefundCreditResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn dispute_adjustment(
            &self,
            _req: Request<DisputeAdjustmentRequest>,
        ) -> Result<Response<DisputeAdjustmentResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn compensate(
            &self,
            _req: Request<CompensateRequest>,
        ) -> Result<Response<CompensateResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn query_budget_state(
            &self,
            _req: Request<QueryBudgetStateRequest>,
        ) -> Result<Response<QueryBudgetStateResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn query_reservation_context(
            &self,
            _req: Request<QueryReservationContextRequest>,
        ) -> Result<Response<QueryReservationContextResponse>, Status> {
            Ok(Response::new(QueryReservationContextResponse {
                outcome: Some(
                    crate::proto::ledger::v1::query_reservation_context_response::Outcome::Context(
                        self.reservation_context(),
                    ),
                ),
            }))
        }

        async fn replay_audit_from_cursor(
            &self,
            _req: Request<ReplayAuditFromCursorRequest>,
        ) -> Result<Response<Self::ReplayAuditFromCursorStream>, Status> {
            let (_tx, rx) = mpsc::channel(1);
            Ok(Response::new(ReceiverStream::new(rx)))
        }

        async fn query_decision_outcome(
            &self,
            _req: Request<QueryDecisionOutcomeRequest>,
        ) -> Result<Response<QueryDecisionOutcomeResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn get_approval_for_resume(
            &self,
            _req: Request<GetApprovalForResumeRequest>,
        ) -> Result<Response<GetApprovalForResumeResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn mark_approval_bundled(
            &self,
            _req: Request<MarkApprovalBundledRequest>,
        ) -> Result<Response<MarkApprovalBundledResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }
    }

    fn test_config(tenant_id: &str, fencing_scope_id: Uuid) -> Config {
        Config {
            uds_path: "/tmp/spendguard-test.sock".into(),
            tenant_id: tenant_id.into(),
            workload_instance_id: "workload-a".into(),
            region: "test-region".into(),
            endpoint_catalog_manifest_url: String::new(),
            trust_root_ca_pem: String::new(),
            trust_root_spki_sha256_hex: String::new(),
            mtls_bootstrap_token: String::new(),
            capability_level: "L3_POLICY_HOOK".into(),
            enforcement_strength: "semantic_adapter".into(),
            manifest_pull_seconds: 60,
            critical_max_stale_seconds: 300,
            drain_window_seconds: 60,
            decision_p99_ms: 50,
            run_cost_projector_url: String::new(),
            run_cost_projector_timeout_ms: 25,
            allow_untrusted_budget_metadata: false,
            metrics_addr: "127.0.0.1:0".into(),
            health_addr: "127.0.0.1:0".into(),
            bundle_root: String::new(),
            contract_bundle_id: Uuid::nil().to_string(),
            contract_bundle_hash_hex: "00".repeat(32),
            schema_bundle_id: Uuid::nil().to_string(),
            schema_bundle_canonical_version: "spendguard.v1alpha1".into(),
            fencing_scope_id: fencing_scope_id.to_string(),
            fencing_initial_epoch: 7,
            fencing_ttl_seconds: 120,
            idempotency_cache_size: 16,
            idempotency_cache_ttl_secs: 600,
            reservation_ttl_seconds: 600,
            runtime_env_path: String::new(),
            hot_reload_poll_ms: 0,
            http_companion_port: 0,
            http_companion_host: "127.0.0.1".into(),
            http_companion_allow_pod_network: false,
            http_companion_max_body_bytes: 4 * 1024 * 1024,
        }
    }

    async fn test_service(
        fake_ledger: FakeReleaseLedger,
    ) -> (AdapterUds, tokio::task::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test ledger");
        let addr = listener.local_addr().expect("local addr");
        drop(listener);

        let server_ledger = fake_ledger.clone();
        let server = tokio::spawn(async move {
            Server::builder()
                .add_service(LedgerServer::new(server_ledger))
                .serve(addr)
                .await
                .expect("test ledger server");
        });
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;

        let channel = Endpoint::from_shared(format!("http://{addr}"))
            .expect("endpoint")
            .connect()
            .await
            .expect("connect test ledger");
        let ledger = LedgerClient::from_channel_for_test(channel.clone());
        let canonical_ingest = CanonicalIngestClient::from_channel_for_test(channel);
        let cfg = test_config(&fake_ledger.tenant_id, fake_ledger.fencing_scope_id);
        let signer = LocalEd25519Signer::from_key(
            SigningKey::from_bytes(&[7u8; 32]),
            "sidecar:workload-a".into(),
        );
        let state = SidecarState::new(
            ledger,
            canonical_ingest,
            IdempotencyCache::new(16, 600),
            1,
            600,
            Arc::new(signer),
            Arc::new(FailPolicyMatrix::default_fail_closed()),
            false,
        );
        crate::fencing::install_active(&state, fake_ledger.fencing_scope_id, 7, 120);

        (make_service(state, cfg, SidecarMetrics::new()), server)
    }

    #[tokio::test]
    async fn release_replay_after_local_fencing_expiry_returns_original_signature() {
        let fake_ledger = FakeReleaseLedger {
            tenant_id: "tenant-a".into(),
            reservation_id: Uuid::new_v4(),
            decision_id: Uuid::new_v4(),
            fencing_scope_id: Uuid::new_v4(),
            release_calls: Arc::new(AtomicUsize::new(0)),
            audit_signatures_seen: Arc::new(parking_lot::Mutex::new(Vec::new())),
        };
        let signatures_seen = fake_ledger.audit_signatures_seen.clone();
        let release_calls = fake_ledger.release_calls.clone();
        let reservation_id = fake_ledger.reservation_id;
        let (service, server) = test_service(fake_ledger).await;

        let req = ReleaseReservationRequest {
            reservation_id: reservation_id.to_string(),
            idempotency_key: "idem-1".into(),
            reason_codes: vec!["runtime_error".into()],
            tenant_id: "tenant-a".into(),
            workload_instance_id: "workload-a".into(),
            session_id: "session-a".into(),
        };

        let first = service
            .release_reservation_inner(Request::new(req.clone()))
            .await
            .expect("first release")
            .into_inner();
        assert!(!first.audit_event_signature.is_empty());

        service
            .state
            .inner
            .fencing
            .write()
            .as_mut()
            .expect("active fencing")
            .ttl_expires_at = Utc::now() - chrono::Duration::seconds(1);

        let replay = service
            .release_reservation_inner(Request::new(req))
            .await
            .expect("same-key replay after local lease expiry")
            .into_inner();

        assert_eq!(release_calls.load(Ordering::SeqCst), 2);
        assert_eq!(replay.ledger_transaction_id, first.ledger_transaction_id);
        assert_eq!(replay.audit_event_signature, first.audit_event_signature);
        let seen = signatures_seen.lock();
        assert_eq!(seen.len(), 2);
        assert_ne!(
            seen[1], replay.audit_event_signature,
            "replay must return cached original signature, not retry-event signature",
        );

        server.abort();
    }
}

// Silence unused warning until vertical slice consumes it.
#[allow(dead_code)]
fn _unused(_a: Arc<()>) {}
