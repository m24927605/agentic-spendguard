"""Unit tests for the OpenAI upstream + SpendGuardLLM._invoke path.

review-standards.md slice 4 checklist coverage:
- 4.1 OpenAI client constructed per-call from credentials (not cached)
- 4.2 spendguard/ prefix stripped before upstream wire
- 4.3 real usage from response.usage feeds commit (INV-5)
- 4.4 DENY -> ZERO upstream HTTP (INV-1)
- 4.5 openai.* exception hierarchy translates to Dify InvokeError subclasses
- 4.6 upstream_base_url honoured
- 4.7 gemini/bedrock stub raises InvokeError
- 4.8 no openai_api_key in logs (INV-6)

Tests stub the openai SDK + the SpendGuard sidecar client so they NEVER
hit the network and never require a real sidecar.
"""

from __future__ import annotations

import logging
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

dify_plugin = pytest.importorskip(
    "dify_plugin",
    reason="dify-plugin SDK requires Python 3.12+",
)

from dify_plugin.entities.model.message import (  # noqa: E402
    SystemPromptMessage,
    UserPromptMessage,
)
from dify_plugin.errors.model import (  # noqa: E402
    InvokeAuthorizationError,
    InvokeError,
    InvokeRateLimitError,
    InvokeServerUnavailableError,
)
from spendguard.errors import DecisionDenied  # noqa: E402

from models.llm._upstream import build_upstream_client  # noqa: E402
from models.llm._upstream.openai import OpenAIUpstream  # noqa: E402
from models.llm.spendguard_llm import SpendGuardLLM  # noqa: E402

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_openai_response(
    *, content: str = "hi back", prompt_tokens: int = 5,
    completion_tokens: int = 2, response_id: str = "chatcmpl-xyz",
    system_fingerprint: str = "fp_test",
):
    return SimpleNamespace(
        id=response_id,
        system_fingerprint=system_fingerprint,
        choices=[
            SimpleNamespace(message=SimpleNamespace(content=content)),
        ],
        usage=SimpleNamespace(
            prompt_tokens=prompt_tokens,
            completion_tokens=completion_tokens,
            total_tokens=prompt_tokens + completion_tokens,
        ),
    )


def _make_credentials(**overrides):
    base = {
        "upstream_provider": "openai",
        "openai_api_key": "sk-secret-do-not-log",
        "upstream_base_url": "",
        "spendguard_sidecar_address": "/tmp/sg.sock",
        "spendguard_tenant_id": "tenant-1",
        "spendguard_budget_id": "bud-1",
        "spendguard_window_instance_id": "win-1",
    }
    base.update(overrides)
    return base


def _stub_sidecar_client():
    """Sidecar client that returns CONTINUE + reservation_ids=('res-1',)."""
    client = MagicMock()
    client.tenant_id = "tenant-1"
    client.session_id = "session-1"
    client.request_decision = AsyncMock(return_value=SimpleNamespace(
        decision="CONTINUE",
        decision_id="dec-1",
        reservation_ids=("res-1",),
        audit_decision_event_id="audit-1",
    ))
    client.emit_llm_call_post = AsyncMock(return_value=None)
    return client


def _make_llm_with_seeded_reservation(client):
    """Build a SpendGuardLLM with the reservation delegate pre-seeded
    to use ``client``, bypassing _ensure_client / network.

    NOTE: ``_DaemonLoop`` is intentionally NOT eagerly initialised here.
    The first ``llm._invoke`` call lazy-starts the loop. This keeps the
    test isolation surface small and avoids gevent monkey-patching
    interfering with pytest-asyncio's per-test event loop.
    """
    llm = SpendGuardLLM.__new__(SpendGuardLLM)
    # Side-step pydantic init by directly instantiating reservation
    # delegate and seeding the client.
    from models.llm._DifyReservation import _DifyReservation
    res = _DifyReservation(socket_path="/sock", tenant_id="tenant-1")
    res._client = client
    SpendGuardLLM._reservation = res
    llm._reservation = res
    return llm


# ---------------------------------------------------------------------------
# O01 — happy-path ALLOW (review-standards.md 4.1 + 4.2 + 4.3)
# ---------------------------------------------------------------------------

