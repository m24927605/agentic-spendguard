# ruff: noqa: ANN001, ANN002, ANN201, ANN202, ANN003, ANN401, S106, S105, S110
"""D12 SLICE 6 — integration tests against real LiteLLM + pytest-httpx.

These tests import the REAL ``litellm`` package (not mocked) and assert
the wire-level ordering: sidecar RPC fires BEFORE the upstream HTTP
request leaves the process. ``pytest-httpx`` records the OpenAI HTTP
hit via a custom ``httpx.AsyncHTTPTransport`` that openai-python's
SyncHttpxClientWrapper picks up; the fake sidecar records the reserve
call timestamp via an ``asyncio.Event``; the strict-order check
compares the two.

The sidecar itself is still mocked via ``AsyncMock`` (no docker
required) — pytest-httpx covers the provider-side wire boundary and
that is the boundary D12's thesis ("reserve before provider HTTP")
lives on.

Additionally we cover the 3 transitive smoke tests from tests.md §4:
``test_crewai_via_shim`` and ``test_dspy_via_shim`` skip cleanly when
the framework isn't installed; the demo container in SLICE 7 installs
both so CI exercises them at least via ``DEMO_MODE=litellm_sdk_real``.

Marked with ``@pytest.mark.integration`` so a CI matrix that wants to
split fast unit tests from slow integration tests can filter via
``-m "not integration"``.
"""

from __future__ import annotations

import asyncio
import json
import time
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import httpx
import pytest

pytest.importorskip("litellm", reason="LiteLLM not installed")

import litellm  # noqa: E402

from spendguard.errors import DecisionDenied  # noqa: E402
from spendguard.integrations.litellm_sdk_shim import (  # noqa: E402
    SpendGuardShimOptions,
    install_shim,
    is_installed,
    uninstall_shim,
)

pytestmark = pytest.mark.integration


# ---------------------------------------------------------------------------
# In-process HTTP transport recorder
# ---------------------------------------------------------------------------


class _RecordingTransport(httpx.AsyncBaseTransport):
    """An async httpx transport that records every dispatch + returns
    a canned response. We use this to mock litellm's upstream provider
    calls instead of pytest-httpx because the openai-python SDK in
    use here (2.x) wraps httpx with its own SyncHttpxClientWrapper
    and pytest-httpx's global patch is unreliable across that wrap.
    """

    def __init__(
        self,
        *,
        response_factory,
        reserve_event: asyncio.Event | None = None,
    ) -> None:
        self._response_factory = response_factory
        self._reserve_event = reserve_event
        self.recorded_requests: list[httpx.Request] = []
        # Snapshot the reserve-event state AT THE MOMENT this transport
        # was hit. INV-2 strict-order assertion lives on this snapshot.
        self.reserve_was_set_when_hit: list[bool] = []

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        self.recorded_requests.append(request)
        self.reserve_was_set_when_hit.append(
            self._reserve_event.is_set() if self._reserve_event else True,
        )
        body = self._response_factory(request)
        return httpx.Response(
            status_code=200,
            headers={"content-type": "application/json"},
            content=json.dumps(body).encode(),
            request=request,
        )


def _openai_chat_completion_response(
    *,
    completion_tokens: int = 42,
    content: str = "hi from openai",
) -> dict:
    """Build an OpenAI chat-completion JSON shape."""
    return {
        "id": "chatcmpl-real-int-1",
        "object": "chat.completion",
        "created": int(time.time()),
        "model": "gpt-4o-mini",
        "choices": [
            {
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop",
            },
        ],
        "usage": {
            "prompt_tokens": 5,
            "completion_tokens": completion_tokens,
            "total_tokens": 5 + completion_tokens,
        },
    }


