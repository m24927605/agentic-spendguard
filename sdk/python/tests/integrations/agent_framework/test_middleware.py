# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S101, S106
"""COV_d07_07 — pytest unit + replay-safety tests for the MAF integration.

Mocks ``SpendGuardClient`` (Tier 1 unit-test convention; integration
tests in deploy/demo land in SLICE 8). Verifies every spec contract from
tests.md §2.2 and §3 plus the replay-safety / concurrent-isolation
guarantees called out in review-standards.md §2.3 P2 and §7 N1-N5.
"""

from __future__ import annotations

import asyncio
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

import pytest

# Skip the entire module when `agent-framework` isn't installed (so the
# rest of the SDK test suite still runs in environments without the
# optional extra).
pytest.importorskip(
    "agent_framework",
    reason="agent-framework not installed; install spendguard-sdk[agent-framework]",
)

from agent_framework import (  # noqa: E402
    ChatContext,
    ChatResponse,
    Message,
)

from spendguard.errors import (  # noqa: E402
    DecisionDenied,
    DecisionStopped,
    SidecarUnavailable,
    SpendGuardConfigError,
)
from spendguard.integrations.agent_framework import (  # noqa: E402
    RunContext,
    SpendGuardAgentFrameworkOptions,
    SpendGuardMiddleware,
    SpendGuardToolMiddleware,
    current_run_context,
    run_context,
)

# ---------------------------------------------------------------------------
# Test scaffolding
# ---------------------------------------------------------------------------


def _make_client_mock(
    *,
    tenant_id: str = "tenant-1",
    session_id: str = "session-1",
    decision_id: str = "dec-1",
    reservation_ids: tuple = ("res-1",),
    request_decision_side_effect: Any = None,
):
    """Build an ``AsyncMock`` shaped like a connected SpendGuardClient.

    Tier-1 convention (mirroring tests/test_litellm_precall_unit.py): unit
    tests mock the client. Tier-2/3 tests in deploy/demo use a real sidecar
    stub.
    """
    client = MagicMock()
    client.tenant_id = tenant_id
    client.session_id = session_id

    outcome = SimpleNamespace(
        decision_id=decision_id,
        reservation_ids=reservation_ids,
        audit_decision_event_id="audit-1",
    )
    if request_decision_side_effect is not None:
        client.request_decision = AsyncMock(
            side_effect=request_decision_side_effect
        )
    else:
        client.request_decision = AsyncMock(return_value=outcome)
    client.emit_llm_call_post = AsyncMock(return_value=None)
    client.release_reservation = AsyncMock(return_value=None)
    return client


def _make_options(
    *,
    tenant_id: str = "tenant-1",
    on_sidecar_unavailable: str = "deny",
) -> SpendGuardAgentFrameworkOptions:
    return SpendGuardAgentFrameworkOptions(
        tenant_id=tenant_id,
        budget_id="b1",
        window_instance_id="w1",
        sidecar_socket_path="/tmp/spendguard.sock",  # noqa: S108
        on_sidecar_unavailable=on_sidecar_unavailable,  # type: ignore[arg-type]
    )


_FAKE_CLAIM = SimpleNamespace(
    budget_id="b1",
    window_instance_id="w1",
    amount_atomic="100",
    unit=SimpleNamespace(unit_id="usd_micros"),
)


def _make_middleware(
    *,
    client=None,
    options: SpendGuardAgentFrameworkOptions | None = None,
    claim_estimator=lambda _msgs: [_FAKE_CLAIM],
) -> SpendGuardMiddleware:
    return SpendGuardMiddleware(
        client=client or _make_client_mock(),
        options=options or _make_options(),
        unit=SimpleNamespace(unit_id="usd_micros"),
        pricing=SimpleNamespace(pricing_version="v1"),
        claim_estimator=claim_estimator,
    )


def _make_chat_context(
    *,
    result: Any = None,
    messages: list[Message] | None = None,
) -> ChatContext:
    """Build a populated MAF ``ChatContext``.

    Real MAF runtime constructs the ``ChatContext`` and passes it down
    the middleware chain; in tests we synthesize one + assert what the
    middleware does with it.
    """
    if messages is None:
        messages = [Message(role="user", contents=["Hello"])]
    ctx = ChatContext(
        client=MagicMock(),
        messages=messages,
        options={"model": "gpt-4o-mini"},
    )
    if result is not None:
        ctx.result = result
    return ctx


