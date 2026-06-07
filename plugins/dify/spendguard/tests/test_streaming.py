"""Unit tests for the SpendGuardLLM streaming path (SLICE 6).

review-standards.md SLICE 6 checklist coverage:
- 6.1 reserve fires once before any upstream HTTP (INV-1 streaming)
- 6.2 each upstream SSE event yields a Dify LLMResultChunk
- 6.3 OpenAI stream_options.include_usage=True is set unconditionally so
  commit gets real usage at end-of-stream
- 6.4 estimator-snapshot fallback (chars/4) when upstream omits usage
- 6.5 end-of-stream commit fires with accumulated usage
- 6.6 mid-stream upstream error -> release + InvokeError re-raise
- 6.7 DENY -> ZERO upstream HTTP (INV-1 invariant carries to streaming)
- 6.8 Anthropic message_start input_tokens + message_delta output_tokens
  accumulated correctly
"""

from __future__ import annotations

from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

dify_plugin = pytest.importorskip(
    "dify_plugin",
    reason="dify-plugin SDK requires Python 3.12+",
)

from dify_plugin.entities.model.llm import LLMResultChunk  # noqa: E402
from dify_plugin.entities.model.message import UserPromptMessage  # noqa: E402
from dify_plugin.errors.model import (  # noqa: E402
    InvokeAuthorizationError,
    InvokeError,
)
from spendguard.errors import DecisionDenied  # noqa: E402

from models.llm.spendguard_llm import SpendGuardLLM, _StreamingAccumulator  # noqa: E402

# ---------------------------------------------------------------------------
# Helpers — fake SSE streams
# ---------------------------------------------------------------------------

def _make_credentials(provider="openai", **overrides):
    base = {
        "upstream_provider": provider,
        "openai_api_key": "sk-secret-do-not-log",
        "anthropic_api_key": "sk-ant-secret",
        "upstream_base_url": "",
        "spendguard_sidecar_address": "/tmp/sg.sock",
        "spendguard_tenant_id": "tenant-1",
        "spendguard_budget_id": "bud-1",
        "spendguard_window_instance_id": "win-1",
    }
    base.update(overrides)
    return base


def _stub_sidecar_client():
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
    llm = SpendGuardLLM.__new__(SpendGuardLLM)
    from models.llm._DifyReservation import _DifyReservation
    res = _DifyReservation(socket_path="/sock", tenant_id="tenant-1")
    res._client = client
    SpendGuardLLM._reservation = res
    llm._reservation = res
    return llm


def _make_openai_chunk(*, content="", chunk_id="chatcmpl-stream-1", finish_reason=None, usage=None):
    """Build a fake ChatCompletionChunk shape."""
    return SimpleNamespace(
        id=chunk_id,
        choices=[SimpleNamespace(
            delta=SimpleNamespace(content=content),
            finish_reason=finish_reason,
        )] if (content or finish_reason) else [],
        usage=usage,
    )


def _make_openai_stream(content_parts, *, prompt_tokens=10, completion_tokens=5):
    """Yield content chunks then a terminal usage chunk."""
    for i, part in enumerate(content_parts):
        yield _make_openai_chunk(content=part, chunk_id=f"chatcmpl-stream-{i}")
    # Final stop chunk
    yield _make_openai_chunk(chunk_id="chatcmpl-stream-stop", finish_reason="stop", content="")
    # Final usage chunk (OpenAI sends this when include_usage=True)
    yield _make_openai_chunk(
        chunk_id="chatcmpl-stream-usage",
        usage=SimpleNamespace(
            prompt_tokens=prompt_tokens,
            completion_tokens=completion_tokens,
            total_tokens=prompt_tokens + completion_tokens,
        ),
    )


# ---------------------------------------------------------------------------
# S01 — happy path streaming (OpenAI) — yields chunks + commits real usage (6.1+6.2+6.5)
# ---------------------------------------------------------------------------

