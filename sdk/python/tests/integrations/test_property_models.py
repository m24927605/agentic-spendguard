"""Phase D — property test: 100 model strings dispatch without crashing.

Per SLICE_12 §8.3, the SDK estimator must:
1. Not raise AttributeError or KeyError on any model string
2. Return a valid `EstimatorFns` (callable count_input + count_output)
3. Either dispatch to a known encoder OR fall back to chars/4 with
   a warning (never silently produce 0 tokens)

The model corpus mixes known + unknown + adversarial strings to
exercise the regex first-match-wins ordering and the chars/4 fallback.
"""

from __future__ import annotations

import warnings

import pytest

from spendguard.estimators import estimator_for_model
from spendguard.estimators.dispatch import EncoderKind


# ─────────────────────────────────────────────────────────────────────
# Build a 100+ model string corpus.
# ─────────────────────────────────────────────────────────────────────

# Known OpenAI (covers o200k / cl100k / p50k)
_KNOWN_OPENAI = [
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-4o-2024-05-13",
    "gpt-4o-2024-08-06",
    "gpt-4o-2024-11-20",
    "gpt-4o-mini-2024-07-18",
    "gpt-4",
    "gpt-4-turbo",
    "gpt-4-turbo-preview",
    "gpt-4-turbo-2024-04-09",
    "gpt-4-1106-preview",
    "gpt-4-0125-preview",
    "gpt-4-preview",
    "gpt-3.5-turbo",
    "gpt-3.5-turbo-16k",
    "gpt-3.5-turbo-1106",
    "gpt-3.5-turbo-instruct",
    "gpt-3.5-turbo-instruct-0914",
    "text-davinci-002",
    "text-davinci-003",
    "code-davinci-001",
    "code-davinci-002",
]

# Known Anthropic native
_KNOWN_ANTHROPIC_NATIVE = [
    "claude-3-haiku",
    "claude-3-sonnet",
    "claude-3-opus",
    "claude-3-haiku-20240307",
    "claude-3-sonnet-20240229",
    "claude-3-opus-20240229",
    "claude-3-5-haiku",
    "claude-3-5-sonnet",
    "claude-3-5-opus",
    "claude-3-5-haiku-20241022",
    "claude-3-5-sonnet-20240620",
    "claude-3-5-sonnet-20241022",
]

# Known Anthropic Bedrock (native + cross-region)
_KNOWN_ANTHROPIC_BEDROCK = [
    "anthropic.claude-3-haiku-20240307-v1:0",
    "anthropic.claude-3-sonnet-20240229-v1:0",
    "anthropic.claude-3-opus-20240229-v1:0",
    "anthropic.claude-3-5-sonnet-20240620-v1:0",
    "anthropic.claude-3-5-sonnet-20241022-v2:0",
    "us.anthropic.claude-3-5-sonnet-20240620-v1:0",
    "eu.anthropic.claude-3-haiku-20240307-v1:0",
    "apac.anthropic.claude-3-5-sonnet-20241022-v1:0",
    "us-gov.anthropic.claude-3-5-sonnet-20240620-v1:0",
]

# Known Gemini
_KNOWN_GEMINI = [
    "gemini-1.5-flash",
    "gemini-1.5-pro",
    "gemini-1.5-pro-002",
    "gemini-1.5-flash-002",
    "gemini-2.0-flash",
    "gemini-2.0-flash-exp",
]

# Known Llama Bedrock (warns + chars/4 fallback in SDK)
_KNOWN_LLAMA = [
    "meta.llama3-8b-instruct-v1:0",
    "meta.llama3-70b-instruct-v1:0",
    "meta.llama3-1-8b-instruct-v1:0",
    "meta.llama3-1-70b-instruct-v1:0",
    "meta.llama3-2-1b-instruct-v1:0",
    "meta.llama3-3-70b-instruct-v1:0",
    "us.meta.llama3-1-70b-instruct-v1:0",
    "eu.meta.llama3-2-1b-instruct-v1:0",
]

