"""Async gRPC client for the SpendGuard sidecar over a Unix Domain Socket.

The sidecar listens on a UDS configured via `SPENDGUARD_SIDECAR_UDS` (or
explicit `socket_path`). All adapter→sidecar RPCs flow through this
client; the sidecar then talks to the ledger / canonical_ingest over
mTLS upstream.

Wire reference: proto/spendguard/sidecar_adapter/v1/adapter.proto.
Spec references:
  - Sidecar Architecture §3 (in_process_adapter ↔ local_sidecar IPC)
  - Sidecar Architecture §5 (UDS peer credentials + handshake)
  - Sidecar Architecture §11 (drain protocol)
  - Stage 2 §4.6 (publish_effect idempotency via effect_hash)
  - Trace Schema §3.4 (idempotency_key fallback)

Failure model:
  - Sidecar unreachable / deadline exceeded → SidecarUnavailable.
  - Sidecar returns gRPC INVALID_ARGUMENT / FAILED_PRECONDITION →
    SpendGuardError with the trailing Status detail.
  - Sidecar returns DECISION = STOP / SKIP / REQUIRE_APPROVAL →
    decision-typed exception (DecisionStopped, DecisionSkipped,
    ApprovalRequired) so the Pydantic-AI Agent run loop unwinds.
"""

from __future__ import annotations

import asyncio
import logging
from collections.abc import AsyncIterator
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any

import grpc

logger = logging.getLogger(__name__)

# Generated proto stubs — produced by `make proto`. Importing here gives a
# clear error at module load if the build step was skipped.
try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
    from spendguard._proto.spendguard.sidecar_adapter.v1 import (
        adapter_pb2,
        adapter_pb2_grpc,
    )
except ImportError as exc:  # pragma: no cover — build configuration error
    raise ImportError(
        "spendguard proto stubs missing. "
        "Run `make proto` from sdk/python/ first."
    ) from exc

from .errors import (
    ApprovalBundleHotReloadedError,
    ApprovalDeniedError,
    ApprovalLapsedError,
    ApprovalRequired,
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    SidecarUnavailable,
    SpendGuardError,
)
from .session import (
    CommitSessionDeltaOutcome,
    CommitSessionDeltaRequest,
    ReleaseSessionOutcome,
    ReleaseSessionRequest,
    ReserveSessionAccepted,
    ReserveSessionDenied,
    ReserveSessionOutcome,
    ReserveSessionRequest,
    build_commit_session_delta_request,
    build_release_session_request,
    build_reserve_session_request,
    timestamp_to_datetime,
)

if TYPE_CHECKING:
    pass


# Default deadlines (per Sidecar Architecture §14 latency budget).
# Decision warm path p99 is 50ms; we use 5x p99 as a hard ceiling so a
# slow stage doesn't queue up adapter retries.
DEFAULT_DECISION_TIMEOUT_S = 0.250
DEFAULT_HANDSHAKE_TIMEOUT_S = 2.0
DEFAULT_PUBLISH_TIMEOUT_S = 0.150
DEFAULT_TRACE_TIMEOUT_S = 0.500


def _build_trace_context(
    traceparent: str, tracestate: str
) -> "common_pb2.TraceContext":
    """Translate a W3C `traceparent` header into the wire `TraceContext`.

    Wire fields (per proto): `trace_id` (32-hex), `span_id` (16-hex),
    `parent_span_id` (optional, 16-hex), `trace_state` (opaque).
    A W3C `traceparent` looks like `00-<32hex>-<16hex>-<2hex>`. We
    reuse the upstream span_id as `parent_span_id` because, from the
    sidecar/ledger's perspective, the next event is a child span.

    Empty / malformed input → empty struct (no trace propagation). The
    sidecar treats trace fields as observability decoration and does
    not gate enforcement on them.
    """
    if not traceparent:
        if tracestate:
            return common_pb2.TraceContext(trace_state=tracestate)
        return common_pb2.TraceContext()
    parts = traceparent.split("-")
    if len(parts) != 4 or len(parts[1]) != 32 or len(parts[2]) != 16:
        return common_pb2.TraceContext(trace_state=tracestate)
    return common_pb2.TraceContext(
        trace_id=parts[1],
        span_id=parts[2],
        parent_span_id=parts[2],
        trace_state=tracestate,
    )


@dataclass(frozen=True, slots=True)
class HandshakeOutcome:
    """Outcome of a successful handshake.

    Carries the session_id the adapter must echo on every subsequent
    RPC, plus the bundle refs and capability the sidecar negotiated.
    """

    session_id: str
    sidecar_version: str
    schema_bundle_id: str
    schema_bundle_hash: bytes
    contract_bundle_id: str
    contract_bundle_hash: bytes
    capability_required: int
    signing_key_id: str
    announcement_signature: bytes


