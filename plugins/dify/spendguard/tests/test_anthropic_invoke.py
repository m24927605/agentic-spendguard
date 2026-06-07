"""Unit tests for the Anthropic upstream + SpendGuardLLM._invoke path.

SLICE 5 coverage (review-standards.md):
- 4.1 Anthropic client constructed per-call from credentials (not cached)
- 4.2 spendguard/ prefix stripped before upstream wire
- 4.3 real usage from response.usage.input_tokens + output_tokens (INV-5)
- 4.4 DENY -> ZERO upstream HTTP (INV-1)
- 4.5 anthropic.* exception hierarchy translates to Dify InvokeError subclasses
- 4.6 upstream_base_url honoured
- 4.8 no anthropic_api_key in logs (INV-6)
- 5.1 system messages SPLIT into top-level system param + filtered messages list
- 5.2 max_tokens defaults to plugin floor when Dify form omits it
- 5.3 sidecar /v1/tokenize companion routing in get_num_tokens (SLICE 5)
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
from models.llm._upstream.anthropic import (  # noqa: E402
    AnthropicUpstream,
    _prompt_messages_to_anthropic,
)
from models.llm.spendguard_llm import SpendGuardLLM  # noqa: E402

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_anthropic_response(
    *, content: str = "hi back", input_tokens: int = 5,
    output_tokens: int = 2, response_id: str = "msg_xyz",
):
    """Build a fake anthropic.Message response shape."""
    return SimpleNamespace(
        id=response_id,
        content=[
            SimpleNamespace(type="text", text=content),
        ],
        usage=SimpleNamespace(
            input_tokens=input_tokens,
            output_tokens=output_tokens,
        ),
    )


def _make_credentials(**overrides):
    base = {
        "upstream_provider": "anthropic",
        "anthropic_api_key": "sk-ant-secret-do-not-log",
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
    """Build a SpendGuardLLM with the reservation delegate pre-seeded."""
    llm = SpendGuardLLM.__new__(SpendGuardLLM)
    from models.llm._DifyReservation import _DifyReservation
    res = _DifyReservation(socket_path="/sock", tenant_id="tenant-1")
    res._client = client
    SpendGuardLLM._reservation = res
    llm._reservation = res
    return llm


# ---------------------------------------------------------------------------
# A01 — happy-path ALLOW (4.1 + 4.2 + 4.3)
# ---------------------------------------------------------------------------

def test_A01_invoke_allow_returns_llm_result_with_real_usage():
    """ALLOW: reserve -> anthropic mock -> commit_success with real tokens.
    INV-5: commit reads real input_tokens + output_tokens from usage."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_anthropic_response(
        content="hello world", input_tokens=11, output_tokens=4,
    )
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        mock_anthropic_mod.Anthropic.return_value.messages.create.return_value = response
        result = llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={"temperature": 0.7, "max_tokens": 100},
            stream=False,
        )
    assert result.message.content == "hello world"
    assert result.usage.prompt_tokens == 11
    assert result.usage.completion_tokens == 4
    assert result.usage.total_tokens == 15
    # 4.2: model passed to Anthropic has prefix stripped.
    create_kwargs = mock_anthropic_mod.Anthropic.return_value.messages.create.call_args.kwargs
    assert create_kwargs["model"] == "claude-3-5-sonnet-latest"
    assert create_kwargs["max_tokens"] == 100
    # 4.3: commit fired with real tokens.
    commit_kwargs = client.emit_llm_call_post.await_args.kwargs
    assert commit_kwargs["outcome"] == "SUCCESS"
    assert commit_kwargs["actual_input_tokens"] == 11
    assert commit_kwargs["actual_output_tokens"] == 4
    assert commit_kwargs["estimated_amount_atomic"] == "15"


# ---------------------------------------------------------------------------
# A02 — DENY never hits the upstream provider (INV-1)
# ---------------------------------------------------------------------------