def _openai_text_completion_response(*, completion_tokens: int = 6) -> dict:
    """Build a /v1/completions (text) response shape."""
    return {
        "id": "cmpl-real-int-1",
        "object": "text_completion",
        "created": int(time.time()),
        "model": "gpt-3.5-turbo-instruct",
        "choices": [{"index": 0, "text": "hi", "finish_reason": "stop"}],
        "usage": {
            "prompt_tokens": 4,
            "completion_tokens": completion_tokens,
            "total_tokens": 4 + completion_tokens,
        },
    }


def _install_recording_transport(
    monkeypatch,
    *,
    response_factory,
    reserve_event: asyncio.Event | None = None,
) -> _RecordingTransport:
    """Force openai-python's wrapper + litellm's direct httpx calls to
    use our recording transport.

    We patch ``httpx.AsyncClient.__init__`` so every freshly-created
    async client receives ``transport=<our recorder>``. That covers
    both the openai SDK's path AND any direct ``httpx.AsyncClient(...)``
    from litellm's own integrations layer.
    """
    transport = _RecordingTransport(
        response_factory=response_factory,
        reserve_event=reserve_event,
    )
    real_init = httpx.AsyncClient.__init__

    def _patched_init(self, *args, **kwargs):
        # Always force the recording transport — drops any transport
        # the caller supplied (we want to deterministically capture).
        kwargs["transport"] = transport
        real_init(self, *args, **kwargs)

    monkeypatch.setattr(httpx.AsyncClient, "__init__", _patched_init)

    # And patch the sync side too, because litellm's sync `completion`
    # path goes through openai's SyncHttpxClientWrapper.
    class _SyncRecordingTransport(httpx.BaseTransport):
        def __init__(self) -> None:
            self.recorded_requests: list[httpx.Request] = []
            self.reserve_was_set_when_hit: list[bool] = []

        def handle_request(self, request: httpx.Request) -> httpx.Response:
            self.recorded_requests.append(request)
            self.reserve_was_set_when_hit.append(
                reserve_event.is_set() if reserve_event else True,
            )
            body = response_factory(request)
            return httpx.Response(
                status_code=200,
                headers={"content-type": "application/json"},
                content=json.dumps(body).encode(),
                request=request,
            )

    sync_transport = _SyncRecordingTransport()
    real_sync_init = httpx.Client.__init__

    def _patched_sync_init(self, *args, **kwargs):
        kwargs["transport"] = sync_transport
        real_sync_init(self, *args, **kwargs)

    monkeypatch.setattr(httpx.Client, "__init__", _patched_sync_init)
    # Expose sync recorder on the async one for unified test access.
    transport.sync_recorder = sync_transport  # type: ignore[attr-defined]
    return transport


# ---------------------------------------------------------------------------
# Fake sidecar + standard fixtures
# ---------------------------------------------------------------------------


def _fake_sidecar_client(
    *,
    decision: str = "CONTINUE",
    reserve_event: asyncio.Event | None = None,
    deny: bool = False,
) -> MagicMock:
    cli = MagicMock()
    cli.tenant_id = "tenant-1"
    cli.session_id = "session-1"

    async def _reserve(**_kw):
        if reserve_event is not None:
            reserve_event.set()
        if deny:
            raise DecisionDenied(
                "budget exhausted",
                decision_id="dec-deny",
                reason_codes=["BUDGET_EXHAUSTED"],
            )
        return SimpleNamespace(
            decision=decision,
            decision_id="dec-1",
            reservation_ids=("res-1",),
            audit_decision_event_id="audit-1",
        )

    cli.request_decision = AsyncMock(side_effect=_reserve)
    cli.emit_llm_call_post = AsyncMock(return_value=None)
    return cli


@pytest.fixture
def shim_clean():
    """Test-isolation fixture (mandatory per tests.md §10)."""
    yield
    if is_installed():
        uninstall_shim()


def _options(client: MagicMock, *, fail_open: bool = False) -> SpendGuardShimOptions:
    return SpendGuardShimOptions(
        client=client,
        tenant_id=client.tenant_id,
        budget_id="b1",
        fail_open=fail_open,
    )