def _make_ok_response(total_tokens: int = 42) -> ChatResponse:
    """Build a non-streaming ChatResponse with usage metadata populated."""
    return ChatResponse(
        messages=[Message(role="assistant", contents=["ok"])],
        response_id="resp-abc",
        model="gpt-4o-mini",
        usage_details={
            "input_token_count": 10,
            "output_token_count": total_tokens - 10,
            "total_token_count": total_tokens,
        },
    )


async def _populate_result_call_next(ctx: ChatContext, response: ChatResponse):
    """Build a ``call_next`` closure that mutates ``context.result``.

    Mirrors what the MAF inner chat client does: ``call_next()`` does
    not return; the result flows through ``context.result``.
    """

    async def _cn() -> None:
        ctx.result = response

    return _cn


# ---------------------------------------------------------------------------
# SLICE 5 — module skeleton + options validation
# ---------------------------------------------------------------------------


class TestOptionsValidation:
    """Spec: review-standards.md §7 N1, N2 + design.md ADR-005."""

    def test_options_construct_with_defaults(self):
        opts = SpendGuardAgentFrameworkOptions(
            tenant_id="t1", budget_id="b1", window_instance_id="w1"
        )
        # Default sidecar socket path is the standard sidecar location.
        assert opts.sidecar_socket_path == "/var/run/spendguard/sidecar.sock"
        # ADR-005: fail-closed is the default.
        assert opts.on_sidecar_unavailable == "deny"

    def test_options_reject_empty_tenant_id(self):
        with pytest.raises(SpendGuardConfigError, match="tenant_id"):
            SpendGuardAgentFrameworkOptions(
                tenant_id="", budget_id="b1", window_instance_id="w1"
            )

    def test_options_reject_whitespace_tenant_id(self):
        with pytest.raises(SpendGuardConfigError, match="tenant_id"):
            SpendGuardAgentFrameworkOptions(
                tenant_id="   ", budget_id="b1", window_instance_id="w1"
            )

    def test_options_reject_empty_budget_id(self):
        """Review-standards §7 N1 — empty BudgetId rejected at construction."""
        with pytest.raises(SpendGuardConfigError, match="budget_id"):
            SpendGuardAgentFrameworkOptions(
                tenant_id="t1", budget_id="", window_instance_id="w1"
            )

    def test_options_reject_empty_window_instance_id(self):
        with pytest.raises(SpendGuardConfigError, match="window_instance_id"):
            SpendGuardAgentFrameworkOptions(
                tenant_id="t1", budget_id="b1", window_instance_id=""
            )

    def test_options_reject_empty_socket_path(self):
        """Review-standards §7 N2 — empty SocketPath rejected."""
        with pytest.raises(SpendGuardConfigError, match="sidecar_socket_path"):
            SpendGuardAgentFrameworkOptions(
                tenant_id="t1",
                budget_id="b1",
                window_instance_id="w1",
                sidecar_socket_path="",
            )

    def test_options_reject_unknown_unavailable_mode(self):
        with pytest.raises(SpendGuardConfigError, match="on_sidecar_unavailable"):
            SpendGuardAgentFrameworkOptions(
                tenant_id="t1",
                budget_id="b1",
                window_instance_id="w1",
                on_sidecar_unavailable="ignore",  # type: ignore[arg-type]
            )