@dataclass(frozen=True, slots=True)
class DecisionOutcome:
    """Surface of a CONTINUE/DEGRADE decision.

    Non-terminal outcomes (STOP / SKIP / REQUIRE_APPROVAL) are raised as
    typed exceptions instead — see errors.py.
    """

    decision_id: str
    audit_decision_event_id: str
    decision: str  # "CONTINUE" | "DEGRADE"
    mutation_patch_json: str  # empty unless DEGRADE
    effect_hash: bytes
    ledger_transaction_id: str
    reservation_ids: tuple[str, ...]
    ttl_expires_at_seconds: int
    reason_codes: tuple[str, ...]
    matched_rule_ids: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class ReleaseOutcome:
    """Surface of a successful `release_reservation` call.

    Matches the Agent Spend Protocol Draft-01 §4 ReleaseResponse shape
    (audit_event_signature) plus the SpendGuard-specific extension
    fields (ledger_transaction_id, released_reservation_ids).
    """

    # Detached Ed25519 signature of the emitted audit.release CloudEvent.
    # Empty bytes when the call hit the ledger's idempotent Replay branch
    # (the freshly-generated signature does not correspond to the
    # persisted-in-chain original; see GH #85). Non-empty on first
    # success. Adapters that want the original signature on replay can
    # re-fetch from the audit chain via ledger_transaction_id.
    audit_event_signature: bytes
    # Ledger transaction id for the release (stable across retries).
    ledger_transaction_id: str
    # Reservations released by this call. Single-element tuple in the
    # current single-reservation-per-call model.
    released_reservation_ids: tuple[str, ...]