def test_O01_invoke_allow_returns_llm_result_with_real_usage():
    """ALLOW: reserve -> openai mock -> commit_success with real tokens.
    INV-5: commit reads real prompt_tokens + completion_tokens from
    response.usage (not estimator).
    """
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_openai_response(
        content="hello world", prompt_tokens=11, completion_tokens=4,
    )
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        mock_openai_mod.OpenAI.return_value.chat.completions.create.return_value = response
        result = llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={"temperature": 0.7, "max_tokens": 100},
            stream=False,
        )
    assert result.message.content == "hello world"
    assert result.usage.prompt_tokens == 11
    assert result.usage.completion_tokens == 4
    assert result.usage.total_tokens == 15
    # 4.2: model passed to OpenAI has prefix stripped.
    create_kwargs = mock_openai_mod.OpenAI.return_value.chat.completions.create.call_args.kwargs
    assert create_kwargs["model"] == "gpt-4o-mini"
    # 4.3: commit fired with real tokens.
    commit_kwargs = client.emit_llm_call_post.await_args.kwargs
    assert commit_kwargs["outcome"] == "SUCCESS"
    assert commit_kwargs["actual_input_tokens"] == 11
    assert commit_kwargs["actual_output_tokens"] == 4
    # commit amount = real tokens (proxy in v1; sidecar reconciles)
    assert commit_kwargs["estimated_amount_atomic"] == "15"


# ---------------------------------------------------------------------------
# O02 — DENY never hits the upstream provider (INV-1 / 4.4)
# ---------------------------------------------------------------------------

def test_O02_deny_does_not_hit_openai():
    """INV-1: DENY -> ZERO openai.OpenAI invocations + raises
    InvokeAuthorizationError. Plus: DENY exception preserves decision_id.
    """
    client = MagicMock()
    client.tenant_id = "tenant-1"
    client.session_id = "session-1"
    client.request_decision = AsyncMock(
        side_effect=DecisionDenied(
            "budget exhausted",
            decision_id="dec-deny",
            reason_codes=["budget.exhausted"],
        ),
    )
    client.emit_llm_call_post = AsyncMock(return_value=None)
    llm = _make_llm_with_seeded_reservation(client)
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        with pytest.raises(InvokeAuthorizationError, match="dec-deny"):
            llm._invoke(
                model="spendguard/gpt-4o-mini",
                credentials=_make_credentials(),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )
        # INV-1: openai.OpenAI() never called -> no upstream HTTP.
        assert mock_openai_mod.OpenAI.call_count == 0


# ---------------------------------------------------------------------------
# O03 — sidecar unavailable -> InvokeServerUnavailableError (HTTP 503)
# ---------------------------------------------------------------------------

def test_O03_sidecar_unavailable_translates_to_server_unavailable():
    """SidecarUnavailable -> InvokeServerUnavailableError + ZERO openai
    invocation (INV-1-adjacent: failed reserve doesn't pass through)."""
    from spendguard.errors import SidecarUnavailable

    client = MagicMock()
    client.tenant_id = "tenant-1"
    client.session_id = "session-1"
    client.request_decision = AsyncMock(
        side_effect=SidecarUnavailable("UDS connection refused"),
    )
    client.emit_llm_call_post = AsyncMock(return_value=None)
    llm = _make_llm_with_seeded_reservation(client)
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        with pytest.raises(InvokeServerUnavailableError):
            llm._invoke(
                model="spendguard/gpt-4o-mini",
                credentials=_make_credentials(),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )
        assert mock_openai_mod.OpenAI.call_count == 0


# ---------------------------------------------------------------------------
# O04 — openai.AuthenticationError -> InvokeAuthorizationError (4.5)
# ---------------------------------------------------------------------------