# ---------------------------------------------------------------------------
# SLICE 6 — middleware lifecycle: ALLOW / DENY / RELEASE / POST
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
class TestMiddlewareAllowPath:
    """Tests.md §2.2 ``test_middleware_allow_invokes_next_and_emits_post``."""

    async def test_allow_invokes_next_and_emits_post(self):
        client = _make_client_mock()
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()
        response = _make_ok_response(total_tokens=137)
        call_next = await _populate_result_call_next(ctx, response)

        async with run_context(RunContext(run_id="run-1")):
            await mw.process(ctx, call_next)

        # PRE happened with LLM_CALL_PRE.
        pre_kwargs = client.request_decision.call_args.kwargs
        assert pre_kwargs["trigger"] == "LLM_CALL_PRE"
        assert pre_kwargs["route"] == "llm.call"
        assert pre_kwargs["run_id"] == "run-1"
        # POST happened and carries the provider-reported usage.
        post_kwargs = client.emit_llm_call_post.call_args.kwargs
        assert post_kwargs["estimated_amount_atomic"] == "137"
        assert post_kwargs["outcome"] == "SUCCESS"
        assert post_kwargs["provider_event_id"] == "resp-abc"
        # Result flows back through ChatContext.
        assert ctx.result is response

    async def test_post_uses_reservation_from_pre(self):
        """POST event must carry the reservation_id minted by PRE."""
        client = _make_client_mock(reservation_ids=("res-xyz",))
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()
        call_next = await _populate_result_call_next(ctx, _make_ok_response())

        async with run_context(RunContext(run_id="run-2")):
            await mw.process(ctx, call_next)

        post_kwargs = client.emit_llm_call_post.call_args.kwargs
        assert post_kwargs["reservation_id"] == "res-xyz"


@pytest.mark.asyncio
class TestMiddlewareDenyPath:
    """Tests.md §2.2 ``test_middleware_deny_short_circuits``."""

    async def test_deny_raises_decision_denied_and_short_circuits(self):
        """DENY → DecisionDenied propagates, call_next NEVER invoked."""
        denied = DecisionDenied(
            "sidecar STOP",
            decision_id="dec-deny-1",
            reason_codes=["budget_exhausted"],
            audit_decision_event_id="audit-deny-1",
            matched_rule_ids=["rule-1"],
        )
        client = _make_client_mock(request_decision_side_effect=denied)
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()
        next_calls: list[int] = []

        async def call_next() -> None:
            next_calls.append(1)

        async with run_context(RunContext(run_id="run-deny-1")):
            with pytest.raises(DecisionDenied) as exc_info:
                await mw.process(ctx, call_next)

        assert exc_info.value.decision_id == "dec-deny-1"
        assert exc_info.value.reason_codes == ["budget_exhausted"]
        assert exc_info.value.matched_rule_ids == ["rule-1"]
        # The MAF inner chat client was NEVER invoked.
        assert next_calls == []
        # POST commit was NEVER emitted.
        client.emit_llm_call_post.assert_not_awaited()

    async def test_decision_stopped_propagates(self):
        """Sidecar STOP -> DecisionStopped subclass surfaces same way."""
        stopped = DecisionStopped(
            "sidecar STOP terminal",
            decision_id="dec-stop-1",
            reason_codes=["policy_violation"],
            audit_decision_event_id="audit-stop-1",
        )
        client = _make_client_mock(request_decision_side_effect=stopped)
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()

        async with run_context(RunContext(run_id="run-stop-1")):
            with pytest.raises(DecisionStopped) as exc_info:
                await mw.process(ctx, lambda: _no_op())

        assert exc_info.value.reason_codes == ["policy_violation"]
        client.emit_llm_call_post.assert_not_awaited()


async def _no_op() -> None:
    pass


@pytest.mark.asyncio
class TestMiddlewareSidecarUnavailable:
    """Tests.md §2.2 + ADR-005: fail-closed default; opt-in fail-open."""

    async def test_sidecar_unavailable_default_raises(self):
        """Default fail-closed: SidecarUnavailable propagates, no call_next."""
        client = _make_client_mock(
            request_decision_side_effect=SidecarUnavailable("UDS down")
        )
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()
        next_calls: list[int] = []

        async def call_next() -> None:
            next_calls.append(1)

        async with run_context(RunContext(run_id="run-down-1")):
            with pytest.raises(SidecarUnavailable):
                await mw.process(ctx, call_next)

        assert next_calls == []
        # 503 status_code attribute preserved on the exception for HTTP wrap.
        with pytest.raises(SidecarUnavailable) as exc_info:
            raise SidecarUnavailable("UDS down")
        assert getattr(exc_info.value, "status_code", None) == 503

    async def test_sidecar_unavailable_allow_proceeds_with_warning(self, caplog):
        """opt-in 'allow' fail-open: call_next runs + warning logged."""
        client = _make_client_mock(
            request_decision_side_effect=SidecarUnavailable("UDS down")
        )
        mw = _make_middleware(
            client=client,
            options=_make_options(on_sidecar_unavailable="allow"),
        )
        ctx = _make_chat_context()
        ran = []

        async def call_next() -> None:
            ran.append(1)

        with caplog.at_level("WARNING"):
            async with run_context(RunContext(run_id="run-degrade-1")):
                await mw.process(ctx, call_next)

        assert ran == [1]
        # POST commit NOT emitted (no reservation was obtained).
        client.emit_llm_call_post.assert_not_awaited()
        # Warning was logged.
        assert any("sidecar unavailable" in r.message.lower() for r in caplog.records)


