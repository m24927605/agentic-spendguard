# ruff: noqa: ANN001, ANN201, ANN202, ANN003, ANN401, S106
"""D12 SLICE 5 — 22 additional unit tests for the LiteLLM SDK shim.

Extends ``test_install.py`` (Slices 1-4, 17 tests). Each test below
covers a behavioural axis from ``tests.md`` §2 + §6 + §7 that the
install-and-patch tests did not pin down on their own:

* §2.3 INV-2 strict ordering via pytest-httpx-style call-order list.
* §2.3 DENY / DEGRADE / fail_open branches.
* §2.4 recursion + per-task contextvar already covered in install
  tests; here we extend with timing-sensitive ordering across asyncio.
* §2.5 commit-side reconciliation, exception → release, cancellation,
  no-usage fallback.
* §6 / §7 — `decision_context` shape carries the ``mode='sdk'`` literal,
  multi-provider routing, idempotency key derivation.

Every test wraps ``install_shim()`` in the ``shim_clean`` fixture so
global state never leaks (review-standards §10).
"""

from __future__ import annotations

import asyncio
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

pytest.importorskip("litellm", reason="LiteLLM not installed")

import litellm  # noqa: E402

from spendguard.errors import DecisionDenied, SidecarUnavailable  # noqa: E402
from spendguard.integrations.litellm_sdk_shim import (  # noqa: E402
    SpendGuardShimOptions,
    SpendGuardShimSyncInAsyncContext,
    install_shim,
    is_installed,
    uninstall_shim,
)

# ---------------------------------------------------------------------------
# Shared helpers (mirrors test_install.py so the two files stay in lockstep)
# ---------------------------------------------------------------------------


def _fake_client(
    *,
    tenant: str = "tenant-1",
    decision: str = "CONTINUE",
    decision_id: str = "dec-1",
    reservation_ids: tuple[str, ...] = ("res-1",),
    request_side_effect=None,
    commit_side_effect=None,
) -> MagicMock:
    cli = MagicMock()
    cli.tenant_id = tenant
    cli.session_id = "session-1"
    if request_side_effect is not None:
        cli.request_decision = AsyncMock(side_effect=request_side_effect)
    else:
        cli.request_decision = AsyncMock(return_value=SimpleNamespace(
            decision=decision,
            decision_id=decision_id,
            reservation_ids=reservation_ids,
            audit_decision_event_id="audit-1",
        ))
    if commit_side_effect is not None:
        cli.emit_llm_call_post = AsyncMock(side_effect=commit_side_effect)
    else:
        cli.emit_llm_call_post = AsyncMock(return_value=None)
    return cli


@pytest.fixture
def shim_clean():
    """Mandatory test-isolation fixture (tests.md §10)."""
    yield
    if is_installed():
        uninstall_shim()


def _make_options(client: MagicMock, *, fail_open: bool = False) -> SpendGuardShimOptions:
    return SpendGuardShimOptions(
        client=client,
        tenant_id=client.tenant_id,
        budget_id="b1",
        fail_open=fail_open,
    )


# ---------------------------------------------------------------------------
# §2.3 (U17) — Strict-order: pytest-httpx-style call recorder
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_u17a_reserve_strictly_before_provider_httpx_style(
    monkeypatch, shim_clean,
):
    """U17 (LOAD-BEARING): provider HTTP wrapper records ``"http"`` AFTER
    the sidecar ``request_decision`` records ``"reserve"`` — proves INV-2
    holds even under tight asyncio scheduling.

    Models the pytest-httpx wire-recorder pattern from tests.md §2.3:
    the order list MUST start with ``"reserve"``; an empty list or a
    list starting with ``"http"`` means the shim let the provider HTTP
    leave before the sidecar said yes.
    """
    order: list[str] = []

    async def _recorded_reserve(**_kw):
        # Yield to other tasks so we surface any "shim called provider
        # without awaiting reserve" bug — a buggy shim would let the
        # provider record before this point returns.
        await asyncio.sleep(0)
        order.append("reserve")
        return SimpleNamespace(
            decision="CONTINUE",
            decision_id="dec-1",
            reservation_ids=("res-1",),
            audit_decision_event_id="audit-1",
        )

    async def _recorded_provider(**_kw):
        order.append("http")
        return SimpleNamespace(
            id="chatcmpl-x",
            usage=SimpleNamespace(prompt_tokens=10, completion_tokens=42),
        )

    client = _fake_client(request_side_effect=_recorded_reserve)
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(side_effect=_recorded_provider))
    install_shim(_make_options(client))
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "u17a"}],
    )
    assert order == ["reserve", "http"], (
        f"INV-2 broken: ordering was {order!r}"
    )