def test_S01_openai_stream_yields_chunks_and_commits_real_usage():
    """6.1+6.2+6.5: reserve fires once, chunks yielded, end-of-stream
    commit captures accumulated real usage from final usage chunk."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    fake_stream = list(_make_openai_stream(
        ["Hello ", "world!"],
        prompt_tokens=12,
        completion_tokens=3,
    ))
    with patch("openai.OpenAI") as mock_openai_cls:
        mock_openai_cls.return_value.chat.completions.create.return_value = iter(fake_stream)
        result = llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials("openai"),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=True,
        )
        chunks = list(result)
    # 6.2: one chunk per content delta (2 content + 1 stop chunk)
    assert all(isinstance(c, LLMResultChunk) for c in chunks)
    # Reconstruct content from chunk deltas — must equal "Hello world!"
    rebuilt = "".join(c.delta.message.content for c in chunks if c.delta.message.content)
    assert rebuilt == "Hello world!"
    # 6.5: end-of-stream commit fires with REAL usage from final chunk
    commit_kwargs = client.emit_llm_call_post.await_args.kwargs
    assert commit_kwargs["outcome"] == "SUCCESS"
    assert commit_kwargs["actual_input_tokens"] == 12
    assert commit_kwargs["actual_output_tokens"] == 3
    assert commit_kwargs["estimated_amount_atomic"] == "15"
    # 6.1: reserve fired exactly once (request_decision called once)
    assert client.request_decision.await_count == 1


# ---------------------------------------------------------------------------
# S02 — DENY -> zero upstream HTTP, no chunks (6.7 / INV-1 streaming)
# ---------------------------------------------------------------------------

def test_S02_deny_streaming_no_upstream_no_chunks():
    """6.7: DENY in streaming -> InvokeAuthorizationError without any
    upstream HTTP. We use a generator-eager raise pattern."""
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
    with patch("openai.OpenAI") as mock_openai_cls:
        with pytest.raises(InvokeAuthorizationError, match="dec-deny"):
            result = llm._invoke(
                model="spendguard/gpt-4o-mini",
                credentials=_make_credentials("openai"),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=True,
            )
            # Must iterate to materialise the generator's first action.
            list(result)
        # No openai.OpenAI() construction -> no upstream HTTP.
        assert mock_openai_cls.call_count == 0


# ---------------------------------------------------------------------------
# S03 — stream_options.include_usage=True always set (6.3)
# ---------------------------------------------------------------------------

def test_S03_openai_stream_options_include_usage_always_set():
    """6.3: OpenAI streaming MUST pass stream_options.include_usage=True
    so the final chunk carries usage for commit."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    fake_stream = list(_make_openai_stream(["hi"]))
    with patch("openai.OpenAI") as mock_openai_cls:
        mock_openai_cls.return_value.chat.completions.create.return_value = iter(fake_stream)
        result = llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials("openai"),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=True,
        )
        list(result)
    create_kwargs = mock_openai_cls.return_value.chat.completions.create.call_args.kwargs
    assert create_kwargs["stream"] is True
    assert create_kwargs["stream_options"] == {"include_usage": True}


# ---------------------------------------------------------------------------
# S04 — fallback estimator (chars/4) when upstream omits usage (6.4)
# ---------------------------------------------------------------------------

def test_S04_no_usage_chunk_falls_back_to_chars_estimate():
    """6.4: when no usage chunk arrives, commit with chars/4 estimate."""
    acc = _StreamingAccumulator()
    acc.append_text("hello world")  # 11 chars
    # No update_usage call -> had_usage stays False
    assert acc.had_usage is False
    acc.fallback_estimate()
    # chars/4 of "hello world" = 11/4 = 2 (int division), max 1
    assert acc.completion_tokens == 2
    assert acc.prompt_tokens == 0  # unknown