# Unknown / adversarial — should warn + chars/4 fallback
_UNKNOWN = [
    # Empty / whitespace
    "",
    " ",
    "model",
    # Pre-Claude-3
    "claude-1",
    "claude-2",
    "claude-2.1",
    "anthropic.claude-instant-v1",
    "anthropic.claude-v2",
    "anthropic.claude-v2:1",
    # Pre-Llama-3
    "meta.llama2-7b-chat-v1",
    "meta.llama2-13b-chat-v1",
    "meta.llama2-70b-chat-v1",
    # Other Bedrock vendors
    "amazon.titan-text-v1:0",
    "ai21.j2-mid-v1:0",
    "ai21.jamba-instruct-v1:0",
    "cohere.embed-english-v3",
    "cohere.embed-multilingual-v3",
    # Cohere (SDK doesn't ship)
    "command-r",
    "command-r-plus",
    "command-light",
    "cohere.command-v1:0",
    "cohere.command-r-v1:0",
    # Future / fictional
    "gpt-5",
    "gpt-5-turbo",
    "gpt-6",
    "claude-4",
    "claude-4-opus",
    "gemini-3.0-flash",
    "deepseek-coder-v2",
    "deepseek-r1",
    "mistral-large",
    "mistral-7b-instruct",
    "phi-3-medium",
    # Invalid prefixes
    "US.anthropic.claude-3-haiku-20240307-v1:0",
    "1us.anthropic.claude-3-haiku-20240307-v1:0",
    # Fuzzy / adversarial suffix
    "gpt-4o-mini-foo-bar",
    "gpt-4-bogus",
    "claude-3-5-sonnet-bogus",
    # Adversarial: contains regex specials
    "gpt-4.*",
    "gpt-4|extra",
    "gpt-4\\",
    # Very long strings
    "a" * 200,
    "b" * 500,
]

_ALL_MODELS = (
    _KNOWN_OPENAI
    + _KNOWN_ANTHROPIC_NATIVE
    + _KNOWN_ANTHROPIC_BEDROCK
    + _KNOWN_GEMINI
    + _KNOWN_LLAMA
    + _UNKNOWN
)


class TestPropertyDispatch:
    """Per SLICE_12 §8.3: 100+ model strings must dispatch correctly
    without AttributeError / KeyError / silent zero-token output."""

    @pytest.mark.parametrize("model", _ALL_MODELS)
    def test_estimator_constructible_for_any_model(self, model: str) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model(model)
        # API contract: every estimator has these four attributes
        assert callable(e.count_input_tokens)
        assert callable(e.count_output_tokens_max)
        assert isinstance(e.encoder_name, str)
        assert len(e.encoder_name) > 0
        # kind is Optional[EncoderKind]
        assert e.kind is None or isinstance(e.kind, EncoderKind)

    @pytest.mark.parametrize("model", _ALL_MODELS)
    def test_input_tokens_returns_positive_int(self, model: str) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model(model)
        # Standard chat message format
        messages = [{"role": "user", "content": "Hello, world!"}]
        result = e.count_input_tokens(messages, model)
        assert isinstance(result, int)
        assert result >= 1, f"model {model!r}: expected ≥1 token, got {result}"

    @pytest.mark.parametrize("model", _ALL_MODELS)
    def test_output_tokens_returns_positive_int(self, model: str) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model(model)
        # Honors explicit max_tokens
        assert e.count_output_tokens_max(500, model) == 500
        # None → uses model's default context window (≥ 1)
        none_result = e.count_output_tokens_max(None, model)
        assert isinstance(none_result, int)
        assert none_result >= 1

    @pytest.mark.parametrize("model", _ALL_MODELS)
    def test_empty_messages_returns_at_least_one_token(self, model: str) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model(model)
        result = e.count_input_tokens([], model)
        assert result >= 1, f"model {model!r}: empty input must return ≥1 token"


class TestPropertyEncoderKindAssignment:
    """Known models must dispatch to the right kind; unknown must
    fallback (kind=None)."""

    @pytest.mark.parametrize("model", _KNOWN_OPENAI)
    def test_openai_models_dispatch_to_openai(self, model: str) -> None:
        e = estimator_for_model(model)
        assert e.kind is EncoderKind.OPENAI

    @pytest.mark.parametrize("model", _KNOWN_ANTHROPIC_NATIVE + _KNOWN_ANTHROPIC_BEDROCK)
    def test_anthropic_models_dispatch_to_anthropic(self, model: str) -> None:
        e = estimator_for_model(model)
        assert e.kind is EncoderKind.ANTHROPIC

    @pytest.mark.parametrize("model", _KNOWN_GEMINI)
    def test_gemini_models_dispatch_to_gemini(self, model: str) -> None:
        e = estimator_for_model(model)
        assert e.kind is EncoderKind.GEMINI

    @pytest.mark.parametrize("model", _KNOWN_LLAMA)
    def test_llama_models_fallback_with_warning(self, model: str) -> None:
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            e = estimator_for_model(model)
        # Llama Bedrock dispatches to LLAMA kind in dispatch table but
        # the estimator_for_model() resolver warns + chars/4 because
        # the SDK doesn't vendor the Llama SentencePiece asset.
        assert e.kind is None
        assert any("Llama" in str(rec.message) for rec in w)

    @pytest.mark.parametrize("model", _UNKNOWN)
    def test_unknown_models_fallback_with_warning(self, model: str) -> None:
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            e = estimator_for_model(model)
        assert e.kind is None
        assert any(
            "unknown model" in str(rec.message)
            or "Llama" in str(rec.message)
            for rec in w
        )