# ---------------------------------------------------------------------------
# §2.3 (U18) — DENY raises DecisionDenied; provider NEVER called
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_u18_deny_raises_decisiondenied_and_zero_provider_calls(
    monkeypatch, shim_clean,
):
    """U18: sidecar raises ``DecisionDenied`` → propagates untouched +
    the original provider callable records ZERO calls (INV-1)."""

    async def _reserve_denies(**_kw):
        raise DecisionDenied(
            "budget exhausted",
            decision_id="dec-deny",
            reason_codes=["BUDGET_EXHAUSTED"],
        )

    provider = AsyncMock(return_value=SimpleNamespace(
        id="should-not-happen",
        usage=SimpleNamespace(prompt_tokens=10, completion_tokens=42),
    ))
    client = _fake_client(request_side_effect=_reserve_denies)
    monkeypatch.setattr(litellm, "acompletion", provider)
    install_shim(_make_options(client))
    with pytest.raises(DecisionDenied) as ei:
        await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "deny"}],
        )
    assert "BUDGET_EXHAUSTED" in ei.value.reason_codes
    assert provider.call_count == 0, (
        "INV-1 broken: DENY must not reach the provider"
    )


# ---------------------------------------------------------------------------
# §2.3 (U19) — DEGRADE fails closed by default
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_u19_degrade_fails_closed_by_default(monkeypatch, shim_clean):
    """U19: sidecar returns DEGRADE → SidecarUnavailable + ZERO provider
    hits when ``fail_open=False`` (the secure default)."""
    provider = AsyncMock(return_value=SimpleNamespace(
        id="should-not-happen",
        usage=SimpleNamespace(prompt_tokens=10, completion_tokens=42),
    ))
    client = _fake_client(decision="DEGRADE")
    monkeypatch.setattr(litellm, "acompletion", provider)
    install_shim(_make_options(client, fail_open=False))
    with pytest.raises(SidecarUnavailable):
        await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "degrade"}],
        )
    assert provider.call_count == 0


# ---------------------------------------------------------------------------
# §2.3 (U20) — fail_open=True allows DEGRADE through with WARN
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_u20_fail_open_allows_degrade(monkeypatch, shim_clean, caplog):
    """U20: ``fail_open=True`` degrades DEGRADE to a WARN+allow path.
    Provider IS hit; sidecar commit MUST NOT fire (no reservation to
    commit against)."""
    provider = AsyncMock(return_value=SimpleNamespace(
        id="chatcmpl-failopen",
        usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1),
    ))
    client = _fake_client(decision="DEGRADE")
    monkeypatch.setattr(litellm, "acompletion", provider)
    install_shim(_make_options(client, fail_open=True))
    with caplog.at_level("WARNING", logger="spendguard.integrations.litellm_sdk_shim"):
        resp = await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "failopen"}],
        )
    assert resp.id == "chatcmpl-failopen"
    assert provider.call_count == 1
    assert client.emit_llm_call_post.call_count == 0
    assert any("fail_open" in r.message.lower() or "fail_open" in r.message
               for r in caplog.records), \
        "fail_open path MUST log a WARN so dev-mode allowance is observable"