@pytest.mark.asyncio
class TestMiddlewareInnerError:
    """Tests.md §2.2 ``test_middleware_next_raises_releases_reservation``."""

    async def test_inner_raises_releases_reservation_and_propagates(self):
        client = _make_client_mock(reservation_ids=("res-fail-1",))
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()

        class ProviderError(RuntimeError):
            pass

        async def call_next() -> None:
            raise ProviderError("provider 500")

        async with run_context(RunContext(run_id="run-fail-1")):
            with pytest.raises(ProviderError, match="provider 500"):
                await mw.process(ctx, call_next)

        # Release was called for the reserved id.
        client.release_reservation.assert_awaited_once()
        release_kwargs = client.release_reservation.call_args.kwargs
        assert release_kwargs["reservation_id"] == "res-fail-1"
        assert "runtime_error" in release_kwargs["reason_codes"]
        # POST commit NOT emitted on inner failure.
        client.emit_llm_call_post.assert_not_awaited()

    async def test_inner_raises_release_failure_does_not_mask_original(self):
        """Release best-effort: a release exception MUST NOT mask the original."""
        client = _make_client_mock(reservation_ids=("res-mask-1",))
        client.release_reservation = AsyncMock(
            side_effect=RuntimeError("release rpc 500")
        )
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()

        async def call_next() -> None:
            raise ValueError("original provider err")

        async with run_context(RunContext(run_id="run-mask-1")):
            with pytest.raises(ValueError, match="original provider err"):
                await mw.process(ctx, call_next)


# ---------------------------------------------------------------------------
# Token usage extraction
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
class TestUsageExtraction:
    async def test_extract_total_token_count_from_usage_details(self):
        client = _make_client_mock()
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()
        call_next = await _populate_result_call_next(
            ctx, _make_ok_response(total_tokens=512)
        )

        async with run_context(RunContext(run_id="run-tok-1")):
            await mw.process(ctx, call_next)

        post_kwargs = client.emit_llm_call_post.call_args.kwargs
        assert post_kwargs["estimated_amount_atomic"] == "512"

    async def test_extract_falls_back_to_zero_when_usage_missing(self):
        client = _make_client_mock()
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()
        # Response with no usage_details at all.
        empty = ChatResponse(
            messages=[Message(role="assistant", contents=["ok"])],
            response_id="resp-empty",
        )
        call_next = await _populate_result_call_next(ctx, empty)

        async with run_context(RunContext(run_id="run-tok-2")):
            await mw.process(ctx, call_next)

        post_kwargs = client.emit_llm_call_post.call_args.kwargs
        assert post_kwargs["estimated_amount_atomic"] == "0"

    async def test_extract_sums_input_plus_output_when_total_missing(self):
        client = _make_client_mock()
        mw = _make_middleware(client=client)
        ctx = _make_chat_context()
        # Provider gave individual counts but no total_token_count.
        partial = ChatResponse(
            messages=[Message(role="assistant", contents=["ok"])],
            response_id="resp-partial",
            usage_details={"input_token_count": 30, "output_token_count": 70},
        )
        call_next = await _populate_result_call_next(ctx, partial)

        async with run_context(RunContext(run_id="run-tok-3")):
            await mw.process(ctx, call_next)

        post_kwargs = client.emit_llm_call_post.call_args.kwargs
        assert post_kwargs["estimated_amount_atomic"] == "100"


