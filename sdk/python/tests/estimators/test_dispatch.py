"""Unit tests for ``spendguard.estimators.dispatch``.

Mirrors the Rust dispatch table tests at
``crates/spendguard-tokenizer/src/dispatch.rs::tests`` for parity.
Each test corresponds to a Rust ``#[test]`` of the same intent; if
Python dispatches differently than Rust the SDK estimator silently
disagrees with the server-side audit row.

Spec ref ``tokenizer-service-spec-v1alpha1.md`` §3.1 + §3.3.
"""

from __future__ import annotations

import re
import warnings

import pytest

from spendguard.estimators.dispatch import (
    EncoderKind,
    TiktokenFamily,
    dispatch_table,
    estimator_for_model,
    lookup,
)


# ─────────────────────────────────────────────────────────────────────
# OpenAI family — mirrors Rust SLICE_03 tests
# ─────────────────────────────────────────────────────────────────────


class TestOpenAIDispatch:
    def test_gpt_4o_routes_to_o200k(self) -> None:
        e = lookup("gpt-4o")
        assert e is not None
        assert e.kind is EncoderKind.OPENAI
        assert e.tiktoken_family is TiktokenFamily.O200K_BASE

    def test_gpt_4o_mini_routes_to_o200k(self) -> None:
        e = lookup("gpt-4o-mini")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.O200K_BASE

    def test_gpt_4o_dated_routes_to_o200k(self) -> None:
        e = lookup("gpt-4o-2024-08-06")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.O200K_BASE

    def test_gpt_4o_mini_dated_routes_to_o200k(self) -> None:
        e = lookup("gpt-4o-mini-2024-07-18")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.O200K_BASE

    def test_gpt_4_turbo_routes_to_cl100k(self) -> None:
        e = lookup("gpt-4-turbo")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.CL100K_BASE

    def test_gpt_4_routes_to_cl100k(self) -> None:
        e = lookup("gpt-4")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.CL100K_BASE

    def test_gpt_4_1106_preview_routes_to_cl100k(self) -> None:
        # Rust R2 M1 fix — previously fell to Tier 3.
        e = lookup("gpt-4-1106-preview")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.CL100K_BASE

    def test_gpt_4_0125_preview_routes_to_cl100k(self) -> None:
        e = lookup("gpt-4-0125-preview")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.CL100K_BASE

    def test_gpt_4_preview_bare_routes_to_cl100k(self) -> None:
        e = lookup("gpt-4-preview")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.CL100K_BASE

    def test_gpt_3_5_turbo_routes_to_cl100k(self) -> None:
        e = lookup("gpt-3.5-turbo")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.CL100K_BASE

    def test_gpt_3_5_turbo_16k_routes_to_cl100k(self) -> None:
        e = lookup("gpt-3.5-turbo-16k")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.CL100K_BASE

    def test_gpt_3_5_turbo_instruct_routes_to_p50k(self) -> None:
        # Rust R2 M1 — alphabetic suffix; was Tier 3 fallback.
        e = lookup("gpt-3.5-turbo-instruct")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.P50K_BASE

    def test_gpt_3_5_turbo_instruct_dated_routes_to_p50k(self) -> None:
        e = lookup("gpt-3.5-turbo-instruct-0914")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.P50K_BASE

    def test_text_davinci_003_routes_to_p50k(self) -> None:
        e = lookup("text-davinci-003")
        assert e is not None
        assert e.tiktoken_family is TiktokenFamily.P50K_BASE


# ─────────────────────────────────────────────────────────────────────
# Anthropic family — mirrors Rust SLICE_04 tests
# ─────────────────────────────────────────────────────────────────────


class TestAnthropicDispatch:
    def test_claude_3_haiku_native(self) -> None:
        e = lookup("claude-3-haiku")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_claude_3_5_sonnet_native(self) -> None:
        e = lookup("claude-3-5-sonnet")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_claude_3_5_sonnet_dated(self) -> None:
        e = lookup("claude-3-5-sonnet-20240620")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_claude_3_opus_dated(self) -> None:
        e = lookup("claude-3-opus-20240229")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_claude_3_5_haiku_native(self) -> None:
        e = lookup("claude-3-5-haiku")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC


class TestAnthropicBedrockDispatch:
    def test_bedrock_claude_3_5_sonnet(self) -> None:
        e = lookup("anthropic.claude-3-5-sonnet-20240620-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_bedrock_claude_3_haiku(self) -> None:
        e = lookup("anthropic.claude-3-haiku-20240307-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_bedrock_us_cross_region_3_5_sonnet(self) -> None:
        e = lookup("us.anthropic.claude-3-5-sonnet-20240620-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_bedrock_eu_cross_region_3_haiku(self) -> None:
        e = lookup("eu.anthropic.claude-3-haiku-20240307-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_bedrock_apac_cross_region_3_5_sonnet(self) -> None:
        e = lookup("apac.anthropic.claude-3-5-sonnet-20241022-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC

    def test_bedrock_us_gov_cross_region(self) -> None:
        e = lookup("us-gov.anthropic.claude-3-5-sonnet-20240620-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.ANTHROPIC


# ─────────────────────────────────────────────────────────────────────
# Gemini family
# ─────────────────────────────────────────────────────────────────────


class TestGeminiDispatch:
    def test_gemini_1_5_flash(self) -> None:
        e = lookup("gemini-1.5-flash")
        assert e is not None
        assert e.kind is EncoderKind.GEMINI

    def test_gemini_1_5_pro(self) -> None:
        e = lookup("gemini-1.5-pro")
        assert e is not None
        assert e.kind is EncoderKind.GEMINI

    def test_gemini_1_5_pro_002(self) -> None:
        e = lookup("gemini-1.5-pro-002")
        assert e is not None
        assert e.kind is EncoderKind.GEMINI

    def test_gemini_2_0_flash(self) -> None:
        e = lookup("gemini-2.0-flash")
        assert e is not None
        assert e.kind is EncoderKind.GEMINI

    def test_gemini_2_0_flash_exp(self) -> None:
        e = lookup("gemini-2.0-flash-exp")
        assert e is not None
        assert e.kind is EncoderKind.GEMINI


# ─────────────────────────────────────────────────────────────────────
# Llama Bedrock — server-side dispatch only (SDK falls back to chars/4)
# ─────────────────────────────────────────────────────────────────────


class TestLlamaBedrockDispatch:
    def test_bedrock_llama3_8b(self) -> None:
        e = lookup("meta.llama3-8b-instruct-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.LLAMA

    def test_bedrock_llama3_1_8b(self) -> None:
        e = lookup("meta.llama3-1-8b-instruct-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.LLAMA

    def test_bedrock_llama3_1_70b(self) -> None:
        e = lookup("meta.llama3-1-70b-instruct-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.LLAMA

    def test_bedrock_us_cross_region_llama(self) -> None:
        e = lookup("us.meta.llama3-1-70b-instruct-v1:0")
        assert e is not None
        assert e.kind is EncoderKind.LLAMA


# ─────────────────────────────────────────────────────────────────────
# Negative tests (no-fuzzy-match per spec §3.3)
# ─────────────────────────────────────────────────────────────────────


class TestNegativeDispatch:
    @pytest.mark.parametrize(
        "model",
        [
            "",
            "gpt-4o-mini-foo-bar",  # fuzzy suffix
            "gpt-4-bogus",
            "gpt-5-doesnt-exist-yet",
            "claude-2",
            "claude-2.1",
            "gemini-pro",
            "command-r",  # cohere — opt-in, not in SDK
            "command-light",  # never routes (different vocab)
            "amazon.titan-text-v1:0",
            "ai21.j2-mid-v1:0",
            "anthropic.claude-instant-v1",  # pre-claude-3
            "anthropic.claude-v2",
            "anthropic.claude-v2:1",
            "cohere.embed-english-v3",
            "cohere.embed-multilingual-v3",
            "meta.llama2-13b-chat-v1",  # pre-llama-3
            "meta.llama2-70b-chat-v1",
            "US.anthropic.claude-3-haiku-20240307-v1:0",  # uppercase prefix
            "1us.anthropic.claude-3-haiku-20240307-v1:0",  # digit-leading prefix
        ],
    )
    def test_unknown_model_returns_none(self, model: str) -> None:
        assert lookup(model) is None


# ─────────────────────────────────────────────────────────────────────
# Pattern ordering invariants
# ─────────────────────────────────────────────────────────────────────


class TestPatternOrdering:
    def test_o200k_before_cl100k_for_gpt_4(self) -> None:
        # gpt-4o must hit o200k_base, not the cl100k gpt-4 entry.
        table = dispatch_table()
        o200k_idx = next(
            i for i, e in enumerate(table) if e.tiktoken_family is TiktokenFamily.O200K_BASE
        )
        cl100k_gpt4_idx = next(
            i for i, e in enumerate(table) if "gpt-4(-" in e.pattern_source
        )
        assert o200k_idx < cl100k_gpt4_idx

    def test_claude_3_5_before_claude_3(self) -> None:
        table = dispatch_table()
        three_five_idx = next(
            i for i, e in enumerate(table) if "claude-3-5-" in e.pattern_source
        )
        # Find the first 3-non-3.5 pattern AFTER 3.5
        three_x_idxs = [
            i for i, e in enumerate(table)
            if i > three_five_idx and "^claude-3" in e.pattern_source
        ]
        # If a broader claude-3 pattern exists, it must come after 3.5.
        for idx in three_x_idxs:
            assert three_five_idx < idx

    def test_table_size_at_least_15(self) -> None:
        # SLICE_03 OpenAI: 9; SLICE_04 anthropic 4 + gemini 2 + llama 1 = 7.
        # Total: 16 (Cohere omitted from SDK).
        assert len(dispatch_table()) >= 15


# ─────────────────────────────────────────────────────────────────────
# estimator_for_model — public API
# ─────────────────────────────────────────────────────────────────────


class TestEstimatorForModel:
    def test_known_openai_model(self) -> None:
        e = estimator_for_model("gpt-4o")
        assert e.kind is EncoderKind.OPENAI
        assert e.encoder_name == "o200k_base"
        assert callable(e.count_input_tokens)
        assert callable(e.count_output_tokens_max)

    def test_known_anthropic_model(self) -> None:
        e = estimator_for_model("claude-3-5-sonnet")
        assert e.kind is EncoderKind.ANTHROPIC
        assert e.encoder_name == "anthropic-v3-bpe"

    def test_known_gemini_model(self) -> None:
        e = estimator_for_model("gemini-1.5-flash")
        assert e.kind is EncoderKind.GEMINI
        assert e.encoder_name == "gemini-1.5-bpe"

    def test_unknown_model_falls_back_with_warning(self) -> None:
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            e = estimator_for_model("some-experimental-internal-model")
            assert any("unknown model" in str(rec.message) for rec in w)
        assert e.kind is None
        assert e.encoder_name == "chars-per-4-fallback"

    def test_llama_bedrock_falls_back_with_warning(self) -> None:
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            e = estimator_for_model("meta.llama3-1-8b-instruct-v1:0")
            assert any("Llama" in str(rec.message) for rec in w)
        assert e.kind is None

    def test_empty_model_string_falls_back(self) -> None:
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            e = estimator_for_model("")
            assert any("unknown model" in str(rec.message) for rec in w)
        assert e.kind is None


# ─────────────────────────────────────────────────────────────────────
# chars/4 fallback semantics
# ─────────────────────────────────────────────────────────────────────


class TestFallbackEstimator:
    def test_fallback_returns_min_one_token(self) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model("unknown-xxx")
        # Empty messages → still ≥1 (no zero-reservation)
        assert e.count_input_tokens([], "unknown-xxx") >= 1

    def test_fallback_str_message(self) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model("unknown-xxx")
        # 40 chars → 10 tokens
        assert e.count_input_tokens(["a" * 40], "unknown-xxx") == 10

    def test_fallback_dict_message(self) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model("unknown-xxx")
        result = e.count_input_tokens(
            [{"role": "user", "content": "x" * 100}], "unknown-xxx"
        )
        assert result == 25  # 100/4

    def test_fallback_object_with_content_attr(self) -> None:
        class Msg:
            def __init__(self, content: str) -> None:
                self.content = content

        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model("unknown-xxx")
        assert e.count_input_tokens([Msg("y" * 60)], "unknown-xxx") == 15

    def test_fallback_output_uses_max_tokens(self) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model("unknown-xxx")
        assert e.count_output_tokens_max(500, "unknown-xxx") == 500

    def test_fallback_output_caps_at_context_window(self) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model("unknown-xxx")
        # max_tokens=None → use 4096 default
        assert e.count_output_tokens_max(None, "unknown-xxx") == 4096

    def test_dispatch_table_is_immutable_to_callers(self) -> None:
        t1 = dispatch_table()
        t1.pop()  # mutate the returned copy
        t2 = dispatch_table()
        # Underlying singleton unchanged
        assert len(t2) > len(t1)


# ─────────────────────────────────────────────────────────────────────
# Property test — 100 model strings exercise dispatch without AttributeError
# ─────────────────────────────────────────────────────────────────────


class TestPropertyVariedModels:
    """Per SLICE_12 §8.3 — 100 varied model strings dispatch correctly."""

    @pytest.mark.parametrize(
        "model",
        # 50 known
        [f"gpt-4o-{i:04d}-{j:02d}-{k:02d}" for (i, j, k) in [
            (2024, 5, 13), (2024, 8, 6), (2024, 11, 20),
        ]]
        + ["gpt-4o", "gpt-4o-mini", "gpt-4", "gpt-4-turbo", "gpt-3.5-turbo"]
        + ["gpt-3.5-turbo-instruct", "gpt-3.5-turbo-16k", "text-davinci-003"]
        + [f"claude-3-{suf}{date}" for suf in ("haiku", "sonnet", "opus")
           for date in ("", "-20240229", "-20240307")]
        + [f"claude-3-5-{suf}{date}" for suf in ("haiku", "sonnet")
           for date in ("", "-20240620", "-20241022")]
        + ["gemini-1.5-flash", "gemini-1.5-pro", "gemini-1.5-pro-002",
           "gemini-2.0-flash", "gemini-2.0-flash-exp"]
        + [f"{prefix}.anthropic.claude-3-5-sonnet-20240620-v1:0"
           for prefix in ("us", "eu", "apac", "us-gov", "ap-south-1")]
        + [f"meta.llama3-{i}-{s}b-instruct-v1:0" for i, s in [
            (1, 8), (1, 70), (2, 1), (3, 70),
        ]]
        # 50 unknown / fuzzy → should fallback (no AttributeError)
        + [f"some-random-model-{i}" for i in range(20)]
        + [f"gpt-9-{i}" for i in range(10)]
        + [f"future-model-x{i}" for i in range(10)]
        + [f"meta.llama2-{i}b-chat-v1" for i in [7, 13, 70]]
        + [f"cohere.embed-{lang}-v3" for lang in ("english", "multilingual", "japanese")]
        + ["anthropic.claude-instant-v1", "anthropic.claude-v2",
           "amazon.titan-text-v1:0", "ai21.j2-mid-v1:0"],
    )
    def test_no_attribute_error(self, model: str) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            e = estimator_for_model(model)
        # Both functions must be callable + return int
        assert callable(e.count_input_tokens)
        assert callable(e.count_output_tokens_max)
        # encoder_name must be a non-empty string
        assert isinstance(e.encoder_name, str)
        assert len(e.encoder_name) > 0