# ---------------------------------------------------------------------------
# §2.5 (U23) — Success commits with real provider usage
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_u23_success_commits_with_real_usage(monkeypatch, shim_clean):
    """U23: ``response.usage.completion_tokens`` is the commit amount —
    NOT the estimator-snapshot. Audit row carries SUCCESS +
    ``provider_event_id`` from ``response.id``."""
    provider = AsyncMock(return_value=SimpleNamespace(
        id="chatcmpl-real-usage-1234",
        usage=SimpleNamespace(prompt_tokens=15, completion_tokens=137),
    ))
    client = _fake_client()
    monkeypatch.setattr(litellm, "acompletion", provider)
    install_shim(_make_options(client))
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "u23"}],
    )
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "SUCCESS"
    assert kw["estimated_amount_atomic"] == "137"
    assert kw["actual_output_tokens"] == 137
    assert kw["actual_input_tokens"] == 15
    assert kw["provider_event_id"] == "chatcmpl-real-usage-1234"


# ---------------------------------------------------------------------------
# §2.5 (U24) — Provider raises HTTP error → release + re-raise
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_u24_provider_http_error_emits_failure_and_reraises(
    monkeypatch, shim_clean,
):
    """U24: provider raises an arbitrary error → shim emits
    ``outcome=FAILURE`` + re-raises the ORIGINAL exception untouched
    (operator-visible exception model)."""

    class _UpstreamHTTP500(Exception):
        pass

    provider = AsyncMock(side_effect=_UpstreamHTTP500("provider 500"))
    client = _fake_client()
    monkeypatch.setattr(litellm, "acompletion", provider)
    install_shim(_make_options(client))
    with pytest.raises(_UpstreamHTTP500, match="provider 500"):
        await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "u24"}],
        )
    # Release commit emitted; SUCCESS commit not emitted.
    assert client.emit_llm_call_post.call_count == 1
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "FAILURE"


# ---------------------------------------------------------------------------
# §2.5 (U25) — asyncio.CancelledError → CANCELLED outcome + re-raise
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_u25_cancellation_emits_cancelled(monkeypatch, shim_clean):
    """U25: ``asyncio.CancelledError`` mid-call routes through
    ``release_failure`` with ``outcome=CANCELLED`` and re-raises."""
    provider = AsyncMock(side_effect=asyncio.CancelledError())
    client = _fake_client()
    monkeypatch.setattr(litellm, "acompletion", provider)
    install_shim(_make_options(client))
    with pytest.raises(asyncio.CancelledError):
        await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "u25"}],
        )
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["outcome"] == "CANCELLED"


# ---------------------------------------------------------------------------
# §6 — decision_context carries mode='sdk' to distinguish from proxy / direct
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_decision_context_has_mode_sdk(monkeypatch, shim_clean):
    """``decision_context.mode='sdk'`` differentiates shim audits from
    proxy / direct / egress (verify SQL gate D12_SDK_GATE depends on
    this)."""
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(return_value=SimpleNamespace(
        id="x", usage=SimpleNamespace(prompt_tokens=1, completion_tokens=2),
    )))
    client = _fake_client()
    install_shim(_make_options(client))
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "ctx"}],
    )
    kw = client.request_decision.call_args.kwargs
    ctx = kw["decision_context_json"]
    assert ctx["integration"] == "litellm"
    assert ctx["mode"] == "sdk"
    assert ctx["model"] == "gpt-4o-mini"
    assert "prompt_hash" in ctx
    assert ctx["stream"] is False