def test_A02_deny_does_not_hit_anthropic():
    """INV-1: DENY -> ZERO anthropic.Anthropic invocations + raises
    InvokeAuthorizationError. Plus: DENY exception preserves decision_id."""
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
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        with pytest.raises(InvokeAuthorizationError, match="dec-deny"):
            llm._invoke(
                model="spendguard/claude-3-5-sonnet-latest",
                credentials=_make_credentials(),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )
        # INV-1: anthropic.Anthropic() never called -> no upstream HTTP.
        assert mock_anthropic_mod.Anthropic.call_count == 0


# ---------------------------------------------------------------------------
# A03 — anthropic.AuthenticationError -> InvokeAuthorizationError (4.5)
# ---------------------------------------------------------------------------

def test_A03_anthropic_auth_error_translates_to_invoke_authorization_error():
    """4.5: AuthenticationError -> InvokeAuthorizationError. Also: the
    reservation is released on upstream failure (TTL backstop)."""
    import anthropic

    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        mock_anthropic_mod.AuthenticationError = anthropic.AuthenticationError
        mock_anthropic_mod.RateLimitError = anthropic.RateLimitError
        mock_anthropic_mod.APIConnectionError = anthropic.APIConnectionError
        mock_anthropic_mod.APIError = anthropic.APIError
        mock_anthropic_mod.APIStatusError = anthropic.APIStatusError
        mock_anthropic_mod.BadRequestError = anthropic.BadRequestError
        mock_anthropic_mod.Anthropic.return_value.messages.create.side_effect = (
            anthropic.AuthenticationError(
                "invalid key", response=MagicMock(status_code=401), body=None,
            )
        )
        with pytest.raises(InvokeAuthorizationError, match="anthropic authentication"):
            llm._invoke(
                model="spendguard/claude-3-5-sonnet-latest",
                credentials=_make_credentials(),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )
    # Release fired.
    emit_calls = client.emit_llm_call_post.await_args_list
    release_calls = [c for c in emit_calls if c.kwargs.get("outcome") == "FAILURE"]
    assert len(release_calls) >= 1


# ---------------------------------------------------------------------------
# A04 — anthropic.RateLimitError -> InvokeRateLimitError (4.5)
# ---------------------------------------------------------------------------

def test_A04_anthropic_rate_limit_translates_to_invoke_rate_limit():
    """4.5: anthropic.RateLimitError -> Dify InvokeRateLimitError."""
    import anthropic

    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        for name in (
            "AuthenticationError", "RateLimitError", "APIConnectionError",
            "APIError", "APIStatusError", "BadRequestError",
        ):
            setattr(mock_anthropic_mod, name, getattr(anthropic, name))
        mock_anthropic_mod.Anthropic.return_value.messages.create.side_effect = (
            anthropic.RateLimitError(
                "429 too many", response=MagicMock(status_code=429), body=None,
            )
        )
        with pytest.raises(InvokeRateLimitError):
            llm._invoke(
                model="spendguard/claude-3-5-sonnet-latest",
                credentials=_make_credentials(),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )


# ---------------------------------------------------------------------------
# A05 — anthropic 529 (Overloaded) -> InvokeServerUnavailable (4.5 / 5.4)
# ---------------------------------------------------------------------------

def test_A05_anthropic_overloaded_529_translates_to_server_unavailable():
    """4.5: Anthropic 529 (Overloaded) -> InvokeServerUnavailableError so
    Dify clients see a retryable error."""
    import anthropic

    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        for name in (
            "AuthenticationError", "RateLimitError", "APIConnectionError",
            "APIError", "APIStatusError", "BadRequestError",
        ):
            setattr(mock_anthropic_mod, name, getattr(anthropic, name))
        # Construct an APIStatusError with 529 to mirror Anthropic's
        # actual overload signal (HTTP 529 Overloaded).
        status_exc = anthropic.APIStatusError.__new__(
            anthropic.APIStatusError,
        )
        status_exc.status_code = 529
        status_exc.message = "Overloaded"
        status_exc.args = ("Overloaded",)
        # Tag onto Exception base
        Exception.__init__(status_exc, "Overloaded")
        mock_anthropic_mod.Anthropic.return_value.messages.create.side_effect = status_exc
        with pytest.raises(InvokeServerUnavailableError):
            llm._invoke(
                model="spendguard/claude-3-5-sonnet-latest",
                credentials=_make_credentials(),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )


# ---------------------------------------------------------------------------
# A06 — upstream_base_url honoured (4.6)
# ---------------------------------------------------------------------------

def test_A06_upstream_base_url_passed_to_anthropic_client():
    """4.6: upstream_base_url -> anthropic.Anthropic(base_url=...)."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    creds = _make_credentials(upstream_base_url="https://proxy.internal/v1")
    response = _make_anthropic_response()
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        mock_anthropic_mod.Anthropic.return_value.messages.create.return_value = response
        llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=creds,
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=False,
        )
        init_kwargs = mock_anthropic_mod.Anthropic.call_args.kwargs
        assert init_kwargs["base_url"] == "https://proxy.internal/v1"


# ---------------------------------------------------------------------------
# A07 — no anthropic_api_key in logs (INV-6 / 4.8)
# ---------------------------------------------------------------------------

def test_A07_no_secret_material_in_logs(caplog):
    """INV-6 / 4.8: anthropic_api_key MUST never appear in log records."""
    secret = "sk-ant-very-secret-XXXX1234"
    creds = _make_credentials(anthropic_api_key=secret)
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)

    import anthropic

    caplog.set_level(logging.DEBUG, logger="spendguard")
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        for name in (
            "AuthenticationError", "RateLimitError", "APIConnectionError",
            "APIError", "APIStatusError", "BadRequestError",
        ):
            setattr(mock_anthropic_mod, name, getattr(anthropic, name))
        mock_anthropic_mod.Anthropic.return_value.messages.create.side_effect = (
            anthropic.APIConnectionError(request=MagicMock())
        )
        with pytest.raises(InvokeError):
            llm._invoke(
                model="spendguard/claude-3-5-sonnet-latest",
                credentials=creds,
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )
    for record in caplog.records:
        msg = record.getMessage()
        assert secret not in msg, f"secret leaked into log: {msg!r}"


# ---------------------------------------------------------------------------
# A08 — system message SPLIT into top-level + filtered messages (5.1)
# ---------------------------------------------------------------------------

def test_A08_system_message_split_into_top_level_system_param():
    """5.1: SystemPromptMessage -> top-level ``system`` kwarg; messages
    list keeps only user/assistant rows. Mirrors Anthropic API shape."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_anthropic_response()
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        mock_anthropic_mod.Anthropic.return_value.messages.create.return_value = response
        llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(),
            prompt_messages=[
                SystemPromptMessage(content="You are helpful."),
                UserPromptMessage(content="What's the weather?"),
            ],
            model_parameters={},
            stream=False,
        )
        call_kwargs = mock_anthropic_mod.Anthropic.return_value.messages.create.call_args.kwargs
        # system message extracted to top-level kwarg
        assert call_kwargs["system"] == "You are helpful."
        # messages list only contains the user row (NO role=system)
        msgs = call_kwargs["messages"]
        assert len(msgs) == 1
        assert msgs[0]["role"] == "user"
        assert msgs[0]["content"] == "What's the weather?"


def test_A08b_multiple_system_messages_joined_with_double_newline():
    """5.1: multiple system messages join with \\n\\n per Anthropic guidance."""
    sys_prompt, messages = _prompt_messages_to_anthropic([
        SystemPromptMessage(content="Be concise."),
        SystemPromptMessage(content="Be friendly."),
        UserPromptMessage(content="Hello."),
    ])
    assert sys_prompt == "Be concise.\n\nBe friendly."
    assert len(messages) == 1
    assert messages[0]["role"] == "user"