def test_O04_openai_auth_error_translates_to_invoke_authorization_error():
    """4.5: AuthenticationError -> InvokeAuthorizationError. Also: the
    reservation is released on upstream failure (TTL backstop)."""
    import openai

    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        # Re-raise the actual openai.AuthenticationError so the upstream's
        # except branch handles it.
        mock_openai_mod.AuthenticationError = openai.AuthenticationError
        mock_openai_mod.RateLimitError = openai.RateLimitError
        mock_openai_mod.APIConnectionError = openai.APIConnectionError
        mock_openai_mod.APIError = openai.APIError
        mock_openai_mod.APIStatusError = openai.APIStatusError
        mock_openai_mod.BadRequestError = openai.BadRequestError
        mock_openai_mod.OpenAI.return_value.chat.completions.create.side_effect = (
            openai.AuthenticationError(
                "invalid key", response=MagicMock(status_code=401), body=None,
            )
        )
        with pytest.raises(InvokeAuthorizationError, match="openai authentication"):
            llm._invoke(
                model="spendguard/gpt-4o-mini",
                credentials=_make_credentials(),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )
    # Release fired (2 emit calls: would-be commit suppressed, release fired)
    # We assert at least one emit went out as a release with outcome=FAILURE.
    emit_calls = client.emit_llm_call_post.await_args_list
    release_calls = [c for c in emit_calls if c.kwargs.get("outcome") == "FAILURE"]
    assert len(release_calls) >= 1


# ---------------------------------------------------------------------------
# O05 — openai.RateLimitError -> InvokeRateLimitError (4.5)
# ---------------------------------------------------------------------------

def test_O05_openai_rate_limit_translates_to_invoke_rate_limit():
    """4.5: openai.RateLimitError -> Dify InvokeRateLimitError."""
    import openai

    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        for name in (
            "AuthenticationError", "RateLimitError", "APIConnectionError",
            "APIError", "APIStatusError", "BadRequestError",
        ):
            setattr(mock_openai_mod, name, getattr(openai, name))
        mock_openai_mod.OpenAI.return_value.chat.completions.create.side_effect = (
            openai.RateLimitError(
                "429 too many", response=MagicMock(status_code=429), body=None,
            )
        )
        with pytest.raises(InvokeRateLimitError):
            llm._invoke(
                model="spendguard/gpt-4o-mini",
                credentials=_make_credentials(),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )


# ---------------------------------------------------------------------------
# O06 — upstream_base_url honoured (4.6)
# ---------------------------------------------------------------------------

def test_O06_upstream_base_url_passed_to_openai_client():
    """4.6: upstream_base_url -> openai.OpenAI(base_url=...). Empty
    string skipped."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    creds = _make_credentials(upstream_base_url="https://litellm.internal/v1")
    response = _make_openai_response()
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        mock_openai_mod.OpenAI.return_value.chat.completions.create.return_value = response
        llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=creds,
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=False,
        )
        init_kwargs = mock_openai_mod.OpenAI.call_args.kwargs
        assert init_kwargs["base_url"] == "https://litellm.internal/v1"


# ---------------------------------------------------------------------------
# O07 — gemini stub raises InvokeError (4.7)
# ---------------------------------------------------------------------------

def test_O07_gemini_upstream_raises_invoke_error_not_supported():
    """4.7: gemini selected in v1 -> InvokeError with explicit
    'not supported in this plugin version' message."""
    creds = _make_credentials(upstream_provider="gemini")
    with pytest.raises(InvokeError, match="not supported in this plugin version"):
        build_upstream_client(creds)


def test_O07b_bedrock_upstream_raises_invoke_error_not_supported():
    """4.7: bedrock similarly stubbed."""
    creds = _make_credentials(upstream_provider="bedrock")
    with pytest.raises(InvokeError, match="not supported"):
        build_upstream_client(creds)


def test_O07c_unknown_upstream_provider_raises_invoke_error():
    """4.7: unknown provider names raise InvokeError, not silent fall-through."""
    creds = _make_credentials(upstream_provider="palm-bison")
    with pytest.raises(InvokeError, match="unknown upstream provider"):
        build_upstream_client(creds)


# ---------------------------------------------------------------------------
# O08 — no openai_api_key in logs (INV-6 / 4.8)
# ---------------------------------------------------------------------------

def test_O08_no_secret_material_in_logs(caplog):
    """INV-6 / 4.8: openai_api_key MUST never appear in log records,
    even partially. We test by triggering a failure path that logs
    extensively and asserting the secret never surfaces."""
    secret = "sk-very-secret-XXXX1234"
    creds = _make_credentials(openai_api_key=secret)
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)

    import openai

    caplog.set_level(logging.DEBUG, logger="spendguard")
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        for name in (
            "AuthenticationError", "RateLimitError", "APIConnectionError",
            "APIError", "APIStatusError", "BadRequestError",
        ):
            setattr(mock_openai_mod, name, getattr(openai, name))
        mock_openai_mod.OpenAI.return_value.chat.completions.create.side_effect = (
            openai.APIConnectionError(request=MagicMock())
        )
        with pytest.raises(InvokeError):
            llm._invoke(
                model="spendguard/gpt-4o-mini",
                credentials=creds,
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )
    for record in caplog.records:
        msg = record.getMessage()
        assert secret not in msg, f"secret leaked into log: {msg!r}"
        assert "sk-very-secret" not in msg


# ---------------------------------------------------------------------------
# O09 — missing openai_api_key triggers actionable error
# ---------------------------------------------------------------------------

def test_O09_missing_openai_api_key_raises_invoke_authorization_error():
    """Missing key surfaces as InvokeAuthorizationError naming the field."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    creds = _make_credentials(openai_api_key="")
    with patch("models.llm._upstream.openai.openai"):
        with pytest.raises(InvokeAuthorizationError, match="openai_api_key"):
            llm._invoke(
                model="spendguard/gpt-4o-mini",
                credentials=creds,
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )


# ---------------------------------------------------------------------------
# O10 — streaming in v1 -> InvokeError (SLICE 6 deferral)
# ---------------------------------------------------------------------------

def test_O10_streaming_now_returns_generator_after_slice_6():
    """SLICE 6 implements ``_stream_generate`` for OpenAI + Anthropic.

    SLICE 4 stub raised ``InvokeError`` when ``stream=True`` was passed
    (because the streaming path was deferred). SLICE 6 wires the SSE
    proxy; ``_invoke(stream=True)`` now returns a generator instead.

    This test pins the SLICE 6 contract: streaming returns a generator
    object (not a single ``LLMResult``). Per-chunk semantics are covered
    by ``test_streaming.py``.
    """
    from collections.abc import Generator
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    # We don't need to mock anything — generators are lazy. Just check
    # the return type contract change from SLICE 4 (InvokeError) -> SLICE 6
    # (Generator).
    result = llm._invoke(
        model="spendguard/gpt-4o-mini",
        credentials=_make_credentials(),
        prompt_messages=[UserPromptMessage(content="hi")],
        model_parameters={},
        stream=True,
    )
    # Generator returned; we deliberately do NOT iterate it here to keep
    # the test focused on the API contract change.
    assert isinstance(result, Generator)


# ---------------------------------------------------------------------------
# O11 — multi-message prompt translation
# ---------------------------------------------------------------------------

def test_O11_multi_message_prompt_translates_to_openai_messages_list():
    """Dify [SystemPromptMessage, UserPromptMessage] translates correctly
    to OpenAI chat completions messages list."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_openai_response()
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        mock_openai_mod.OpenAI.return_value.chat.completions.create.return_value = response
        llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials(),
            prompt_messages=[
                SystemPromptMessage(content="You are helpful."),
                UserPromptMessage(content="What's the weather?"),
            ],
            model_parameters={},
            stream=False,
        )
        call_kwargs = mock_openai_mod.OpenAI.return_value.chat.completions.create.call_args.kwargs
        msgs = call_kwargs["messages"]
        assert len(msgs) == 2
        assert msgs[0]["role"] == "system"
        assert msgs[0]["content"] == "You are helpful."
        assert msgs[1]["role"] == "user"
        assert msgs[1]["content"] == "What's the weather?"


# ---------------------------------------------------------------------------
# O12 — usage extraction handles None usage gracefully
# ---------------------------------------------------------------------------

def test_O12_response_usage_none_uses_zero_tokens():
    """Some compat layers (litellm proxy) omit usage; we default to 0
    rather than crash. The commit still fires (with 0 amount) so the
    audit chain is closed; estimator-fallback warn is a SLICE 6 streaming
    concern, not v1 non-streaming."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = SimpleNamespace(
        id="chatcmpl-no-usage",
        system_fingerprint="fp_test",
        choices=[SimpleNamespace(message=SimpleNamespace(content="ok"))],
        usage=None,
    )
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        mock_openai_mod.OpenAI.return_value.chat.completions.create.return_value = response
        result = llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=False,
        )
    assert result.message.content == "ok"
    assert result.usage.prompt_tokens == 0
    assert result.usage.completion_tokens == 0
    commit_kwargs = client.emit_llm_call_post.await_args.kwargs
    assert commit_kwargs["actual_input_tokens"] == 0
    assert commit_kwargs["actual_output_tokens"] == 0