# ---------------------------------------------------------------------------
# Provider routing: model strings for each LiteLLM-normalised provider
# all reach the same shim core (no provider-specific bypass).
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
@pytest.mark.parametrize("model", [
    "gpt-4o-mini",                       # OpenAI
    "claude-3-5-sonnet-20240620",        # Anthropic
    "bedrock/anthropic.claude-3-sonnet", # Bedrock route
    "gemini/gemini-1.5-flash",           # Gemini
    "cohere/command-r-plus",             # Cohere
])
async def test_multi_provider_routing_reserves_for_all(
    model, monkeypatch, shim_clean,
):
    """Every LiteLLM-supported provider model string reserves once.
    LiteLLM normalises ``response.usage.completion_tokens`` across
    providers — the shim's commit path doesn't need provider-specific
    branches."""
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(return_value=SimpleNamespace(
        id=f"resp-{model}",
        usage=SimpleNamespace(prompt_tokens=3, completion_tokens=11),
    )))
    client = _fake_client()
    install_shim(_make_options(client))
    await litellm.acompletion(
        model=model,
        messages=[{"role": "user", "content": "multi"}],
    )
    assert client.request_decision.call_count == 1
    assert client.emit_llm_call_post.call_count == 1
    ctx = client.request_decision.call_args.kwargs["decision_context_json"]
    assert ctx["model"] == model


# ---------------------------------------------------------------------------
# Idempotency key derivation — same call signature, deterministic key
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_idempotency_key_is_deterministic_per_call(monkeypatch, shim_clean):
    """The shim derives a deterministic ``idempotency_key`` from
    (tenant, session, run, step, llm_call_id, trigger). Two calls with
    DIFFERENT litellm_call_id produce DIFFERENT keys; a third call with
    the SAME litellm_call_id reproduces the first key (sidecar dedupe
    contract)."""
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(return_value=SimpleNamespace(
        id="r", usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1),
    )))
    client = _fake_client()
    install_shim(_make_options(client))
    await litellm.acompletion(
        model="gpt-4o-mini", messages=[],
        litellm_call_id="11111111-1111-4111-8111-111111111111",
    )
    await litellm.acompletion(
        model="gpt-4o-mini", messages=[],
        litellm_call_id="22222222-2222-4222-8222-222222222222",
    )
    await litellm.acompletion(
        model="gpt-4o-mini", messages=[],
        litellm_call_id="11111111-1111-4111-8111-111111111111",
    )
    ids = [c.kwargs["idempotency_key"]
           for c in client.request_decision.call_args_list]
    assert ids[0] != ids[1], "different litellm_call_id MUST produce different idempotency keys"
    assert ids[0] == ids[2], "same litellm_call_id MUST reproduce the idempotency key"


# ---------------------------------------------------------------------------
# Sync completion from inside a running loop raises (already in
# test_install.py; here we cover ``text_completion`` for parity)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_text_completion_inside_loop_raises(monkeypatch, shim_clean):
    """Sync ``text_completion`` from inside ``pytest.mark.asyncio``
    raises ``SpendGuardShimSyncInAsyncContext`` with a hint pointing at
    ``acompletion``."""
    monkeypatch.setattr(litellm, "text_completion", MagicMock())
    client = _fake_client()
    install_shim(_make_options(client))
    with pytest.raises(SpendGuardShimSyncInAsyncContext, match="acompletion"):
        litellm.text_completion(
            model="gpt-3.5-turbo-instruct", prompt="sync",
        )


def test_text_completion_outside_loop_bridges(monkeypatch, shim_clean):
    """Sync ``text_completion`` from a non-async context bridges via
    ``asyncio.run`` and drives reserve+commit on the bridged loop."""
    original = MagicMock(return_value=SimpleNamespace(
        id="t1", usage=SimpleNamespace(prompt_tokens=2, completion_tokens=4),
    ))
    monkeypatch.setattr(litellm, "text_completion", original)
    client = _fake_client()
    install_shim(_make_options(client))
    resp = litellm.text_completion(
        model="gpt-3.5-turbo-instruct", prompt="hi",
    )
    assert resp.id == "t1"
    assert client.request_decision.call_count == 1
    assert client.emit_llm_call_post.call_count == 1


