"""Unit tests for ``spendguard.estimators.openai``.

Validates the tiktoken-based estimator returns sensible token counts
for the three encoder families. Parity with the Rust crate is verified
via shared golden samples in tests/estimators/test_rust_parity.py.
"""

from __future__ import annotations

import pytest

from spendguard.estimators import estimator_for_model
from spendguard.estimators.dispatch import TiktokenFamily
from spendguard.estimators.openai import (
    count_output_tokens_max,
    make_count_input_tokens,
)


class TestCountInputTokens:
    def test_simple_user_message_o200k(self) -> None:
        # "Hello world" in o200k_base is 2 raw tokens; + 3 priming + 3
        # per-message + 1 role token = 9 (depending on tokenizer
        # version). We assert a stable lower bound.
        count = make_count_input_tokens(TiktokenFamily.O200K_BASE)
        result = count([{"role": "user", "content": "Hello world"}], "gpt-4o")
        assert result >= 5
        assert result <= 15

    def test_empty_messages_returns_min_one(self) -> None:
        count = make_count_input_tokens(TiktokenFamily.O200K_BASE)
        # Empty list returns at least 1 (the priming overhead)
        assert count([], "gpt-4o") >= 1

    def test_multi_message_increases_count(self) -> None:
        count = make_count_input_tokens(TiktokenFamily.CL100K_BASE)
        single = count(
            [{"role": "user", "content": "Hello"}], "gpt-3.5-turbo"
        )
        triple = count(
            [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
            ],
            "gpt-3.5-turbo",
        )
        assert triple > single

    def test_string_message_supported(self) -> None:
        count = make_count_input_tokens(TiktokenFamily.O200K_BASE)
        # Plain string (not dict): should work
        assert count(["Hello world"], "gpt-4o") > 0

    def test_object_with_content_attr(self) -> None:
        class Msg:
            content = "Hello world"

        count = make_count_input_tokens(TiktokenFamily.O200K_BASE)
        assert count([Msg()], "gpt-4o") > 0

    def test_multimodal_content_extracts_text_blocks(self) -> None:
        count = make_count_input_tokens(TiktokenFamily.O200K_BASE)
        msg = {
            "role": "user",
            "content": [
                {"type": "text", "text": "What is in this image?"},
                {"type": "image_url", "image_url": {"url": "..."}},
            ],
        }
        result = count([msg], "gpt-4o")
        # Only text block contributes; image block billed elsewhere
        assert result > 5


class TestCountOutputTokensMax:
    def test_explicit_max_tokens_returned(self) -> None:
        assert count_output_tokens_max(500, "gpt-4o") == 500
        assert count_output_tokens_max(1000, "gpt-3.5-turbo") == 1000

    def test_none_uses_family_context_window_o200k(self) -> None:
        # o200k family default = 16384
        assert count_output_tokens_max(None, "gpt-4o") == 16384
        assert count_output_tokens_max(None, "gpt-4o-mini") == 16384

    def test_none_uses_family_context_window_cl100k(self) -> None:
        # cl100k family default = 4096
        assert count_output_tokens_max(None, "gpt-3.5-turbo") == 4096

    def test_zero_max_tokens_treated_as_none(self) -> None:
        # max_tokens=0 is invalid → fall back to context window
        assert count_output_tokens_max(0, "gpt-4o") == 16384

    def test_unknown_model_uses_cl100k_default(self) -> None:
        # Unknown model: dispatch.lookup returns None → use cl100k 4096
        assert count_output_tokens_max(None, "unknown-model") == 4096


class TestEstimatorIntegration:
    """End-to-end: estimator_for_model → callables → token counts."""

    @pytest.mark.parametrize(
        "model",
        [
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4-turbo",
            "gpt-4",
            "gpt-3.5-turbo",
            "gpt-3.5-turbo-instruct",
            "text-davinci-003",
        ],
    )
    def test_estimator_pipeline_works(self, model: str) -> None:
        e = estimator_for_model(model)
        messages = [{"role": "user", "content": "What is 2+2?"}]
        input_count = e.count_input_tokens(messages, model)
        output_count = e.count_output_tokens_max(100, model)
        assert input_count > 0
        assert output_count == 100