# ---------------------------------------------------------------------------
# O13 — openai client construction NOT cached at module level (4.1)
# ---------------------------------------------------------------------------

def test_O13_openai_client_constructed_per_call_not_cached():
    """4.1: two calls with different api_keys construct two separate
    openai.OpenAI clients (multi-workspace safety)."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_openai_response()
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        mock_openai_mod.OpenAI.return_value.chat.completions.create.return_value = response
        llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials(openai_api_key="sk-key-A"),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=False,
        )
        llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials(openai_api_key="sk-key-B"),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=False,
        )
        # 4.1 INVARIANT: two distinct openai.OpenAI(api_key=...) calls.
        assert mock_openai_mod.OpenAI.call_count == 2
        first_key = mock_openai_mod.OpenAI.call_args_list[0].kwargs["api_key"]
        second_key = mock_openai_mod.OpenAI.call_args_list[1].kwargs["api_key"]
        assert first_key == "sk-key-A"
        assert second_key == "sk-key-B"


# ---------------------------------------------------------------------------
# O14 — get_num_tokens fallback heuristic
# ---------------------------------------------------------------------------

def test_O14_get_num_tokens_chars_over_4_heuristic():
    """v1 plugin uses chars/4 fallback; SLICE 5 routes through sidecar
    count_tokens. Test pins the v1 behavior so regression catches the
    move."""
    llm = SpendGuardLLM.__new__(SpendGuardLLM)
    n = llm.get_num_tokens(
        model="spendguard/gpt-4o-mini",
        credentials=_make_credentials(),
        prompt_messages=[
            UserPromptMessage(content="x" * 40),  # 40 chars
        ],
    )
    assert n == 10  # 40 / 4
    # Empty prompt -> at least 1.
    n_empty = llm.get_num_tokens(
        model="spendguard/gpt-4o-mini",
        credentials=_make_credentials(),
        prompt_messages=[UserPromptMessage(content="")],
    )
    assert n_empty == 1


# ---------------------------------------------------------------------------
# O15 — OpenAIUpstream is stateless (multi-workspace safety)
# ---------------------------------------------------------------------------

def test_O15_openai_upstream_no_module_level_client_state():
    """4.1: OpenAIUpstream.generate creates a fresh openai.OpenAI
    instance per call. Test by assertion of instance equality (None)."""
    upstream = OpenAIUpstream()
    # Class has no '_client' or 'session' attribute that would betray
    # module-level caching.
    assert not hasattr(upstream, "_client")
    assert not hasattr(upstream, "_session")
    assert not hasattr(OpenAIUpstream, "_client")


# ---------------------------------------------------------------------------
# O16 — stop tokens forwarded to OpenAI
# ---------------------------------------------------------------------------

def test_O16_stop_tokens_forwarded_to_openai():
    """``stop`` parameter passes through to upstream kwargs."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_openai_response()
    with patch("models.llm._upstream.openai.openai") as mock_openai_mod:
        mock_openai_mod.OpenAI.return_value.chat.completions.create.return_value = response
        llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stop=["\nUSER:", "STOP"],
            stream=False,
        )
        create_kwargs = mock_openai_mod.OpenAI.return_value.chat.completions.create.call_args.kwargs
        assert create_kwargs["stop"] == ["\nUSER:", "STOP"]