# ---------------------------------------------------------------------------
# Commit path — sidecar commit RPC failure is swallowed (TTL backstop)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_commit_rpc_failure_does_not_propagate(monkeypatch, shim_clean):
    """If the commit RPC fails the shim logs a WARN and returns the
    provider response anyway — the TTL sweeper is the durable backstop.
    Operator-visible behaviour: the caller does NOT see a sidecar error
    after a successful provider response."""
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(return_value=SimpleNamespace(
        id="r-commit-fail",
        usage=SimpleNamespace(prompt_tokens=3, completion_tokens=5),
    )))

    async def _commit_fails(**_kw):
        from spendguard.errors import SpendGuardError
        raise SpendGuardError("sidecar commit RPC blew up")

    client = _fake_client(commit_side_effect=_commit_fails)
    install_shim(_make_options(client))
    resp = await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "commit-fail"}],
    )
    assert resp.id == "r-commit-fail", \
        "operator MUST see the provider response even if commit RPC failed"


# ---------------------------------------------------------------------------
# Default estimator falls back when response.usage is missing
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_commit_falls_back_to_estimator_when_usage_missing(
    monkeypatch, shim_clean,
):
    """LiteLLM-normalised responses can omit ``usage`` for compatibility
    layers. Shim falls back to the estimator-snapshot so the audit row
    always carries a non-empty amount."""
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(return_value=SimpleNamespace(
        id="no-usage", usage=None,
    )))
    client = _fake_client()
    install_shim(_make_options(client))
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "no usage"}],
    )
    kw = client.emit_llm_call_post.call_args.kwargs
    assert kw["estimated_amount_atomic"] != ""
    assert int(kw["estimated_amount_atomic"]) >= 1
    assert kw["actual_output_tokens"] is None
    assert kw["actual_input_tokens"] is None


# ---------------------------------------------------------------------------
# Reserve / commit / release lifecycle: counts under concurrent gather
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_concurrent_calls_each_get_own_reservation(monkeypatch, shim_clean):
    """N=4 concurrent ``asyncio.gather`` calls → N reservations + N
    commits (sibling tasks do NOT share the ``_IN_FLIGHT`` token).

    Tightens the install-test ``test_in_flight_contextvar_isolated_per_task``
    by also checking the commit count and that idempotency keys differ
    across tasks.
    """
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(return_value=SimpleNamespace(
        id="x", usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1),
    )))
    client = _fake_client()
    install_shim(_make_options(client))

    async def _one():
        await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "gather"}],
        )

    await asyncio.gather(_one(), _one(), _one(), _one())
    assert client.request_decision.call_count == 4
    assert client.emit_llm_call_post.call_count == 4
    keys = {c.kwargs["idempotency_key"]
            for c in client.request_decision.call_args_list}
    assert len(keys) == 4, (
        f"idempotency keys MUST differ across gather siblings; got {keys!r}"
    )


# ---------------------------------------------------------------------------
# Options validation — empty tenant_id rejected (already in install
# tests); confirm None client rejected too.
# ---------------------------------------------------------------------------


def test_options_rejects_none_client():
    """SpendGuardShimOptions refuses ``client=None`` — surfaces a
    misconfiguration loudly at construction, not at first request."""
    with pytest.raises(ValueError, match="client"):
        SpendGuardShimOptions(client=None, tenant_id="tenant-1")


# ---------------------------------------------------------------------------
# Atext_completion routes through the shim core (atext-specific arg shape)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_atext_completion_routes_through_core_with_prompt_arg(
    monkeypatch, shim_clean,
):
    """``atext_completion`` takes ``prompt=`` not ``messages=``; shim
    estimator falls back to a chars/4 path so the absence of
    ``messages`` doesn't crash."""
    original = AsyncMock(return_value=SimpleNamespace(
        id="atext-1",
        usage=SimpleNamespace(prompt_tokens=4, completion_tokens=8),
    ))
    monkeypatch.setattr(litellm, "atext_completion", original)
    client = _fake_client()
    install_shim(_make_options(client))
    resp = await litellm.atext_completion(
        model="gpt-3.5-turbo-instruct",
        prompt="hi there",
    )
    assert resp.id == "atext-1"
    assert client.request_decision.call_count == 1
    assert client.emit_llm_call_post.call_count == 1