# ---------------------------------------------------------------------------
# Run context
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
class TestRunContext:
    """Tests.md §2.2 ``test_run_context_required`` + concurrent isolation."""

    async def test_current_run_context_outside_run_context_raises(self):
        with pytest.raises(RuntimeError, match="run_context"):
            current_run_context()

    async def test_middleware_outside_run_context_raises(self):
        """Tests.md §2.2: error is the framework's, not silent fallthrough."""
        mw = _make_middleware()
        ctx = _make_chat_context()
        with pytest.raises(RuntimeError, match="run_context"):
            await mw.process(ctx, _no_op)

    async def test_run_context_isolated_per_async_task(self):
        """Per-async-task isolation via contextvars.

        Two concurrent ``asyncio.gather`` branches MUST see their own
        ``RunContext`` value, not stomp on each other.
        """
        client = _make_client_mock()
        seen: dict[str, str] = {}

        async def branch(run_id: str) -> None:
            async with run_context(RunContext(run_id=run_id)):
                # Tiny sleep forces the event loop to interleave.
                await asyncio.sleep(0)
                seen[run_id] = current_run_context().run_id

        await asyncio.gather(branch("run-A"), branch("run-B"), branch("run-C"))
        assert seen == {"run-A": "run-A", "run-B": "run-B", "run-C": "run-C"}
        # Sanity: client was untouched (branch() didn't actually call middleware).
        client.request_decision.assert_not_awaited()

    async def test_multiple_middleware_instances_independent(self):
        """Two middlewares operating on different clients don't share state."""
        client_a = _make_client_mock(tenant_id="tenant-a")
        client_b = _make_client_mock(tenant_id="tenant-b")
        mw_a = _make_middleware(
            client=client_a, options=_make_options(tenant_id="tenant-a")
        )
        mw_b = _make_middleware(
            client=client_b, options=_make_options(tenant_id="tenant-b")
        )
        ctx_a = _make_chat_context()
        ctx_b = _make_chat_context()
        call_next_a = await _populate_result_call_next(ctx_a, _make_ok_response(11))
        call_next_b = await _populate_result_call_next(ctx_b, _make_ok_response(22))

        async with run_context(RunContext(run_id="run-multi-1")):
            await asyncio.gather(
                mw_a.process(ctx_a, call_next_a),
                mw_b.process(ctx_b, call_next_b),
            )

        # Each client got exactly its own commit; no cross-contamination.
        assert client_a.emit_llm_call_post.await_count == 1
        assert client_b.emit_llm_call_post.await_count == 1
        post_a = client_a.emit_llm_call_post.call_args.kwargs
        post_b = client_b.emit_llm_call_post.call_args.kwargs
        assert {post_a["estimated_amount_atomic"], post_b["estimated_amount_atomic"]} == {
            "11",
            "22",
        }


# ---------------------------------------------------------------------------
# Replay safety / idempotency
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
class TestReplaySafety:
    """Tests.md §3 ``Middleware_Replay_Idempotent`` (parity with .NET)."""

    async def test_same_call_same_idempotency_key(self):
        """Same (run_id, message-content) → same idempotency_key.

        Two invocations of ``process()`` with identical ``ChatContext``
        produce identical ``request_decision`` payloads — which is what
        makes the sidecar replay cache short-circuit and prevents
        double-reservation when MAF retry middleware re-enters.
        """
        client = _make_client_mock()
        mw = _make_middleware(client=client)

        keys: list[str] = []
        for _ in range(2):
            ctx = _make_chat_context()
            call_next = await _populate_result_call_next(ctx, _make_ok_response())
            async with run_context(RunContext(run_id="run-replay-1")):
                await mw.process(ctx, call_next)
            keys.append(client.request_decision.call_args.kwargs["idempotency_key"])

        assert keys[0] == keys[1]
        assert keys[0].startswith("sg-")

    async def test_different_runs_different_keys(self):
        """Different run_id → different idempotency_key (no spurious replay)."""
        client = _make_client_mock()
        mw = _make_middleware(client=client)
        keys: list[str] = []

        for run_id in ("r-A", "r-B"):
            ctx = _make_chat_context()
            call_next = await _populate_result_call_next(ctx, _make_ok_response())
            async with run_context(RunContext(run_id=run_id)):
                await mw.process(ctx, call_next)
            keys.append(client.request_decision.call_args.kwargs["idempotency_key"])

        assert keys[0] != keys[1]

    async def test_different_messages_different_keys(self):
        """Different message content → different idempotency_key."""
        client = _make_client_mock()
        mw = _make_middleware(client=client)
        keys: list[str] = []

        for content in ("hello", "goodbye"):
            ctx = _make_chat_context(
                messages=[Message(role="user", contents=[content])]
            )
            call_next = await _populate_result_call_next(ctx, _make_ok_response())
            async with run_context(RunContext(run_id="run-msg-diff")):
                await mw.process(ctx, call_next)
            keys.append(client.request_decision.call_args.kwargs["idempotency_key"])

        assert keys[0] != keys[1]