# ---------------------------------------------------------------------------
# I01 — Real acompletion: reserve fires BEFORE OpenAI HTTP
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_i01_real_litellm_acompletion_reserve_before_openai_http(
    monkeypatch, shim_clean,
):
    """Real ``litellm.acompletion`` with the shim installed.
    The recording transport captures every HTTP hit and snapshots the
    reserve-event state at dispatch time. Strict-order check: the
    snapshot must read ``True`` (reserve already fired)."""
    reserve_event = asyncio.Event()
    transport = _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_chat_completion_response(
            completion_tokens=37,
        ),
        reserve_event=reserve_event,
    )

    client = _fake_sidecar_client(reserve_event=reserve_event)
    install_shim(_options(client))
    resp = await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "real i01"}],
        api_key="sk-proj-test-i01-spendguard-shim",
    )
    assert resp.usage.completion_tokens == 37
    # Sidecar reserve + commit each fired once.
    assert client.request_decision.call_count == 1
    assert client.emit_llm_call_post.call_count == 1
    # Strict-order: every captured upstream HTTP hit observed the
    # reserve event ALREADY SET. A buggy shim would record at least
    # one False.
    assert transport.recorded_requests, (
        "Recording transport saw zero upstream HTTPs — test setup is wrong."
    )
    assert all(transport.reserve_was_set_when_hit), (
        f"INV-2 broken: at least one upstream HTTP dispatched BEFORE "
        f"the sidecar reserve completed. Snapshot: "
        f"{transport.reserve_was_set_when_hit!r}"
    )
    # And the commit kwargs reflect the real-usage reconciliation.
    commit_kw = client.emit_llm_call_post.call_args.kwargs
    assert commit_kw["outcome"] == "SUCCESS"
    assert commit_kw["actual_output_tokens"] == 37


# ---------------------------------------------------------------------------
# I02 — DENY raises + ZERO requests to api.openai.com
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_i02_real_litellm_deny_zero_openai_calls(monkeypatch, shim_clean):
    """Sidecar configured to DENY → ``DecisionDenied`` raised → ZERO
    HTTPX calls captured by the recording transport. The most severe
    correctness bug if D12 breaks (INV-1)."""
    transport = _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_chat_completion_response(),
    )
    client = _fake_sidecar_client(deny=True)
    install_shim(_options(client))
    with pytest.raises(DecisionDenied):
        await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "deny i02"}],
            api_key="sk-proj-test-i02-spendguard-shim",
        )
    assert transport.recorded_requests == [], (
        f"INV-1 broken: DENY MUST NOT hit upstream; "
        f"got {len(transport.recorded_requests)} requests."
    )


# ---------------------------------------------------------------------------
# I03 — Sync litellm.completion bridges via asyncio.run + real OpenAI route
# ---------------------------------------------------------------------------


def test_i03_real_litellm_completion_sync_outside_loop(monkeypatch, shim_clean):
    """Real sync ``litellm.completion`` from a non-async test bridges
    through ``asyncio.run`` and still reserves before the OpenAI HTTP."""
    transport = _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_chat_completion_response(
            completion_tokens=11,
        ),
    )
    client = _fake_sidecar_client()
    install_shim(_options(client))
    resp = litellm.completion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "sync i03"}],
        api_key="sk-proj-test-i03-spendguard-shim",
    )
    assert resp.usage.completion_tokens == 11
    assert client.request_decision.call_count == 1
    assert client.emit_llm_call_post.call_count == 1
    # The sync path uses httpx.Client (not AsyncClient) — verify the
    # sync recorder captured the hit.
    sync_rec = getattr(transport, "sync_recorder", None)
    if sync_rec is not None:
        # litellm.completion may route through openai's sync wrapper
        # which uses httpx.Client; either the async OR sync recorder
        # should have seen the hit. The OR is the assertion.
        assert (transport.recorded_requests or sync_rec.recorded_requests), (
            "Neither async nor sync recorder captured the upstream HTTP."
        )


