"""SpendGuard MAF chat-middleware — gates ``ChatClient.get_response`` via sidecar.

Implements the ``agent_framework.ChatMiddleware`` abstract base. Each
``agent.run()`` -> ``ChatClient.get_response()`` boundary flows through
``process(context, call_next)``:

  1. PRE — derive idempotency key, call ``RequestDecision(LLM_CALL_PRE)``.
  2. Decision branching:
       * ``CONTINUE`` / ``DEGRADE`` → invoke ``await call_next()``.
       * ``STOP`` / ``STOP_RUN_PROJECTION`` → ``DecisionStopped`` propagates.
       * ``SKIP`` → ``DecisionSkipped`` propagates.
       * ``REQUIRE_APPROVAL`` → ``ApprovalRequired`` propagates.
  3. INNER — ``await call_next()`` runs the actual chat client.
       * If ``call_next`` raises, RELEASE the reservation, propagate.
  4. POST — emit ``LLM_CALL_POST`` with real provider usage.

Sidecar-unavailable handling per ADR-005 (fail-closed default, opt-in fail-open).

Reviewer pinning (review-standards.md):
  - S1: only ``LLM_CALL_PRE`` trigger is used in pre-gate path.
  - S2: POST event carries the provider's reported ``UsageDetails``, not a
    re-estimate.
  - S3: ``handshake()`` must precede ``RequestDecision`` — enforced by
    ``SpendGuardClient`` itself (raises if not handshook).
  - S4: ``call_next`` exceptions → ``release_reservation`` before re-raise.
  - D4: this class only gates LLM calls; tool gating is the separate
    ``SpendGuardToolMiddleware`` class.
  - D5: fail-closed is the default; ``allow`` requires explicit opt-in.
"""

from __future__ import annotations

import hashlib
import logging
from collections.abc import Awaitable, Callable, Sequence
from typing import TYPE_CHECKING, Any

from ...client import DecisionOutcome, SpendGuardClient
from ...ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)
from ._errors import SidecarUnavailable, SpendGuardConfigError
from ._options import SpendGuardAgentFrameworkOptions
from ._run_context import current_run_context

try:
    from agent_framework import ChatContext, ChatMiddleware
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.agent_framework requires the "
        "[agent-framework] extra. Install with: "
        "pip install 'spendguard-sdk[agent-framework]'"
    ) from exc

# Validate proto stubs were built without binding the symbol (we forward
# the proto types via the `unit` / `pricing` callable parameters; the
# explicit import keeps a clean error if the build step was skipped).
try:
    from spendguard._proto.spendguard.common.v1 import common_pb2 as _common_pb2  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc

if TYPE_CHECKING:
    pass


logger = logging.getLogger(__name__)


# A claim estimator: takes the MAF messages list, returns a list of
# proto BudgetClaim. Matches the langchain / openai_agents shape per
# review-standards.md §2.3 P2 (case-translated naming parity).
ClaimEstimator = Callable[[Sequence[Any]], list[Any]]


def _serialize_content_part(part: Any) -> str:
    """Stable string form of a single MAF ``Content`` part.

    MAF wraps each content fragment in a ``Content`` object whose default
    ``__repr__`` exposes the Python ``id(...)`` — useless for hashing.
    We pull the canonical fields out via the public surface:

      * Plain text → ``Content.text`` (most common path).
      * Tool/function/image/etc. → ``Content.to_dict()`` if available
        (covers every public ``Content`` subtype).
      * Bare string (passed in by callers using ``contents=["foo"]``
        which MAF coerces on construction) → the string itself.

    Returns a deterministic string even when ``part`` is a bare ``str``,
    a ``Content`` with text, or any other ``Content`` shape.
    """
    if isinstance(part, str):
        return part
    text = getattr(part, "text", None)
    if isinstance(text, str) and text:
        return f"text:{text}"
    to_dict = getattr(part, "to_dict", None)
    if callable(to_dict):
        try:
            return f"dict:{to_dict()!r}"
        except Exception:  # noqa: BLE001
            # Fall through to ``repr`` — any failure to canonicalise is
            # handled by the deterministic ``repr`` fallback below.
            logger.debug(
                "MAF Content.to_dict() raised during signature hashing; "
                "falling back to repr"
            )
    return repr(part)