# ---------------------------------------------------------------------------
# Configuration validation at middleware construction
# ---------------------------------------------------------------------------


class TestMiddlewareConstruction:
    """Reviewer N1 / N2: configuration errors must fail early + clearly."""

    def test_construct_without_client_raises(self):
        with pytest.raises(SpendGuardConfigError, match="client"):
            SpendGuardMiddleware(
                client=None,  # type: ignore[arg-type]
                options=_make_options(),
                unit=SimpleNamespace(),
                pricing=SimpleNamespace(),
            )

    def test_construct_with_wrong_options_type_raises(self):
        with pytest.raises(SpendGuardConfigError, match="options"):
            SpendGuardMiddleware(
                client=_make_client_mock(),
                options={"tenant_id": "t1"},  # type: ignore[arg-type]
                unit=SimpleNamespace(),
                pricing=SimpleNamespace(),
            )

    def test_construct_with_tenant_mismatch_raises(self):
        """Review-standards §7 N4 — client tenant must agree with options.tenant_id."""
        client = _make_client_mock(tenant_id="tenant-A")
        opts = _make_options(tenant_id="tenant-B")
        with pytest.raises(SpendGuardConfigError, match="tenant_id"):
            SpendGuardMiddleware(
                client=client,
                options=opts,
                unit=SimpleNamespace(),
                pricing=SimpleNamespace(),
            )


@pytest.mark.asyncio
class TestMiddlewareEstimatorEnforcement:
    async def test_missing_estimator_raises_when_called(self):
        """No default estimator in v1 — surfaces as ConfigError at call time."""
        client = _make_client_mock()
        mw = SpendGuardMiddleware(
            client=client,
            options=_make_options(),
            unit=SimpleNamespace(),
            pricing=SimpleNamespace(),
            claim_estimator=None,
        )
        ctx = _make_chat_context()
        async with run_context(RunContext(run_id="run-estimator-1")):
            with pytest.raises(SpendGuardConfigError, match="claim_estimator"):
                await mw.process(ctx, _no_op)
        # Sidecar wasn't even called — fail-before-the-wire.
        client.request_decision.assert_not_awaited()


# ---------------------------------------------------------------------------
# Tool middleware (opt-in, ADR-002)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
class TestToolMiddleware:
    async def test_tool_middleware_uses_tool_call_pre_trigger(self):
        client = _make_client_mock()
        mw = SpendGuardToolMiddleware(
            client=client,
            options=_make_options(),
            unit=SimpleNamespace(unit_id="usd_micros"),
            pricing=SimpleNamespace(pricing_version="v1"),
            claim_estimator=lambda _fn, _args: [_FAKE_CLAIM],
        )

        fn = SimpleNamespace(name="lookup_balance")
        args = {"user": "alice"}
        ctx = SimpleNamespace(function=fn, arguments=args, result=None)

        ran: list[int] = []

        async def call_next() -> None:
            ran.append(1)

        async with run_context(RunContext(run_id="run-tool-1")):
            await mw.process(ctx, call_next)

        assert ran == [1]
        pre_kwargs = client.request_decision.call_args.kwargs
        # ADR-003: TOOL_CALL_PRE trigger, not LLM_CALL_PRE.
        assert pre_kwargs["trigger"] == "TOOL_CALL_PRE"
        # tool_call_id is set, llm_call_id is empty.
        assert pre_kwargs["tool_call_id"]
        assert pre_kwargs["llm_call_id"] == ""

    async def test_tool_middleware_requires_estimator(self):
        with pytest.raises(SpendGuardConfigError, match="claim_estimator"):
            SpendGuardToolMiddleware(
                client=_make_client_mock(),
                options=_make_options(),
                unit=SimpleNamespace(),
                pricing=SimpleNamespace(),
                claim_estimator=None,  # type: ignore[arg-type]
            )

    async def test_tool_middleware_sidecar_unavailable_default_raises(self):
        client = _make_client_mock(
            request_decision_side_effect=SidecarUnavailable("UDS down")
        )
        mw = SpendGuardToolMiddleware(
            client=client,
            options=_make_options(),
            unit=SimpleNamespace(),
            pricing=SimpleNamespace(),
            claim_estimator=lambda _fn, _args: [_FAKE_CLAIM],
        )
        fn = SimpleNamespace(name="t")
        ctx = SimpleNamespace(function=fn, arguments={}, result=None)
        async with run_context(RunContext(run_id="run-tool-down")):
            with pytest.raises(SidecarUnavailable):
                await mw.process(ctx, _no_op)

    async def test_tool_middleware_deny_short_circuits(self):
        denied = DecisionDenied(
            "tool denied",
            decision_id="dec-tool-deny",
            reason_codes=["tool_blocked"],
        )
        client = _make_client_mock(request_decision_side_effect=denied)
        mw = SpendGuardToolMiddleware(
            client=client,
            options=_make_options(),
            unit=SimpleNamespace(),
            pricing=SimpleNamespace(),
            claim_estimator=lambda _fn, _args: [_FAKE_CLAIM],
        )
        ran: list[int] = []

        async def call_next() -> None:
            ran.append(1)

        ctx = SimpleNamespace(function=SimpleNamespace(name="t"), arguments={}, result=None)
        async with run_context(RunContext(run_id="run-tool-deny")):
            with pytest.raises(DecisionDenied):
                await mw.process(ctx, call_next)
        assert ran == []