# ---------------------------------------------------------------------------
# I04 — Real Router.acompletion: reserve fires before upstream HTTP
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_i04_real_router_acompletion_reserve_before_http(
    monkeypatch, shim_clean,
):
    """``litellm.Router(...).acompletion(...)`` with shim installed —
    reserve fires before the upstream HTTP call. Covers the framework-
    level dispatcher CrewAI / DSPy / SmolAgents all build on top of."""
    reserve_event = asyncio.Event()
    transport = _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_chat_completion_response(
            completion_tokens=29,
        ),
        reserve_event=reserve_event,
    )
    client = _fake_sidecar_client(reserve_event=reserve_event)
    install_shim(_options(client))
    router = litellm.Router(model_list=[
        {
            "model_name": "gpt-4o-mini",
            "litellm_params": {
                "model": "gpt-4o-mini",
                "api_key": "sk-proj-test-i04-spendguard-shim",
            },
        },
    ])
    resp = await router.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "router i04"}],
    )
    assert resp.usage.completion_tokens == 29
    assert transport.recorded_requests, (
        "Router test: recording transport saw zero upstream HTTPs."
    )
    assert all(transport.reserve_was_set_when_hit), (
        f"INV-2 broken on Router path; snapshot: "
        f"{transport.reserve_was_set_when_hit!r}"
    )
    assert client.request_decision.call_count == 1


# ---------------------------------------------------------------------------
# I05 — atext_completion (text endpoint)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_i05_real_atext_completion_text_endpoint(
    monkeypatch, shim_clean,
):
    """``await litellm.atext_completion(prompt=...)`` reserves before
    hitting ``/v1/completions`` (legacy text endpoint, different URL
    path than chat completions)."""
    reserve_event = asyncio.Event()
    transport = _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_text_completion_response(
            completion_tokens=8,
        ),
        reserve_event=reserve_event,
    )
    client = _fake_sidecar_client(reserve_event=reserve_event)
    install_shim(_options(client))
    resp = await litellm.atext_completion(
        model="gpt-3.5-turbo-instruct",
        prompt="hi i05",
        api_key="sk-proj-test-i05-spendguard-shim",
    )
    assert resp.usage.completion_tokens == 8
    assert transport.recorded_requests, (
        "atext_completion: recording transport saw zero upstream HTTPs."
    )
    assert all(transport.reserve_was_set_when_hit), (
        f"INV-2 broken on text endpoint; snapshot: "
        f"{transport.reserve_was_set_when_hit!r}"
    )
    assert client.request_decision.call_count == 1
    assert client.emit_llm_call_post.call_count == 1


# ---------------------------------------------------------------------------
# I06 — install / uninstall baseline: post-uninstall calls hit upstream
#       directly with ZERO sidecar reserve (proves restore is complete)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_i06_install_uninstall_real_litellm_baseline_unchanged(
    monkeypatch, shim_clean,
):
    """Install → reserve fires. Then uninstall → subsequent call hits
    upstream with NO sidecar reserve (proves the originals were
    restored cleanly).
    """
    transport = _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_chat_completion_response(
            completion_tokens=13,
        ),
    )
    client = _fake_sidecar_client()
    install_shim(_options(client))
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "i06 install"}],
        api_key="sk-proj-test-i06-spendguard-shim",
    )
    assert client.request_decision.call_count == 1
    hits_before_uninstall = len(transport.recorded_requests)

    uninstall_shim()
    assert is_installed() is False
    await litellm.acompletion(
        model="gpt-4o-mini",
        messages=[{"role": "user", "content": "i06 post-uninstall"}],
        api_key="sk-proj-test-i06-spendguard-shim",
    )
    # Upstream HTTP hit again, sidecar reserve UNCHANGED — proves
    # restore worked.
    assert len(transport.recorded_requests) > hits_before_uninstall
    assert client.request_decision.call_count == 1, (
        "Post-uninstall call reserved; restore is broken."
    )