class SpendGuardClient:
    """Async UDS gRPC client for the spendguard sidecar.

    Use as an async context manager so the channel is closed on exit:

        async with SpendGuardClient(socket_path="/run/spendguard.sock") as c:
            await c.handshake(...)
            outcome = await c.request_decision(...)

    Thread/task safety: a single client instance can be shared across
    concurrent Pydantic-AI runs in the same event loop. The underlying
    grpc.aio channel multiplexes RPCs over the UDS connection.
    """

    def __init__(
        self,
        *,
        socket_path: str,
        tenant_id: str,
        runtime_kind: str = "pydantic-ai",
        runtime_version: str = "",
        sdk_version: str = "0.1.0a1",
        protocol_version: int = 1,
        capability_level: int = 0x40,  # L3_POLICY_HOOK
        decision_timeout_s: float = DEFAULT_DECISION_TIMEOUT_S,
        handshake_timeout_s: float = DEFAULT_HANDSHAKE_TIMEOUT_S,
        publish_timeout_s: float = DEFAULT_PUBLISH_TIMEOUT_S,
        trace_timeout_s: float = DEFAULT_TRACE_TIMEOUT_S,
    ) -> None:
        if not socket_path:
            raise ValueError("socket_path is required")
        if not tenant_id:
            raise ValueError("tenant_id is required")

        self._socket_path = socket_path
        self._tenant_id = tenant_id
        self._runtime_kind = runtime_kind
        self._runtime_version = runtime_version
        self._sdk_version = sdk_version
        self._protocol_version = protocol_version
        self._capability_level = capability_level
        self._decision_timeout_s = decision_timeout_s
        self._handshake_timeout_s = handshake_timeout_s
        self._publish_timeout_s = publish_timeout_s
        self._trace_timeout_s = trace_timeout_s

        self._channel: grpc.aio.Channel | None = None
        self._stub: adapter_pb2_grpc.SidecarAdapterStub | None = None
        self._handshake: HandshakeOutcome | None = None
        self._handshake_lock = asyncio.Lock()

    # -------------------------------------------------------------------
    # Lifecycle
    # -------------------------------------------------------------------

    async def __aenter__(self) -> "SpendGuardClient":
        await self.connect()
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: Any,
    ) -> None:
        await self.close()

    async def connect(self) -> None:
        if self._channel is not None:
            return
        # `unix:` URI scheme is supported by grpc-python ≥ 1.30.
        target = f"unix:{self._socket_path}"
        # Override the HTTP/2 `:authority` pseudo-header — grpc-python
        # defaults it to the URL-encoded UDS path (e.g.,
        # `var%2Frun%2Fspendguard%2Fadapter.sock`), which tonic's h2
        # parser rejects as a malformed authority and resets every
        # stream with PROTOCOL_ERROR before the gRPC handler runs.
        # `localhost` is a stable, valid authority over UDS where the
        # peer is implicitly the connection itself.
        options = [
            ("grpc.default_authority", "localhost"),
        ]
        self._channel = grpc.aio.insecure_channel(target, options=options)
        self._stub = adapter_pb2_grpc.SidecarAdapterStub(self._channel)

    async def close(self) -> None:
        ch = self._channel
        if ch is None:
            return
        self._channel = None
        self._stub = None
        try:
            await ch.close(grace=0.5)
        except Exception:  # noqa: BLE001 — closing on an already-broken channel is fine
            pass

    @property
    def session_id(self) -> str:
        if self._handshake is None:
            raise HandshakeError(
                "handshake() has not completed; session_id is not yet known"
            )
        return self._handshake.session_id

    @property
    def tenant_id(self) -> str:
        """The tenant_id this client asserted at construction.

        Stable across the client's lifetime — handshake re-uses it. This
        is the value subsequent idempotency keys must be derived against.
        """
        return self._tenant_id

    @property
    def handshake_outcome(self) -> HandshakeOutcome:
        if self._handshake is None:
            raise HandshakeError("handshake() has not completed")
        return self._handshake

    async def safe_confirm_apply_failed(
        self,
        *,
        decision_id: str,
        effect_hash: bytes,
        adapter_error: str,
    ) -> None:
        """Confirm publish_outcome=APPLY_FAILED, swallowing transport errors.

        Used in the inner-call exception path: we want to anchor the
        failed publish so the audit chain records the rollback, but we
        must NOT shadow the original exception that triggered cleanup.
        Errors here are logged at WARNING and dropped.
        """
        try:
            await self.confirm_publish_outcome(
                decision_id=decision_id,
                effect_hash=effect_hash,
                outcome="APPLY_FAILED",
                adapter_error=adapter_error[:1024],
            )
        except SpendGuardError as e:
            logger.warning(
                "confirm_publish_outcome(APPLY_FAILED) failed; "
                "audit chain may be missing terminal anchor for "
                "decision_id=%s: %s",
                decision_id,
                e,
            )
        except asyncio.CancelledError:
            raise
        except Exception as e:  # noqa: BLE001 — defensive: never shadow caller
            logger.warning(
                "confirm_publish_outcome(APPLY_FAILED) raised unexpected "
                "exception for decision_id=%s: %s",
                decision_id,
                e,
            )

    # -------------------------------------------------------------------
    # Handshake
    # -------------------------------------------------------------------

    async def handshake(self, *, workload_instance_id: str = "") -> HandshakeOutcome:
        """Perform the mandatory initial handshake.

        Idempotent: a second call returns the cached outcome without
        re-issuing the RPC.

        Per Sidecar §5: sidecar verifies tenant_id assertion against
        SO_PEERCRED + signed manifest. POC adapter trusts the response;
        Phase 1+ verifies announcement_signature against the
        Helm-pinned root CA bundle.
        """
        async with self._handshake_lock:
            if self._handshake is not None:
                return self._handshake
            stub = self._require_stub()

            req = adapter_pb2.HandshakeRequest(
                sdk_version=self._sdk_version,
                runtime_kind=self._runtime_kind,
                runtime_version=self._runtime_version,
                capability_level=self._capability_level,
                tenant_id_assertion=self._tenant_id,
                workload_instance_id=workload_instance_id,
                protocol_version=self._protocol_version,
            )
            try:
                resp: adapter_pb2.HandshakeResponse = await stub.Handshake(
                    req, timeout=self._handshake_timeout_s
                )
            except grpc.aio.AioRpcError as e:
                raise self._classify_rpc_error(e, op="handshake") from e

            if resp.protocol_version != self._protocol_version:
                raise HandshakeError(
                    f"protocol version mismatch: adapter={self._protocol_version} "
                    f"sidecar={resp.protocol_version}"
                )

            schema = resp.schema_bundle
            contract = resp.contract_bundle
            # SchemaBundleRef carries the full prefix (schema_bundle_id /
            # schema_bundle_hash); ContractBundleRef uses the short form
            # (bundle_id / bundle_hash). They are distinct messages.
            outcome = HandshakeOutcome(
                session_id=resp.session_id,
                sidecar_version=resp.sidecar_version,
                schema_bundle_id=schema.schema_bundle_id,
                schema_bundle_hash=bytes(schema.schema_bundle_hash),
                contract_bundle_id=contract.bundle_id,
                contract_bundle_hash=bytes(contract.bundle_hash),
                capability_required=int(resp.capability_required),
                signing_key_id=resp.signing_key_id,
                announcement_signature=bytes(resp.announcement_signature),
            )
            if outcome.capability_required > self._capability_level:
                raise HandshakeError(
                    f"sidecar requires capability {hex(outcome.capability_required)} "
                    f"but adapter advertised {hex(self._capability_level)}; refusing"
                )
            self._handshake = outcome
            return outcome

    # -------------------------------------------------------------------
    # RequestDecision
    # -------------------------------------------------------------------

    async def request_decision(
        self,
        *,
        trigger: str,
        run_id: str,
        step_id: str,
        llm_call_id: str,
        tool_call_id: str,
        decision_id: str,
        route: str,
        projected_claims: list[common_pb2.BudgetClaim],
        idempotency_key: str,
        traceparent: str = "",
        tracestate: str = "",
        parent_run_id: str = "",
        budget_grant_jti: str = "",
        projected_p50_atomic: str = "",
        projected_p90_atomic: str = "",
        projected_p95_atomic: str = "",
        projected_p99_atomic: str = "",
        projected_unit: common_pb2.UnitRef | None = None,
        prompt_text: str | None = None,
        decision_context_json: dict | None = None,
        claim_estimate: "adapter_pb2.ClaimEstimate | None" = None,
    ) -> DecisionOutcome:
        """Run a `*.pre` decision boundary through the sidecar.

        Returns a DecisionOutcome only for CONTINUE/DEGRADE. Other
        decisions raise the corresponding typed exception so the
        Pydantic-AI Agent run loop unwinds without further LLM/tool
        calls.

        `idempotency_key` MUST be deterministic over the (tenant, run,
        step, llm_call, trigger) tuple — see ids.derive_idempotency_key.
        Otherwise a Pydantic-AI internal retry mints a fresh key, the
        sidecar treats it as a new logical request, and the ledger
        double-reserves.
        """
        stub = self._require_stub()
        session_id = self.session_id
        trigger_enum = self._trigger_for_name(trigger)

        ids = common_pb2.SpendGuardIds(
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id=tool_call_id,
            decision_id=decision_id,
        )
        # TraceContext on the wire uses the W3C *fields* (trace_id,
        # span_id, parent_span_id, trace_state), not the *header*
        # (`traceparent`). Parse the caller-supplied W3C `traceparent`
        # string when present (format: 00-<32 hex>-<16 hex>-<2 hex>).
        # Otherwise leave the struct empty — the sidecar/ledger don't
        # require trace propagation for POC enforcement decisions.
        trace = _build_trace_context(traceparent, tracestate)
        # Cost Advisor P0.5 enrichment: pass prompt_hash via
        # runtime_metadata. Sidecar reads it back in
        # services/sidecar/src/decision/transaction.rs::extract_enrichment
        # and threads into the audit.decision CloudEvent.
        #
        # Codex r1 P3 fix: None means "caller didn't pass a prompt"
        # → skip enrichment. Empty string "" means "caller explicitly
        # asked to hash empty content" → compute HMAC of empty bytes
        # (matches Rust prompt_hash::compute behavior on empty input).
        runtime_metadata = None
        if prompt_text is not None or decision_context_json is not None:
            from google.protobuf import struct_pb2

            runtime_metadata = struct_pb2.Struct()
            if prompt_text is not None:
                from .prompt_hash import compute as compute_prompt_hash

                runtime_metadata["prompt_hash"] = compute_prompt_hash(
                    prompt_text, self._tenant_id
                )
            # LiteLLM integration (DESIGN.md §8.2a): fold caller-supplied
            # decision context fields into runtime_metadata so they land
            # in canonical_events.decision_context_json.
            if decision_context_json is not None:
                for key, value in decision_context_json.items():
                    # Struct.update would overwrite prompt_hash if caller
                    # collides; assign per-key so existing fields win.
                    if key not in runtime_metadata:
                        runtime_metadata[key] = value

        inputs = adapter_pb2.DecisionRequest.Inputs(
            projected_claims=list(projected_claims),
            projected_p50_atomic=projected_p50_atomic,
            projected_p90_atomic=projected_p90_atomic,
            projected_p95_atomic=projected_p95_atomic,
            projected_p99_atomic=projected_p99_atomic,
            projected_unit=projected_unit or common_pb2.UnitRef(),
            runtime_metadata=runtime_metadata,
            claim_estimate=claim_estimate,
        )
        idem = common_pb2.Idempotency(
            key=idempotency_key,
            request_hash=b"",  # let sidecar/ledger own canonical
        )
        # SLICE_12: read the active RunPlan (Signal 3) from the
        # context-var set by ``with_run_plan``. When no plan is active
        # (``None``), we leave ``planned_steps_hint=0`` and the
        # projector falls back to Signal 1 (history-induced) per
        # `run-cost-projector-spec-v1alpha1.md` §5.2.
        from .run_plan import current_run_plan

        plan = current_run_plan()
        planned_steps_hint = plan.planned_steps_hint if plan is not None else 0
        req = adapter_pb2.DecisionRequest(
            session_id=session_id,
            trigger=trigger_enum,
            trace=trace,
            ids=ids,
            route=route,
            inputs=inputs,
            parent_run_id=parent_run_id,
            budget_grant_jti=budget_grant_jti,
            idempotency=idem,
            planned_steps_hint=planned_steps_hint,
        )

        try:
            resp: adapter_pb2.DecisionResponse = await stub.RequestDecision(
                req, timeout=self._decision_timeout_s
            )
        except grpc.aio.AioRpcError as e:
            raise self._classify_rpc_error(e, op="request_decision") from e

        if resp.error.code:
            raise SpendGuardError(
                f"sidecar error code={resp.error.code} message={resp.error.message}"
            )

        decision_name = self._decision_name(resp.decision)
        if decision_name in ("CONTINUE", "DEGRADE"):
            return DecisionOutcome(
                decision_id=resp.decision_id,
                audit_decision_event_id=resp.audit_decision_event_id,
                decision=decision_name,
                mutation_patch_json=resp.mutation_patch_json,
                effect_hash=bytes(resp.effect_hash),
                ledger_transaction_id=resp.ledger_transaction_id,
                reservation_ids=tuple(resp.reservation_ids),
                ttl_expires_at_seconds=resp.ttl_expires_at.seconds,
                reason_codes=tuple(resp.reason_codes),
                matched_rule_ids=tuple(resp.matched_rule_ids),
            )
        if decision_name in ("STOP", "STOP_RUN_PROJECTION"):
            raise DecisionStopped(
                f"sidecar {decision_name} terminal={resp.terminal} reasons={list(resp.reason_codes)}",
                decision_id=resp.decision_id,
                reason_codes=list(resp.reason_codes),
                audit_decision_event_id=resp.audit_decision_event_id,
                matched_rule_ids=list(resp.matched_rule_ids),
            )
        if decision_name == "SKIP":
            raise DecisionSkipped(
                f"sidecar SKIP reasons={list(resp.reason_codes)}",
                decision_id=resp.decision_id,
                reason_codes=list(resp.reason_codes),
                audit_decision_event_id=resp.audit_decision_event_id,
                matched_rule_ids=list(resp.matched_rule_ids),
            )
        if decision_name == "REQUIRE_APPROVAL":
            raise ApprovalRequired(
                f"sidecar REQUIRE_APPROVAL approval_request_id={resp.approval_request_id}",
                decision_id=resp.decision_id,
                approval_request_id=resp.approval_request_id,
                approver_role=resp.approver_role,
                reason_codes=list(resp.reason_codes),
                audit_decision_event_id=resp.audit_decision_event_id,
                matched_rule_ids=list(resp.matched_rule_ids),
                # Round-2 #9 part 2 PR 9d: propagate tenant_id so the
                # ApprovalRequired.resume() round-trip can scope the
                # GetApprovalForResume lookup against tenant.
                tenant_id=self.tenant_id,
            )
        # Unknown decision kind — treat as denial.
        raise DecisionDenied(
            f"sidecar returned unknown decision={resp.decision}",
            decision_id=resp.decision_id,
            reason_codes=list(resp.reason_codes),
            audit_decision_event_id=resp.audit_decision_event_id,
            matched_rule_ids=list(resp.matched_rule_ids),
        )

    # -------------------------------------------------------------------
    # ResumeAfterApproval (Round-2 #9 part 2 PR 9d)
    # -------------------------------------------------------------------

    async def resume_after_approval(
        self,
        *,
        approval_id: str,
        tenant_id: str,
        decision_id: str,
        workload_instance_id: str = "",
    ) -> "DecisionOutcome":
        """Call sidecar `ResumeAfterApproval` after the operator has
        approved (or denied) the gating approval request.

        Returns a `DecisionOutcome` if the approval is `approved` and
        the resume produced (or replayed) a Continue decision. Raises
        `ApprovalDeniedError` if the operator rejected, or
        `ApprovalLapsedError` for non-actionable states (pending /
        expired / cancelled).

        Typical usage from a Pydantic-AI handler:

            try:
                await client.request_decision(...)
            except ApprovalRequired as e:
                # ... wait for approver via Slack / your control plane ...
                outcome = await e.resume(client)
        """
        stub = self._require_stub()
        session_id = self.session_id

        req = adapter_pb2.ResumeAfterApprovalRequest(
            tenant_id=tenant_id,
            decision_id=decision_id,
            approval_id=approval_id,
            workload_instance_id=workload_instance_id,
            session_id=session_id,
        )
        try:
            resp = await stub.ResumeAfterApproval(
                req, timeout=DEFAULT_DECISION_TIMEOUT_S * 4.0
            )
        except grpc.aio.AioRpcError as e:
            raise self._classify_rpc_error(e, op="resume_after_approval") from e

        kind = resp.WhichOneof("outcome")
        if kind == "decision":
            d = resp.decision
            return DecisionOutcome(
                decision_id=d.decision_id,
                audit_decision_event_id=d.audit_decision_event_id,
                decision=adapter_pb2.DecisionResponse.Decision.Name(d.decision),
                mutation_patch_json=d.mutation_patch_json,
                effect_hash=bytes(d.effect_hash),
                ledger_transaction_id=d.ledger_transaction_id,
                reservation_ids=tuple(d.reservation_ids),
                ttl_expires_at_seconds=(
                    d.ttl_expires_at.seconds
                    if d.HasField("ttl_expires_at")
                    else 0
                ),
                reason_codes=tuple(d.reason_codes),
                matched_rule_ids=tuple(d.matched_rule_ids),
            )
        if kind == "denied":
            denied = resp.denied
            raise ApprovalDeniedError(
                f"approval denied by {denied.approver_subject or '<unknown>'}",
                decision_id=decision_id,
                approver_subject=denied.approver_subject or None,
                approver_reason=denied.approver_reason or None,
                audit_decision_event_id=denied.audit_decision_event_id or None,
                matched_rule_ids=list(denied.matched_rule_ids),
            )
        if kind == "error":
            err = resp.error
            # Round-2 #9 PR 9c: sidecar tags the message with bracketed
            # prefixes like [APPROVAL_NON_TERMINAL], [PRODUCER_SP_NOT_WIRED],
            # [LEDGER_RPC_FAILED] etc. Map non-terminal → ApprovalLapsedError;
            # bundle hot-reload → ApprovalBundleHotReloadedError (issue #59);
            # everything else → SpendGuardError.
            msg = err.message
            if msg.startswith("[APPROVAL_NON_TERMINAL]"):
                state = "unknown"
                # Best-effort parse: "[APPROVAL_NON_TERMINAL] approval state=\"X\" ..."
                idx = msg.find("state=")
                if idx >= 0:
                    tail = msg[idx + 6 :].strip().strip('"')
                    state = tail.split()[0].strip('"') if tail else "unknown"
                raise ApprovalLapsedError(
                    msg,
                    decision_id=decision_id,
                    state=state,
                )
            if "[BUNDLE_HOT_RELOADED]" in msg:
                # Sidecar shape (may be wrapped by an outer
                # [RESUME_BUILD_FAILED] tag from adapter_uds.rs's
                # into_reserve_set_request catch-all; substring match
                # tolerates the wrap):
                #   "[BUNDLE_HOT_RELOADED] approval was issued under bundle hash <A>
                #    but the sidecar's currently-installed bundle is <B>; ..."
                import re

                hashes = re.findall(r"\b[0-9a-f]{64}\b", msg)
                original = hashes[0] if len(hashes) >= 1 else ""
                current = hashes[1] if len(hashes) >= 2 else ""
                raise ApprovalBundleHotReloadedError(
                    msg,
                    original_bundle_hash=original,
                    current_bundle_hash=current,
                )
            raise SpendGuardError(
                f"sidecar ResumeAfterApproval error: {msg}"
            )
        raise SpendGuardError(
            f"sidecar ResumeAfterApproval returned unknown oneof: {kind!r}"
        )

    # -------------------------------------------------------------------
    # ReleaseReservation (Agent Spend Protocol Draft-01 §4)
    # -------------------------------------------------------------------

    async def release_reservation(
        self,
        *,
        reservation_id: str,
        idempotency_key: str,
        reason_codes: tuple[str, ...] | list[str] = (),
        workload_instance_id: str = "",
        tenant_id: str = "",
    ) -> "ReleaseOutcome":
        """Explicit adapter-initiated release of a held reservation.

        Matches Agent Spend Protocol Draft-01 §4 — use when the provider
        call is aborted, the client times out, or the agent run is
        cancelled, and the adapter wants to surface that explicitly
        rather than waiting for the implicit outcome-driven release
        paths (ConfirmPublishOutcome.APPLY_FAILED, EmitTraceEvents
        run-aborted / runtime-error).

        Reason-code mapping (sidecar — aligned with the implicit
        EmitTraceEvents path so audit reason values are consistent):

            "provider_error" | "runtime_error" | "client_timeout"
                → audit RELEASE reason RuntimeError
            "run_aborted" | "run_cancelled"
                → RunAborted
            anything else
                → Explicit (adapter intent preserved in the audit
                  metadata regardless)

        Idempotent: same (reservation_id, idempotency_key) returns the
        original outcome on retry. Same-process retry returns the
        original audit_event_signature while the sidecar replay cache is
        warm; cache miss returns empty bytes rather than a fabricated
        signature. Different idempotency_key against an already-released
        reservation, stale first-time mutation, and same-key different
        ledger request body surface as gRPC FailedPrecondition.
        """
        stub = self._require_stub()
        session_id = self.session_id

        req = adapter_pb2.ReleaseReservationRequest(
            reservation_id=reservation_id,
            idempotency_key=idempotency_key,
            reason_codes=list(reason_codes),
            tenant_id=tenant_id,
            workload_instance_id=workload_instance_id,
            session_id=session_id,
        )
        try:
            resp: adapter_pb2.ReleaseReservationResponse = await stub.ReleaseReservation(
                req, timeout=self._publish_timeout_s
            )
        except grpc.aio.AioRpcError as e:
            raise self._classify_rpc_error(e, op="release_reservation") from e

        return ReleaseOutcome(
            audit_event_signature=bytes(resp.audit_event_signature),
            ledger_transaction_id=resp.ledger_transaction_id,
            released_reservation_ids=tuple(resp.released_reservation_ids),
        )

    # -------------------------------------------------------------------
    # D41 session reservation substrate
    # -------------------------------------------------------------------

    async def reserve_session(
        self, req: ReserveSessionRequest
    ) -> ReserveSessionOutcome:
        """Reserve a session-scoped hold for realtime voice spend.

        If ``req.session_id`` is empty, the SDK fills it from the completed
        sidecar handshake session id. The RPC is otherwise a direct wrapper
        around the SR-V1 ``ReserveSession`` wire contract.
        """
        stub = self._require_stub()
        session_id = self.session_id
        wire_req = build_reserve_session_request(
            replace(req, session_id=req.session_id or session_id)
        )
        try:
            resp: adapter_pb2.ReserveSessionOutcome = await stub.ReserveSession(
                wire_req, timeout=self._decision_timeout_s
            )
        except grpc.aio.AioRpcError as e:
            raise self._classify_rpc_error(e, op="reserve_session") from e
        return _map_reserve_session_outcome(resp)

    async def commit_session_delta(
        self, req: CommitSessionDeltaRequest
    ) -> CommitSessionDeltaOutcome:
        """Commit one positive streaming spend delta against a session hold."""
        stub = self._require_stub()
        _ = self.session_id
        wire_req = build_commit_session_delta_request(req)
        try:
            resp: adapter_pb2.CommitSessionDeltaOutcome = await stub.CommitSessionDelta(
                wire_req, timeout=self._trace_timeout_s
            )
        except grpc.aio.AioRpcError as e:
            raise self._classify_rpc_error(e, op="commit_session_delta") from e
        return _map_commit_session_delta_outcome(resp)

    async def release_session(
        self, req: ReleaseSessionRequest
    ) -> ReleaseSessionOutcome:
        """Release the uncommitted remainder of a session reservation."""
        stub = self._require_stub()
        _ = self.session_id
        wire_req = build_release_session_request(req)
        try:
            resp: adapter_pb2.ReleaseSessionOutcome = await stub.ReleaseSession(
                wire_req, timeout=self._publish_timeout_s
            )
        except grpc.aio.AioRpcError as e:
            raise self._classify_rpc_error(e, op="release_session") from e
        return _map_release_session_outcome(resp)

    # -------------------------------------------------------------------
    # ConfirmPublishOutcome
    # -------------------------------------------------------------------

    async def confirm_publish_outcome(
        self,
        *,
        decision_id: str,
        effect_hash: bytes,
        outcome: str,
        adapter_error: str = "",
    ) -> str:
        """Confirm the publish_effect step (Contract §6 stage 7).

        Idempotent on the sidecar via effect_hash (Stage 2 §4.6); the
        adapter is free to call this multiple times for the same
        (decision_id, effect_hash) and will get the same audit_outcome
        anchor back.

        `outcome` is one of: APPLIED, APPLIED_NOOP, APPLY_FAILED,
        APPROVAL_GRANTED, APPROVAL_DENIED, APPROVAL_TIMED_OUT.
        """
        stub = self._require_stub()
        session_id = self.session_id

        outcome_enum = self._outcome_for_name(outcome)
        req = adapter_pb2.PublishOutcomeRequest(
            session_id=session_id,
            decision_id=decision_id,
            effect_hash=effect_hash,
            outcome=outcome_enum,
            adapter_error=adapter_error,
        )
        try:
            resp: adapter_pb2.PublishOutcomeResponse = await stub.ConfirmPublishOutcome(
                req, timeout=self._publish_timeout_s
            )
        except grpc.aio.AioRpcError as e:
            raise self._classify_rpc_error(e, op="confirm_publish_outcome") from e
        if resp.error.code:
            raise SpendGuardError(
                f"sidecar publish error code={resp.error.code} "
                f"message={resp.error.message}"
            )
        return resp.audit_outcome_event_id

    # -------------------------------------------------------------------
    # EmitTraceEvents (one-shot helper)
    # -------------------------------------------------------------------

    async def emit_llm_call_post(
        self,
        *,
        run_id: str,
        step_id: str,
        llm_call_id: str,
        decision_id: str,
        reservation_id: str,
        provider_reported_amount_atomic: str,
        unit: common_pb2.UnitRef,
        pricing: common_pb2.PricingFreeze,
        provider_event_id: str,
        outcome: str,
        estimated_amount_atomic: str = "",
        actual_input_tokens: int | None = None,
        actual_output_tokens: int | None = None,
        delta_b_ratio: float | None = None,
        delta_c_ratio: float | None = None,
        traceparent: str = "",
        tracestate: str = "",
        provider_response_metadata: str = "",
    ) -> None:
        """Emit a single LLM_CALL_POST trace event and close the stream.

        For POC simplicity each event opens a fresh bidi stream; for
        production keep a long-lived stream in `_trace_stream` so that
        per-event setup cost (TLS handshake on remote, channel
        multiplexing) doesn't add to LLM-call latency.

        Phase 2B Step 7: pass `estimated_amount_atomic` to drive the
        sidecar's CommitEstimated path; mutually exclusive with
        `provider_reported_amount_atomic` (which targets the deferred
        ProviderReport path in step 8).
        """
        stub = self._require_stub()
        session_id = self.session_id

        outcome_enum = self._llm_outcome_for_name(outcome)
        from google.protobuf import timestamp_pb2

        ts = timestamp_pb2.Timestamp()
        ts.GetCurrentTime()
        payload_kwargs = {
            "reservation_id": reservation_id,
            "provider_reported_amount_atomic": provider_reported_amount_atomic,
            "estimated_amount_atomic": estimated_amount_atomic,
            "unit": unit,
            "pricing": pricing,
            "provider_event_id": provider_event_id,
            "outcome": outcome_enum,
        }
        if actual_input_tokens is not None:
            payload_kwargs["actual_input_tokens"] = actual_input_tokens
        if actual_output_tokens is not None:
            payload_kwargs["actual_output_tokens"] = actual_output_tokens
        if delta_b_ratio is not None:
            payload_kwargs["delta_b_ratio"] = delta_b_ratio
        if delta_c_ratio is not None:
            payload_kwargs["delta_c_ratio"] = delta_c_ratio
        payload = adapter_pb2.LlmCallPostPayload(**payload_kwargs)
        ev = adapter_pb2.TraceEvent(
            session_id=session_id,
            trace=_build_trace_context(traceparent, tracestate),
            ids=common_pb2.SpendGuardIds(
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                decision_id=decision_id,
            ),
            kind=adapter_pb2.TraceEvent.LLM_CALL_POST,
            event_time=ts,
            llm_call_post=payload,
            provider_response_metadata=provider_response_metadata,
        )

        async def _gen() -> AsyncIterator[adapter_pb2.TraceEvent]:
            yield ev

        try:
            stream = stub.EmitTraceEvents(
                _gen(), timeout=self._trace_timeout_s
            )
            # Drain the ack stream and surface rejection. Sidecar emits
            # exactly one ack per inbound event in this POC; if status is
            # not ACCEPTED the commit lifecycle failed and the caller MUST
            # see the error rather than silently treat it as success
            # (Codex round 2 challenge P1.1).
            async for ack in stream:
                if ack.status != adapter_pb2.TraceEventAck.ACCEPTED:
                    err = ack.error
                    raise SpendGuardError(
                        "EmitTraceEvents rejected: status="
                        f"{adapter_pb2.TraceEventAck.Status.Name(ack.status)} "
                        f"code={err.code if err else 0} "
                        f"message={(err.message if err else '')!r}"
                    )
        except grpc.aio.AioRpcError as e:
            raise self._classify_rpc_error(e, op="emit_trace_event") from e

    # -------------------------------------------------------------------
    # Internals
    # -------------------------------------------------------------------

    def _require_stub(self) -> adapter_pb2_grpc.SidecarAdapterStub:
        if self._stub is None:
            raise SidecarUnavailable(
                "client is not connected; call connect() or use 'async with'"
            )
        return self._stub

    @staticmethod
    def _classify_rpc_error(e: grpc.aio.AioRpcError, *, op: str) -> SpendGuardError:
        code = e.code()
        if code in (
            grpc.StatusCode.UNAVAILABLE,
            grpc.StatusCode.DEADLINE_EXCEEDED,
            grpc.StatusCode.CANCELLED,
        ):
            return SidecarUnavailable(
                f"{op} failed: code={code.name} detail={e.details()!r}"
            )
        return SpendGuardError(
            f"{op} failed: code={code.name} detail={e.details()!r}"
        )

    @staticmethod
    def _trigger_for_name(name: str) -> int:
        mapping = {
            "RUN_PRE": adapter_pb2.DecisionRequest.RUN_PRE,
            "AGENT_STEP_PRE": adapter_pb2.DecisionRequest.AGENT_STEP_PRE,
            "LLM_CALL_PRE": adapter_pb2.DecisionRequest.LLM_CALL_PRE,
            "TOOL_CALL_PRE": adapter_pb2.DecisionRequest.TOOL_CALL_PRE,
        }
        try:
            return mapping[name]
        except KeyError as e:
            raise ValueError(f"unknown trigger: {name!r}") from e

    @staticmethod
    def _decision_name(decision_enum: int) -> str:
        mapping = {
            adapter_pb2.DecisionResponse.CONTINUE: "CONTINUE",
            adapter_pb2.DecisionResponse.DEGRADE: "DEGRADE",
            adapter_pb2.DecisionResponse.SKIP: "SKIP",
            adapter_pb2.DecisionResponse.STOP: "STOP",
            adapter_pb2.DecisionResponse.REQUIRE_APPROVAL: "REQUIRE_APPROVAL",
            adapter_pb2.DecisionResponse.STOP_RUN_PROJECTION: "STOP_RUN_PROJECTION",
        }
        return mapping.get(decision_enum, "UNKNOWN")

    @staticmethod
    def _outcome_for_name(name: str) -> int:
        mapping = {
            "APPLIED": adapter_pb2.PublishOutcomeRequest.APPLIED,
            "APPLIED_NOOP": adapter_pb2.PublishOutcomeRequest.APPLIED_NOOP,
            "APPLY_FAILED": adapter_pb2.PublishOutcomeRequest.APPLY_FAILED,
            "APPROVAL_GRANTED": adapter_pb2.PublishOutcomeRequest.APPROVAL_GRANTED,
            "APPROVAL_DENIED": adapter_pb2.PublishOutcomeRequest.APPROVAL_DENIED,
            "APPROVAL_TIMED_OUT": adapter_pb2.PublishOutcomeRequest.APPROVAL_TIMED_OUT,
        }
        try:
            return mapping[name]
        except KeyError as e:
            raise ValueError(f"unknown publish outcome: {name!r}") from e

    @staticmethod
    def _llm_outcome_for_name(name: str) -> int:
        mapping = {
            "SUCCESS": adapter_pb2.LlmCallPostPayload.SUCCESS,
            "PROVIDER_ERROR": adapter_pb2.LlmCallPostPayload.PROVIDER_ERROR,
            "CLIENT_TIMEOUT": adapter_pb2.LlmCallPostPayload.CLIENT_TIMEOUT,
            "RUN_ABORTED": adapter_pb2.LlmCallPostPayload.RUN_ABORTED,
        }
        try:
            return mapping[name]
        except KeyError as e:
            raise ValueError(f"unknown llm outcome: {name!r}") from e