def _signature(messages: Sequence[Any], options: Any) -> str:
    """Stable content hash over messages + options for ID derivation.

    Mirrors langchain's ``_default_call_signature`` so cross-adapter
    behaviour is symmetric: same logical call → same idempotency_key →
    sidecar cache hit on retry.

    The hash is taken over a canonicalised projection of ``contents``
    rather than ``repr(messages)`` because ``Message.__repr__`` includes
    the Python object id which mints a new value every call.
    """
    parts: list[str] = []
    for i, msg in enumerate(messages):
        role = getattr(msg, "role", "?")
        # role may be a Role enum; use .value or .name when present.
        role_str = getattr(role, "value", None) or getattr(role, "name", None) or str(role)
        contents = getattr(msg, "contents", None) or getattr(msg, "content", "")
        if isinstance(contents, (list, tuple)):
            content_str = "|".join(_serialize_content_part(p) for p in contents)
        else:
            content_str = _serialize_content_part(contents)
        parts.append(f"msg{i}|{role_str}|{content_str}")
    # ``options`` is a Mapping — sort keys for stability.
    if isinstance(options, dict):
        opt_items = sorted(options.items(), key=lambda kv: kv[0])
        opt_str = "|".join(f"{k}={v!r}" for k, v in opt_items)
    else:
        opt_str = repr(options)
    parts.append(f"options|{opt_str}")
    payload = "\x1f".join(parts)
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()


def _extract_total_tokens(response: Any) -> int:
    """Read ``total_token_count`` from a MAF ``ChatResponse`` if present.

    Falls back across multiple shapes because MAF 1.x ``ChatResponse``
    has a ``usage_details: UsageDetails | None`` dict and individual
    providers sometimes still surface counts on ``response.raw_representation``.
    Returns 0 when no count is discoverable.
    """
    usage = getattr(response, "usage_details", None)
    if isinstance(usage, dict):
        total = usage.get("total_token_count")
        if isinstance(total, int):
            return total
        # Fallback: sum input + output if present individually.
        inp = usage.get("input_token_count") or 0
        out = usage.get("output_token_count") or 0
        if isinstance(inp, int) and isinstance(out, int) and (inp + out) > 0:
            return inp + out
    return 0


def _extract_response_id(response: Any) -> str:
    """Extract a provider-side identifier for POST event correlation."""
    return getattr(response, "response_id", None) or ""