# ---------------------------------------------------------------------------
# litellm_call_id reuse on reserve injection
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_injected_litellm_call_id_visible_to_provider(monkeypatch, shim_clean):
    """The shim injects ``litellm_call_id`` into kwargs BEFORE calling
    the provider (so LiteLLM's call-id chain remains stable). When the
    caller pre-supplies the id, the shim MUST NOT overwrite it."""
    captured: dict = {}

    async def _capturing_provider(**kw):
        captured.update(kw)
        return SimpleNamespace(
            id="captured", usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1),
        )

    monkeypatch.setattr(litellm, "acompletion", AsyncMock(side_effect=_capturing_provider))
    client = _fake_client()
    install_shim(_make_options(client))

    # Caller pre-supplies — shim must preserve.
    await litellm.acompletion(
        model="gpt-4o-mini", messages=[],
        litellm_call_id="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    )
    assert captured["litellm_call_id"] == "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"

    # Caller does NOT supply — shim must inject something stable.
    captured.clear()
    await litellm.acompletion(
        model="gpt-4o-mini", messages=[],
    )
    assert "litellm_call_id" in captured
    assert captured["litellm_call_id"]  # non-empty


# ---------------------------------------------------------------------------
# Subclass Router added AFTER install picks up the patched parent via MRO
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_post_install_router_subclass_inherits_patched_parent(
    monkeypatch, shim_clean,
):
    """A Router subclass created AFTER ``install_shim()`` inherits the
    patched parent ``Router.acompletion`` via MRO — no extra subclass
    walk needed."""
    router_mock = AsyncMock(return_value=SimpleNamespace(
        id="r-post", usage=SimpleNamespace(prompt_tokens=2, completion_tokens=3),
    ))
    monkeypatch.setattr(litellm.Router, "acompletion", router_mock)
    client = _fake_client()
    install_shim(_make_options(client))

    class PostInstallRouter(litellm.Router):
        pass

    router = PostInstallRouter(model_list=[
        {"model_name": "gpt-4o-mini",
         "litellm_params": {"model": "gpt-4o-mini", "api_key": "sk-test"}},
    ])
    resp = await router.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "post-install subclass"}],
    )
    assert resp.id == "r-post"
    assert client.request_decision.call_count == 1


# ---------------------------------------------------------------------------
# config_signature: same scalar fields but different client identity → raises
# ---------------------------------------------------------------------------


def test_same_scalar_fields_but_different_client_is_blocked(
    monkeypatch, shim_clean,
):
    """Two ``SpendGuardClient`` instances with bit-identical scalar
    config are STILL treated as a config-signature change because their
    channel state is independent.

    Locks the design decision in ``_state._compute_config_signature``
    (DESIGN §5): client identity is part of the signature.
    """
    monkeypatch.setattr(litellm, "acompletion", AsyncMock())
    cli_a = _fake_client()
    install_shim(SpendGuardShimOptions(
        client=cli_a, tenant_id="tenant-1", budget_id="b1", fail_open=False,
    ))
    cli_b = _fake_client()  # different instance, same scalar fields
    from spendguard.integrations.litellm_sdk_shim import (
        SpendGuardShimAlreadyInstalled,
    )
    with pytest.raises(SpendGuardShimAlreadyInstalled):
        install_shim(SpendGuardShimOptions(
            client=cli_b, tenant_id="tenant-1", budget_id="b1",
            fail_open=False,
        ))


# ---------------------------------------------------------------------------
# Partial-install rollback: a patch failure leaves litellm unmodified
# ---------------------------------------------------------------------------