# ---------------------------------------------------------------------------
# A09 — max_tokens defaults to floor when omitted (5.2)
# ---------------------------------------------------------------------------

def test_A09_max_tokens_defaults_when_omitted():
    """5.2: Anthropic API requires max_tokens; we default to the plugin
    floor (1024) when the Dify form omits it."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_anthropic_response()
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        mock_anthropic_mod.Anthropic.return_value.messages.create.return_value = response
        llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},  # NO max_tokens
            stream=False,
        )
        call_kwargs = mock_anthropic_mod.Anthropic.return_value.messages.create.call_args.kwargs
        assert call_kwargs["max_tokens"] == 1024


# ---------------------------------------------------------------------------
# A10 — anthropic provider routing via build_upstream_client (4.7)
# ---------------------------------------------------------------------------

def test_A10_anthropic_provider_routes_to_anthropic_upstream():
    """build_upstream_client(credentials) with upstream_provider=anthropic
    returns an AnthropicUpstream instance (lazy import works)."""
    creds = _make_credentials(upstream_provider="anthropic")
    upstream = build_upstream_client(creds)
    assert isinstance(upstream, AnthropicUpstream)


# ---------------------------------------------------------------------------
# A11 — anthropic client constructed per-call (4.1 / multi-workspace safety)
# ---------------------------------------------------------------------------

def test_A11_anthropic_client_constructed_per_call_not_cached():
    """4.1: two calls with different api_keys construct two separate
    Anthropic clients (multi-workspace safety)."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_anthropic_response()
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        mock_anthropic_mod.Anthropic.return_value.messages.create.return_value = response
        llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(anthropic_api_key="sk-ant-A"),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=False,
        )
        llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(anthropic_api_key="sk-ant-B"),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=False,
        )
        assert mock_anthropic_mod.Anthropic.call_count == 2
        first_key = mock_anthropic_mod.Anthropic.call_args_list[0].kwargs["api_key"]
        second_key = mock_anthropic_mod.Anthropic.call_args_list[1].kwargs["api_key"]
        assert first_key == "sk-ant-A"
        assert second_key == "sk-ant-B"


# ---------------------------------------------------------------------------
# A12 — missing anthropic_api_key triggers actionable error
# ---------------------------------------------------------------------------

def test_A12_missing_anthropic_api_key_raises_invoke_authorization_error():
    """Missing key surfaces as InvokeAuthorizationError naming the field."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    creds = _make_credentials(anthropic_api_key="")
    with patch("models.llm._upstream.anthropic.anthropic"):
        with pytest.raises(InvokeAuthorizationError, match="anthropic_api_key"):
            llm._invoke(
                model="spendguard/claude-3-5-sonnet-latest",
                credentials=creds,
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=False,
            )


# ---------------------------------------------------------------------------
# A13 — content blocks of mixed types (text + tool_use): only text extracted
# ---------------------------------------------------------------------------

def test_A13_content_blocks_mixed_types_extracts_text_only():
    """v1 forwards text-only; tool_use blocks are dropped (tool-calling
    lands in v1.1). The response's TextBlock list is concatenated."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = SimpleNamespace(
        id="msg_mixed",
        content=[
            SimpleNamespace(type="text", text="Part A "),
            SimpleNamespace(type="tool_use", name="get_weather", input={}),
            SimpleNamespace(type="text", text="Part B"),
        ],
        usage=SimpleNamespace(input_tokens=3, output_tokens=4),
    )
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        mock_anthropic_mod.Anthropic.return_value.messages.create.return_value = response
        result = llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=False,
        )
    assert result.message.content == "Part A Part B"


# ---------------------------------------------------------------------------
# A14 — frequency_penalty/presence_penalty NOT forwarded (Anthropic rejects them)
# ---------------------------------------------------------------------------