def _raise_proto_error(op: str, err: common_pb2.Error) -> None:
    raise SpendGuardError(
        f"sidecar {op} error code={err.code} message={err.message}"
    )


def _map_reserve_session_outcome(
    resp: adapter_pb2.ReserveSessionOutcome,
) -> ReserveSessionOutcome:
    kind = resp.WhichOneof("outcome")
    if kind == "accepted":
        accepted = resp.accepted
        return ReserveSessionAccepted(
            session_reservation_id=accepted.session_reservation_id,
            ledger_transaction_id=accepted.ledger_transaction_id,
            audit_session_event_id=accepted.audit_session_event_id,
            ttl_expires_at=timestamp_to_datetime(
                accepted.ttl_expires_at
                if accepted.HasField("ttl_expires_at")
                else None
            ),
            reserved_amount_atomic=accepted.reserved_amount_atomic,
            remaining_amount_atomic=accepted.remaining_amount_atomic,
        )
    if kind == "denied":
        denied = resp.denied
        return ReserveSessionDenied(
            audit_session_event_id=denied.audit_session_event_id,
            reason_codes=tuple(denied.reason_codes),
            matched_rule_ids=tuple(denied.matched_rule_ids),
            error=denied.error if denied.HasField("error") else None,
        )
    if kind == "error":
        _raise_proto_error("reserve_session", resp.error)
    raise SpendGuardError("sidecar reserve_session returned empty outcome")