def test_install_failure_rolls_back_partial_patches(monkeypatch, shim_clean):
    """If one of the patch helpers raises mid-install, the rollback
    walk MUST restore every already-patched attribute (so litellm
    returns to its pre-install state and the operator can fix + retry)."""
    monkeypatch.setattr(litellm, "acompletion", AsyncMock())
    pre_acompletion = litellm.acompletion
    pre_router_acompletion = litellm.Router.acompletion

    # Inject a failure into the _patch_router helper so the install
    # walk fails AFTER acompletion + completion patches have landed.
    from spendguard.integrations.litellm_sdk_shim import _patches

    def _broken_router(_state):
        raise RuntimeError("synthetic router-patch failure")

    monkeypatch.setattr(_patches._router, "_patch_router", _broken_router)
    client = _fake_client()
    with pytest.raises(RuntimeError, match="synthetic router-patch"):
        install_shim(_make_options(client))

    # acompletion + completion + Router.acompletion MUST be restored.
    assert litellm.acompletion is pre_acompletion
    assert litellm.Router.acompletion is pre_router_acompletion
    assert is_installed() is False


# ---------------------------------------------------------------------------
# Re-entry inside _DirectCore via inner litellm.acompletion call still
# avoids double-reserve via contextvar guard
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_inner_litellm_call_inside_provider_skips_reserve(
    monkeypatch, shim_clean,
):
    """Models a LiteLLM-internal fallback chain that calls back into
    ``litellm.acompletion``. The recursion guard ContextVar short-
    circuits the inner call straight to the original (no double-reserve).
    """
    call_count = {"original": 0}

    async def _original_with_fallback(**kwargs):
        call_count["original"] += 1
        if call_count["original"] == 1:
            # Synthetic fallback: re-enter patched entry point.
            await litellm.acompletion(model="gpt-4o-mini", messages=[])
        return SimpleNamespace(
            id="x", usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1),
        )

    monkeypatch.setattr(litellm, "acompletion", AsyncMock(side_effect=_original_with_fallback))
    client = _fake_client()
    install_shim(_make_options(client))
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "outer"}],
    )
    assert call_count["original"] == 2
    assert client.request_decision.call_count == 1
    assert client.emit_llm_call_post.call_count == 1


# ---------------------------------------------------------------------------
# Counts: reserve + commit fire EXACTLY once on a success path
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_reserve_and_commit_fire_exactly_once_on_success(
    monkeypatch, shim_clean,
):
    """One litellm.acompletion call → exactly one reserve + exactly one
    commit. Catches a future regression where a retry / fallback chain
    accidentally double-fires the sidecar RPCs."""
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(return_value=SimpleNamespace(
        id="once", usage=SimpleNamespace(prompt_tokens=1, completion_tokens=1),
    )))
    client = _fake_client()
    install_shim(_make_options(client))
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "once"}],
    )
    assert client.request_decision.call_count == 1
    assert client.emit_llm_call_post.call_count == 1


# ---------------------------------------------------------------------------
# Empty messages list: estimator handles gracefully (lower bound = 1)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_empty_messages_does_not_crash_estimator(monkeypatch, shim_clean):
    """``messages=[]`` is uncommon but legal in LiteLLM. The default
    estimator must produce a non-zero floor so the sidecar reserve has
    a positive amount to evaluate."""
    monkeypatch.setattr(litellm, "acompletion", AsyncMock(return_value=SimpleNamespace(
        id="empty", usage=SimpleNamespace(prompt_tokens=0, completion_tokens=0),
    )))
    client = _fake_client()
    install_shim(_make_options(client))
    await litellm.acompletion(model="gpt-4o-mini", messages=[])
    assert client.request_decision.call_count == 1
    kw = client.request_decision.call_args.kwargs
    assert kw["projected_claims"]
    assert int(kw["projected_claims"][0].amount_atomic) >= 1
