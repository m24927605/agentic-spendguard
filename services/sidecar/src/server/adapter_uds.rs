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

    type EmitTraceEventsStream =
        ReceiverStream<Result<TraceEventAck, Status>>;

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
        self.metrics.inc_handler(Handler::IssueBudgetGrant, Outcome::Err);
        Err(Status::unimplemented(
            "IssueBudgetGrant: vertical slice expansion (Phase 1 sub-agent flow)",
        ))
    }
    async fn revoke_budget_grant(
        &self,
        _req: Request<RevokeBudgetGrantRequest>,
    ) -> Result<Response<RevokeBudgetGrantResponse>, Status> {
        self.metrics.inc_handler(Handler::RevokeBudgetGrant, Outcome::Err);
        Err(Status::unimplemented("RevokeBudgetGrant: vertical slice expansion"))
    }
    async fn consume_budget_grant(
        &self,
        _req: Request<ConsumeBudgetGrantRequest>,
    ) -> Result<Response<ConsumeBudgetGrantResponse>, Status> {
        self.metrics.inc_handler(Handler::ConsumeBudgetGrant, Outcome::Err);
        Err(Status::unimplemented("ConsumeBudgetGrant: vertical slice expansion"))
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
    ) -> Result<
        Response<crate::proto::sidecar_adapter::v1::ResumeAfterApprovalResponse>,
        Status,
    > {
        let result = self.resume_after_approval_inner(req).await;
        record_outcome(&self.metrics, Handler::ResumeAfterApproval, &result);
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
            active_key_epochs: Some(crate::proto::sidecar_adapter::v1::handshake_response::KeyEpochs {
                producer_signing_key_epochs: vec!["epoch-2026-Q2".into()],
                hmac_tenant_salt_epochs: vec!["epoch-2026-Q2".into()],
            }),
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

        // Idempotency short-circuit (Contract §6). Adapter retries with the
        // same Idempotency.key MUST collapse to the same cached response;
        // otherwise sidecar mints a fresh decision_id per call and the
        // ledger sees a duplicate logical request.
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
        if let Some(cached) = self.state.inner.idempotency.get(&idempotency_key) {
            tracing::debug!(key = %idempotency_key, "idempotent decision cache hit");
            return Ok(Response::new(cached));
        }

        crate::fencing::check_active(&self.state).map_err(|e| e.to_status())?;

        let ctx = DecisionContext {
            session_id: req.session_id.clone(),
            workload_instance_id: self.cfg.workload_instance_id.clone(),
            tenant_id: self.cfg.tenant_id.clone(),
            region: self.cfg.region.clone(),
        };

        let out = transaction::run_through_reserve(&self.cfg, &self.state, &ctx, &req)
            .await
            .map_err(|e| e.to_status())?;

        let response = transaction::build_response(out);
        self.state
            .inner
            .idempotency
            .put(idempotency_key, response.clone());
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
            use crate::proto::sidecar_adapter::v1::{
                llm_call_post_payload::Outcome as LlmOutcome, trace_event::EventKind,
                trace_event_ack::Status as AckStatus,
            };
            use crate::decision::transaction::{self, DecisionContext, ReleaseReason};
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
                    Some(crate::proto::sidecar_adapter::v1::trace_event::Payload::LlmCallPost(p)) => p,
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
                let outcome = LlmOutcome::try_from(payload.outcome).unwrap_or(LlmOutcome::Unspecified);
                let result: Result<String, crate::domain::error::DomainError> = match outcome {
                    LlmOutcome::Success => {
                        match transaction::run_commit_estimated(&cfg, &state, &dctx, &payload).await {
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
                    LlmOutcome::ProviderError | LlmOutcome::ClientTimeout | LlmOutcome::RunAborted => {
                        let reason = if outcome == LlmOutcome::RunAborted {
                            ReleaseReason::RunAborted
                        } else {
                            ReleaseReason::RuntimeError
                        };
                        let reservation_uuid = match uuid::Uuid::parse_str(&payload.reservation_id) {
                            Ok(u) => u,
                            Err(e) => {
                                let _ = tx
                                    .send(Ok(TraceEventAck {
                                        event_id,
                                        status: AckStatus::Rejected as i32,
                                        error: Some(crate::proto::common::v1::Error {
                                            code: crate::proto::common::v1::error::Code::Unspecified as i32,
                                            message: format!("reservation_id parse: {e}"),
                                            details: Default::default(),
                                        }),
                                    }))
                                    .await;
                                continue;
                            }
                        };
                        match transaction::run_release(
                            &cfg, &state, &dctx, reservation_uuid, reason,
                            if payload.provider_event_id.is_empty() {
                                None
                            } else {
                                Some(payload.provider_event_id.as_str())
                            },
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
                    LlmOutcome::Unspecified => Err(
                        crate::domain::error::DomainError::InvalidRequest(
                            "LLM_CALL_POST outcome=UNSPECIFIED".into(),
                        ),
                    ),
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
    ) -> Result<
        Response<crate::proto::sidecar_adapter::v1::ResumeAfterApprovalResponse>,
        Status,
    > {
        use crate::proto::common::v1::error::Code as ProtoCode;
        use crate::proto::ledger::v1::{
            get_approval_for_resume_response::Outcome as GetOutcome,
            GetApprovalForResumeRequest,
        };
        use crate::proto::sidecar_adapter::v1::{
            decision_response::Decision,
            resume_after_approval_response::Outcome as ResumeOutcome, DecisionResponse,
            ResumeAfterApprovalDenied, ResumeAfterApprovalResponse,
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
                    "[LEDGER_RESPONSE_EMPTY] GetApprovalForResume returned no oneof"
                        .into(),
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
                        ledger_transaction_id: context
                            .bundled_ledger_transaction_id
                            .clone(),
                        reservation_ids: vec![],
                        ttl_expires_at: None,
                        approval_request_id: context.approval_id.clone(),
                        approval_ttl: None,
                        approver_role: String::new(),
                        terminal: false,
                        error: None,
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

                    let reserve_resp = match self.state.inner.ledger.reserve_set(reserve_req).await {
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
                            s.reservations.iter().map(|r| r.reservation_id.clone()).collect::<Vec<_>>(),
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
                            return into_err(format!("[LEDGER_RPC_FAILED] MarkApprovalBundled: {e}"));
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

    pub fn parse(
        ctx: &crate::proto::ledger::v1::ApprovalResumeContext,
    ) -> Result<Parsed, String> {
        let decision: DecisionContext =
            serde_json::from_slice(&ctx.decision_context_json)
                .map_err(|e| format!("decision_context_json: {e}"))?;
        let effect: RequestedEffect =
            serde_json::from_slice(&ctx.requested_effect_json)
                .map_err(|e| format!("requested_effect_json: {e}"))?;
        Ok(Parsed { decision, effect })
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
                budget_claim::Direction, unit_ref::Kind as UnitKind, BudgetClaim,
                CloudEvent, ContractBundleRef, Fencing, Idempotency, PricingFreeze,
                UnitRef,
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
                run_id: String::new(),
                decision_id: decision_id.to_string(),
                schema_bundle_id: self.decision.schema_bundle_id.clone(),
                producer_id: format!("sidecar:{}", workload_id),
                producer_sequence,
                producer_signature: vec![].into(),
                signing_key_id: String::new(),
            };
            crate::audit::sign_cloudevent_in_place(&*state.inner.signer, &mut cloudevent)
                .await
                .map_err(|e| format!("sign resume cloudevent: {e}"))?;

            let pricing = PricingFreeze {
                pricing_version: String::new(),
                price_snapshot_hash: vec![].into(),
                fx_rate_version: String::new(),
                unit_conversion_version: String::new(),
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
    AdapterUds { state, cfg, metrics }
}

// Silence unused warning until vertical slice consumes it.
#[allow(dead_code)]
fn _unused(_a: Arc<()>) {}