def test_A14_unsupported_openai_params_dropped_for_anthropic():
    """5.2: Anthropic rejects frequency_penalty / presence_penalty. We
    silently drop them rather than forward and have the upstream 400."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    response = _make_anthropic_response()
    with patch("models.llm._upstream.anthropic.anthropic") as mock_anthropic_mod:
        mock_anthropic_mod.Anthropic.return_value.messages.create.return_value = response
        llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={
                "temperature": 0.5,
                "top_p": 0.9,
                "top_k": 50,
                "frequency_penalty": 0.5,  # NOT forwarded
                "presence_penalty": 0.5,   # NOT forwarded
            },
            stream=False,
        )
        call_kwargs = mock_anthropic_mod.Anthropic.return_value.messages.create.call_args.kwargs
        assert call_kwargs["temperature"] == 0.5
        assert call_kwargs["top_p"] == 0.9
        assert call_kwargs["top_k"] == 50
        assert "frequency_penalty" not in call_kwargs
        assert "presence_penalty" not in call_kwargs


# ---------------------------------------------------------------------------
# A15 — get_num_tokens via sidecar /v1/tokenize companion (SLICE 5)
# ---------------------------------------------------------------------------

def test_A15_get_num_tokens_uses_sidecar_companion_when_configured(monkeypatch):
    """5.3 / SLICE 5: get_num_tokens routes through sidecar /v1/tokenize
    when SPENDGUARD_SIDECAR_HTTP_URL is set; companion returns the count."""
    monkeypatch.setenv("SPENDGUARD_SIDECAR_HTTP_URL", "http://sidecar:9090")
    llm = SpendGuardLLM.__new__(SpendGuardLLM)

    # Patch httpx.Client at the real module level so the inline
    # `import httpx` inside _sidecar_tokenize uses our mock.
    fake_response = MagicMock()
    fake_response.status_code = 200
    fake_response.json.return_value = {"token_count": 42}
    with patch("httpx.Client") as mock_httpx_client:
        mock_httpx_client.return_value.__enter__.return_value.post.return_value = fake_response
        mock_httpx_client.return_value.__exit__.return_value = None
        n = llm.get_num_tokens(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="x" * 40)],
        )
    assert n == 42  # sidecar returned 42 (not chars/4=10)


def test_A15b_get_num_tokens_falls_back_to_chars4_when_companion_unreachable(monkeypatch):
    """5.3: when the companion URL is set but the call fails, fall back
    to chars/4 silently (with a WARN)."""
    monkeypatch.setenv("SPENDGUARD_SIDECAR_HTTP_URL", "http://sidecar:9090")
    llm = SpendGuardLLM.__new__(SpendGuardLLM)
    with patch("httpx.Client") as mock_httpx_client:
        mock_httpx_client.return_value.__enter__.return_value.post.side_effect = (
            ConnectionError("no route to host")
        )
        n = llm.get_num_tokens(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials(),
            prompt_messages=[UserPromptMessage(content="x" * 40)],
        )
    assert n == 10  # 40 / 4 fallback


def test_A15c_get_num_tokens_falls_back_to_chars4_when_no_companion_url():
    """5.3: when SPENDGUARD_SIDECAR_HTTP_URL is unset and credentials
    don't override, use chars/4 directly without httpx import."""
    llm = SpendGuardLLM.__new__(SpendGuardLLM)
    creds = _make_credentials()
    creds.pop("spendguard_sidecar_http_url", None)
    # Ensure env var not set
    import os
    os.environ.pop("SPENDGUARD_SIDECAR_HTTP_URL", None)
    n = llm.get_num_tokens(
        model="spendguard/claude-3-5-sonnet-latest",
        credentials=creds,
        prompt_messages=[UserPromptMessage(content="x" * 40)],
    )
    assert n == 10