# ─────────────────────────────────────────────────────────────────────
# HARDEN_D05_UR — TP-01..03: `unit_id` options field threading.
#
# Per docs/specs/harden_d05_unit_ref/tests.md §2.2, every Python adapter
# in the sweep MUST expose an optional ``unit_id`` on its options
# dataclass and (a) accept it at construction, (b) thread it onto the
# wire ``BudgetClaim.unit.unit_id``, and (c) keep constructing when the
# field is omitted (backward compat).
# ─────────────────────────────────────────────────────────────────────

_UNIT_ID_FIXTURE = "550e8400-e29b-41d4-a716-446655440000"


def test_TP01_options_accepts_unit_id() -> None:
    """TP-01 — ``SpendGuardAgentFrameworkOptions(unit_id=...)`` constructs."""
    opts = SpendGuardAgentFrameworkOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
        unit_id=_UNIT_ID_FIXTURE,
    )
    assert opts.unit_id == _UNIT_ID_FIXTURE


@pytest.mark.asyncio
async def test_TP02_unit_id_threads_to_wire_claim() -> None:
    """TP-02 — operator binds ``options.unit_id`` to the proto ``UnitRef``;
    the resulting wire ``BudgetClaim.unit.unit_id`` carries it verbatim.
    """
    opts = SpendGuardAgentFrameworkOptions(
        tenant_id="tenant-1",
        budget_id="b1",
        window_instance_id="w1",
        unit_id=_UNIT_ID_FIXTURE,
    )
    fake_claim = SimpleNamespace(
        budget_id="b1",
        window_instance_id="w1",
        amount_atomic="100",
        unit=SimpleNamespace(unit_id=opts.unit_id or ""),
    )
    client = _make_client_mock()
    mw = SpendGuardMiddleware(
        client=client,
        options=opts,
        unit=SimpleNamespace(unit_id=opts.unit_id or ""),
        pricing=SimpleNamespace(pricing_version="v1"),
        claim_estimator=lambda _msgs: [fake_claim],
    )
    ctx = _make_chat_context()
    response = _make_ok_response(total_tokens=42)
    call_next = await _populate_result_call_next(ctx, response)
    async with run_context(RunContext(run_id="run-tp02")):
        await mw.process(ctx, call_next)
    kw = client.request_decision.call_args.kwargs
    assert kw["projected_claims"][0].unit.unit_id == _UNIT_ID_FIXTURE


def test_TP03_options_without_unit_id_constructs() -> None:
    """TP-03 — backward compat: omitting ``unit_id`` keeps default None."""
    opts = SpendGuardAgentFrameworkOptions(
        tenant_id="t1",
        budget_id="b1",
        window_instance_id="w1",
    )
    assert opts.unit_id is None