class SpendGuardMiddleware(ChatMiddleware):  # type: ignore[misc, valid-type]
    """MAF ChatMiddleware that gates each chat-client call through SpendGuard.

    Subclass of ``agent_framework.ChatMiddleware``; the MAF runtime
    invokes ``process(context, call_next)`` once per LLM boundary.

    Lifecycle (per the design.md §3.1 sequence diagram):

        PRE  → RequestDecision(LLM_CALL_PRE)
              ├ CONTINUE / DEGRADE → call_next()
              ├ STOP / SKIP / REQUIRE_APPROVAL → raises typed exception
        CALL → context.result populated by inner chat client
              ├ exception inside next() → release_reservation, re-raise
        POST → emit_llm_call_post(SUCCESS, real_usage)

    Args:
        client: A connected + handshook ``SpendGuardClient``.
        options: ``SpendGuardAgentFrameworkOptions`` (tenant_id, budget_id,
            window_instance_id, sidecar_socket_path, on_sidecar_unavailable).
        unit: ``common_pb2.UnitRef`` describing the token unit.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup.
        claim_estimator: Optional ``(messages) -> [BudgetClaim]`` callable.
            When None, raises ``SpendGuardConfigError`` at first call (v1
            does not auto-derive from MAF model metadata; the .NET side
            already requires the same).
        route: Decision route string (default ``"llm.call"``).
    """

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        options: SpendGuardAgentFrameworkOptions,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator | None = None,
        route: str = "llm.call",
    ) -> None:
        # Don't call super().__init__() — ChatMiddleware is an ABC with no
        # shared state, so it has no required __init__ contract. Calling
        # super would just be a no-op object.__init__().
        if client is None:
            raise SpendGuardConfigError(
                "SpendGuardMiddleware(client=...) is required; got None."
            )
        if not isinstance(options, SpendGuardAgentFrameworkOptions):
            raise SpendGuardConfigError(
                "SpendGuardMiddleware(options=...) must be a "
                "SpendGuardAgentFrameworkOptions instance."
            )
        # Defensive guard against a client wired to a different tenant
        # than the options declare; review-standards §7 N4.
        if client.tenant_id and client.tenant_id != options.tenant_id:
            raise SpendGuardConfigError(
                f"client.tenant_id={client.tenant_id!r} disagrees with "
                f"options.tenant_id={options.tenant_id!r}."
            )
        self._client = client
        self._options = options
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._route = route

    # MAF abstract surface.
    async def process(
        self,
        context: ChatContext,
        call_next: Callable[[], Awaitable[None]],
    ) -> None:
        """MAF middleware entry point — see class docstring for the flow."""
        run_ctx = current_run_context()
        if self._claim_estimator is None:
            raise SpendGuardConfigError(
                "SpendGuardMiddleware requires claim_estimator=... at "
                "construction (no default estimator is dispatched for MAF "
                "in v1; the .NET side has the same constraint)."
            )

        signature = _signature(context.messages, context.options)
        llm_call_id = str(
            derive_uuid_from_signature(signature, scope="llm_call_id")
        )
        decision_id = str(
            derive_uuid_from_signature(signature, scope="decision_id")
        )
        step_id = f"{run_ctx.run_id}:maf-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        # ---- PRE: reserve ---------------------------------------------------
        try:
            outcome: DecisionOutcome = await self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_ctx.run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                tool_call_id="",
                decision_id=decision_id,
                route=self._route,
                projected_claims=self._claim_estimator(context.messages),
                idempotency_key=idempotency_key,
            )
        except SidecarUnavailable:
            if self._options.on_sidecar_unavailable == "allow":
                logger.warning(
                    "SpendGuard sidecar unavailable; "
                    "on_sidecar_unavailable='allow' — proceeding without "
                    "a reservation (NO audit row will be produced for this "
                    "call). run_id=%s",
                    run_ctx.run_id,
                )
                await call_next()
                return
            # Default fail-closed (ADR-005): re-raise so caller sees it.
            raise

        # ---- CALL: invoke inner chat client --------------------------------
        try:
            await call_next()
        except BaseException as inner_exc:
            # S4: release the reservation before letting the exception propagate
            # so the budget isn't pinned by a failed call.
            if outcome.reservation_ids:
                try:
                    await self._client.release_reservation(
                        reservation_id=outcome.reservation_ids[0],
                        idempotency_key=derive_idempotency_key(
                            tenant_id=self._client.tenant_id,
                            session_id=self._client.session_id,
                            run_id=run_ctx.run_id,
                            step_id=step_id,
                            llm_call_id=llm_call_id,
                            trigger="LLM_CALL_RELEASE",
                        ),
                        reason_codes=("runtime_error",),
                        tenant_id=self._client.tenant_id,
                    )
                except Exception as release_exc:  # noqa: BLE001
                    # Releasing best-effort — swallow + log so we don't mask
                    # the original error.
                    logger.warning(
                        "SpendGuard release_reservation after inner error "
                        "failed: %s (original error preserved)",
                        release_exc,
                    )
            raise inner_exc

        # ---- POST: commit ---------------------------------------------------
        if outcome.reservation_ids:
            total_tokens = _extract_total_tokens(context.result)
            provider_event_id = _extract_response_id(context.result)
            await self._client.emit_llm_call_post(
                run_id=run_ctx.run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=outcome.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(total_tokens),
                unit=self._unit,
                pricing=self._pricing,
                provider_event_id=provider_event_id,
                outcome="SUCCESS",
            )


__all__ = [
    "ClaimEstimator",
    "SpendGuardMiddleware",
]
