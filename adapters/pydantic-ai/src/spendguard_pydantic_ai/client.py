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
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

import grpc

logger = logging.getLogger(__name__)

# Generated proto stubs — produced by `make proto`. Importing here gives a
# clear error at module load if the build step was skipped.
try:
    from spendguard_pydantic_ai._proto.spendguard.common.v1 import common_pb2
    from spendguard_pydantic_ai._proto.spendguard.sidecar_adapter.v1 import (
        adapter_pb2,
        adapter_pb2_grpc,
    )
except ImportError as exc:  # pragma: no cover — build configuration error
    raise ImportError(
        "spendguard_pydantic_ai proto stubs missing. "
        "Run `make proto` from adapters/pydantic-ai/ first."
    ) from exc

from .errors import (
    ApprovalRequired,
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    SidecarUnavailable,
    SpendGuardError,
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
        inputs = adapter_pb2.DecisionRequest.Inputs(
            projected_claims=list(projected_claims),
            projected_p50_atomic=projected_p50_atomic,
            projected_p90_atomic=projected_p90_atomic,
            projected_p95_atomic=projected_p95_atomic,
            projected_p99_atomic=projected_p99_atomic,
            projected_unit=projected_unit or common_pb2.UnitRef(),
        )
        idem = common_pb2.Idempotency(
            key=idempotency_key,
            request_hash=b"",  # let sidecar/ledger own canonical
        )
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
        if decision_name == "STOP":
            raise DecisionStopped(
                f"sidecar STOP terminal={resp.terminal} reasons={list(resp.reason_codes)}",
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
        payload = adapter_pb2.LlmCallPostPayload(
            reservation_id=reservation_id,
            provider_reported_amount_atomic=provider_reported_amount_atomic,
            estimated_amount_atomic=estimated_amount_atomic,
            unit=unit,
            pricing=pricing,
            provider_event_id=provider_event_id,
            outcome=outcome_enum,
        )
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