def test_S04b_stream_without_terminal_usage_chunk_commits_estimate():
    """End-to-end: stream WITHOUT a usage chunk falls back to estimate."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)
    # Stream with NO usage chunk
    chunks = [
        _make_openai_chunk(content="word1 ", chunk_id="c1"),
        _make_openai_chunk(content="word2", chunk_id="c2"),
        _make_openai_chunk(content="", chunk_id="c-stop", finish_reason="stop"),
    ]
    with patch("openai.OpenAI") as mock_openai_cls:
        mock_openai_cls.return_value.chat.completions.create.return_value = iter(chunks)
        result = llm._invoke(
            model="spendguard/gpt-4o-mini",
            credentials=_make_credentials("openai"),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=True,
        )
        list(result)
    commit_kwargs = client.emit_llm_call_post.await_args.kwargs
    # actual_input_tokens = 0 (unknown), completion = chars/4 of "word1 word2" = 11/4 = 2
    assert commit_kwargs["actual_input_tokens"] == 0
    assert commit_kwargs["actual_output_tokens"] == 2


# ---------------------------------------------------------------------------
# S05 — mid-stream APIError -> release + re-raise (6.6)
# ---------------------------------------------------------------------------

def test_S05_mid_stream_upstream_error_releases_and_reraises():
    """6.6: upstream raises APIError mid-stream -> release_failure fires
    AND the InvokeError surfaces to the Dify caller."""
    import openai

    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)

    def _stream_that_dies():
        yield _make_openai_chunk(content="partial", chunk_id="c1")
        raise openai.APIError(
            "upstream went away",
            request=MagicMock(),
            body=None,
        )

    with patch("openai.OpenAI") as mock_openai_cls:
        mock_openai_cls.return_value.chat.completions.create.return_value = _stream_that_dies()
        with pytest.raises(InvokeError):
            result = llm._invoke(
                model="spendguard/gpt-4o-mini",
                credentials=_make_credentials("openai"),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=True,
            )
            # Force iteration to materialise the error
            list(result)
    # Release fired with FAILURE
    emit_calls = client.emit_llm_call_post.await_args_list
    release_calls = [c for c in emit_calls if c.kwargs.get("outcome") == "FAILURE"]
    assert len(release_calls) >= 1


# ---------------------------------------------------------------------------
# S06 — Anthropic streaming yields chunks + accumulates input+output (6.8)
# ---------------------------------------------------------------------------

def test_S06_anthropic_stream_yields_chunks_and_accumulates_split_usage():
    """6.8: Anthropic stream — message_start carries input_tokens,
    message_delta carries output_tokens. Both accumulate correctly."""
    client = _stub_sidecar_client()
    llm = _make_llm_with_seeded_reservation(client)

    # Build fake Anthropic SSE event sequence
    msg_start = SimpleNamespace(
        type="message_start",
        message=SimpleNamespace(
            id="msg_stream_1",
            usage=SimpleNamespace(input_tokens=8, output_tokens=0),
        ),
    )
    delta1 = SimpleNamespace(
        type="content_block_delta",
        delta=SimpleNamespace(type="text_delta", text="Hello "),
    )
    delta2 = SimpleNamespace(
        type="content_block_delta",
        delta=SimpleNamespace(type="text_delta", text="from Claude"),
    )
    msg_delta = SimpleNamespace(
        type="message_delta",
        delta=SimpleNamespace(stop_reason="end_turn"),
        usage=SimpleNamespace(output_tokens=4),
    )

    events = [msg_start, delta1, delta2, msg_delta]

    # The Anthropic SDK's stream() returns a context manager that yields
    # events on iteration; mock the shape.
    class _FakeStreamCtx:
        def __enter__(self_inner):
            return iter(events)
        def __exit__(self_inner, *a):
            return False

    with patch("anthropic.Anthropic") as mock_anthropic_cls:
        mock_anthropic_cls.return_value.messages.stream.return_value = _FakeStreamCtx()
        result = llm._invoke(
            model="spendguard/claude-3-5-sonnet-latest",
            credentials=_make_credentials("anthropic"),
            prompt_messages=[UserPromptMessage(content="hi")],
            model_parameters={},
            stream=True,
        )
        chunks = list(result)

    rebuilt = "".join(c.delta.message.content for c in chunks if c.delta.message.content)
    assert rebuilt == "Hello from Claude"
    # Final chunk surfaces stop_reason
    finish_chunks = [c for c in chunks if c.delta.finish_reason]
    assert len(finish_chunks) == 1
    assert finish_chunks[0].delta.finish_reason == "end_turn"
    # 6.8: accumulator captured BOTH input_tokens (msg_start) and
    # output_tokens (msg_delta)
    commit_kwargs = client.emit_llm_call_post.await_args.kwargs
    assert commit_kwargs["outcome"] == "SUCCESS"
    assert commit_kwargs["actual_input_tokens"] == 8
    assert commit_kwargs["actual_output_tokens"] == 4
    assert commit_kwargs["estimated_amount_atomic"] == "12"


# ---------------------------------------------------------------------------
# S07 — _StreamingAccumulator unit tests
# ---------------------------------------------------------------------------

def test_S07_accumulator_basics():
    """_StreamingAccumulator: append_text + update_usage + build_llm_result."""
    acc = _StreamingAccumulator()
    acc.append_text("Hello ")
    acc.append_text("world")
    acc.update_usage(prompt_tokens=7, completion_tokens=3)
    acc.provider_event_id = "evt-1"
    result = acc.build_llm_result(model="claude-3-5-sonnet-latest")
    assert result.message.content == "Hello world"
    assert result.usage.prompt_tokens == 7
    assert result.usage.completion_tokens == 3
    assert result.usage.total_tokens == 10
    assert acc.had_usage is True


def test_S07b_accumulator_fallback_only_fires_when_no_usage():
    """fallback_estimate() is a no-op when had_usage is True (real usage
    already captured)."""
    acc = _StreamingAccumulator()
    acc.append_text("some content for chars math")
    acc.update_usage(prompt_tokens=99, completion_tokens=5)  # real usage
    acc.fallback_estimate()  # should NOT override
    assert acc.prompt_tokens == 99
    assert acc.completion_tokens == 5


# ---------------------------------------------------------------------------
# S08 — DENY in Anthropic streaming -> ZERO anthropic.Anthropic call
# ---------------------------------------------------------------------------

def test_S08_anthropic_stream_deny_no_upstream():
    """6.7 mirrored for Anthropic: DENY raises before any upstream."""
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
    with patch("anthropic.Anthropic") as mock_anthropic_cls:
        with pytest.raises(InvokeAuthorizationError, match="dec-deny"):
            result = llm._invoke(
                model="spendguard/claude-3-5-sonnet-latest",
                credentials=_make_credentials("anthropic"),
                prompt_messages=[UserPromptMessage(content="hi")],
                model_parameters={},
                stream=True,
            )
            list(result)
        # INV-1 streaming: Anthropic constructor never called.
        assert mock_anthropic_cls.call_count == 0