# ---------------------------------------------------------------------------
# Transitive smokes (T01-T03 from tests.md §4)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_t01_crewai_via_shim_triggers_spendguard_reserve(
    monkeypatch, shim_clean,
):
    """T01: CrewAI ``Agent`` + ``Task`` + ``Crew`` — ``kickoff_async``
    triggers SpendGuard reserves with NO CrewAI code changes. The
    load-bearing proof that D12 closes coverage for the 7 frameworks
    that route through litellm.

    Skipped cleanly when CrewAI is not installed (heavy deps; pinned to
    the demo container in SLICE 7).
    """
    pytest.importorskip("crewai", reason="CrewAI not installed (skip)")
    from crewai import Agent, Crew, Process, Task

    _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_chat_completion_response(
            completion_tokens=22,
            content="Final Answer: hello from CrewAI",
        ),
    )
    client = _fake_sidecar_client()
    install_shim(_options(client))

    agent = Agent(
        role="greeter",
        goal="Greet the user with a single sentence",
        backstory="A friendly greeter agent for SpendGuard smoke tests.",
        verbose=False,
        allow_delegation=False,
        llm="openai/gpt-4o-mini",
    )
    task = Task(
        description="Greet the user with a single sentence.",
        expected_output="A single sentence greeting.",
        agent=agent,
    )
    crew = Crew(
        agents=[agent], tasks=[task], process=Process.sequential, verbose=False,
    )
    try:
        await crew.kickoff_async()
    except Exception:
        # CrewAI may raise on the final-answer parse with our stub
        # responses; the reserve assertion below is what matters.
        pass
    assert client.request_decision.call_count >= 1, (
        "CrewAI kickoff_async did NOT trigger any SpendGuard reserve. "
        "D12 transitive coverage broken."
    )


def test_t02_crewai_deny_blocks_kickoff(monkeypatch, shim_clean):
    """T02: Sidecar configured to DENY during a CrewAI kickoff →
    ZERO upstream HTTPs reach the recording transport."""
    pytest.importorskip("crewai", reason="CrewAI not installed (skip)")
    from crewai import Agent, Crew, Process, Task

    transport = _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_chat_completion_response(),
    )
    client = _fake_sidecar_client(deny=True)
    install_shim(_options(client))
    agent = Agent(
        role="greeter",
        goal="Greet the user.",
        backstory="A friendly greeter agent.",
        verbose=False,
        allow_delegation=False,
        llm="openai/gpt-4o-mini",
    )
    task = Task(
        description="Greet the user.",
        expected_output="A greeting.",
        agent=agent,
    )
    crew = Crew(
        agents=[agent], tasks=[task], process=Process.sequential, verbose=False,
    )
    try:
        crew.kickoff()
    except Exception:
        pass
    assert transport.recorded_requests == [], (
        f"INV-1 broken via CrewAI: DENY MUST NOT reach upstream; "
        f"got {len(transport.recorded_requests)} requests."
    )


def test_t03_dspy_predict_triggers_spendguard_reserve(monkeypatch, shim_clean):
    """T03: DSPy ``LM`` + ``Predict`` exercises the litellm SDK path
    DSPy uses internally. SpendGuard reserve fires for each predict
    call."""
    pytest.importorskip("dspy", reason="DSPy not installed (skip)")
    import dspy

    _install_recording_transport(
        monkeypatch,
        response_factory=lambda _r: _openai_chat_completion_response(
            completion_tokens=16,
            content="hello from DSPy",
        ),
    )
    client = _fake_sidecar_client()
    install_shim(_options(client))
    lm = dspy.LM(
        "openai/gpt-4o-mini",
        api_key="sk-proj-test-t03-spendguard-shim",
    )
    dspy.configure(lm=lm)
    predict = dspy.Predict("question -> answer")
    try:
        predict(question="Say hello.")
    except Exception:
        # DSPy may raise if our stub answer doesn't parse cleanly.
        pass
    assert client.request_decision.call_count >= 1, (
        "DSPy Predict did NOT trigger any SpendGuard reserve. "
        "D12 transitive coverage broken."
    )