def _map_commit_session_delta_outcome(
    resp: adapter_pb2.CommitSessionDeltaOutcome,
) -> CommitSessionDeltaOutcome:
    kind = resp.WhichOneof("outcome")
    if kind == "accepted":
        accepted = resp.accepted
        return CommitSessionDeltaOutcome(
            session_reservation_id=accepted.session_reservation_id,
            streaming_commit_id=accepted.streaming_commit_id,
            ledger_transaction_id=accepted.ledger_transaction_id,
            audit_session_event_id=accepted.audit_session_event_id,
            committed_delta_atomic=accepted.committed_delta_atomic,
            cumulative_committed_atomic=accepted.cumulative_committed_atomic,
            remaining_amount_atomic=accepted.remaining_amount_atomic,
            recorded_at=timestamp_to_datetime(
                accepted.recorded_at if accepted.HasField("recorded_at") else None
            ),
        )
    if kind == "error":
        _raise_proto_error("commit_session_delta", resp.error)
    raise SpendGuardError("sidecar commit_session_delta returned empty outcome")


def _map_release_session_outcome(
    resp: adapter_pb2.ReleaseSessionOutcome,
) -> ReleaseSessionOutcome:
    kind = resp.WhichOneof("outcome")
    if kind == "accepted":
        accepted = resp.accepted
        return ReleaseSessionOutcome(
            session_reservation_id=accepted.session_reservation_id,
            ledger_transaction_id=accepted.ledger_transaction_id,
            audit_session_event_id=accepted.audit_session_event_id,
            released_amount_atomic=accepted.released_amount_atomic,
            committed_amount_atomic=accepted.committed_amount_atomic,
            recorded_at=timestamp_to_datetime(
                accepted.recorded_at if accepted.HasField("recorded_at") else None
            ),
        )
    if kind == "error":
        _raise_proto_error("release_session", resp.error)
    raise SpendGuardError("sidecar release_session returned empty outcome")
