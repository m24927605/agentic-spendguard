"""OpenAI token estimator using the ``tiktoken`` library.

Spec ref ``tokenizer-service-spec-v1alpha1.md`` §3.1 (dispatch table).
Mirrors the Rust ``crates/spendguard-tokenizer`` OpenAI path
(SLICE_03), using the same three encoder families:

* ``o200k_base`` — gpt-4o, gpt-4o-mini (+ dated variants)
* ``cl100k_base`` — gpt-4, gpt-4-turbo, gpt-4-NNNN-preview, gpt-3.5-turbo
* ``p50k_base`` — gpt-3.5-turbo-instruct, text-davinci-002/003, code-davinci

The chat-style overhead (per-message + per-role tokens that OpenAI
applies on top of raw content tokenisation) is hardcoded from the
OpenAI cookbook ``num_tokens_from_messages`` reference implementation:
3 tokens for the assistant priming + 4 per message (3 for ``role`` /
``content`` / ``name`` boundaries + 1 for the message separator).
"""

from __future__ import annotations

from collections.abc import Callable
from typing import TYPE_CHECKING

from .dispatch import TiktokenFamily

if TYPE_CHECKING:
    import tiktoken


# Per-message overhead per OpenAI cookbook num_tokens_from_messages:
# https://github.com/openai/openai-cookbook/blob/main/examples/How_to_count_tokens_with_tiktoken.ipynb
#
# These constants are encoder-family dependent; the cookbook reference
# uses 3/1 for cl100k_base and 4/1 for older models. We use the
# conservative cl100k_base value (3/1) for all OpenAI rows — slight
# over-count for o200k_base (~2-3 tokens per message) is acceptable
# because Strategy A under-counting is the real risk (spec §1.1).
_PER_MESSAGE_OVERHEAD = 3
_PER_NAME_OVERHEAD = 1
_REPLY_PRIMING_OVERHEAD = 3  # tokens reserved for "assistant\n" priming

# Per-call ``max_tokens`` default when the caller doesn't pass one.
# OpenAI's default is "leave the choice to the model" which makes
# Strategy A reservation unbounded; we cap at the model's known
# context window as a conservative floor.
_DEFAULT_CONTEXT_WINDOW = {
    TiktokenFamily.O200K_BASE: 16384,
    TiktokenFamily.CL100K_BASE: 4096,
    TiktokenFamily.P50K_BASE: 4096,
}


def _encoding(family: TiktokenFamily) -> "tiktoken.Encoding":
    """Return the cached tiktoken Encoding for ``family``.

    Lazy import so users that don't touch the OpenAI path don't pay
    the ``tiktoken`` import cost (the library loads its BPE merges on
    first ``encoding_for_model`` call).
    """
    try:
        import tiktoken
    except ImportError as exc:  # pragma: no cover — covered by missing-extra test
        raise RuntimeError(
            "spendguard.estimators.openai requires the `tiktoken` "
            "package. Install via `pip install 'spendguard-sdk[openai]'` "
            "or `pip install tiktoken`."
        ) from exc
    return tiktoken.get_encoding(family.value)


def _encode_message_to_token_count(
    encoding: "tiktoken.Encoding", message: object
) -> int:
    """Count tokens for one message, honoring role / name overheads.

    Supports two shapes:

    * Dict with ``"role"`` / ``"content"`` / ``"name"`` keys (OpenAI
      chat completions input format)
    * Plain string (treated as a user message)

    For multi-modal ``content`` (list of blocks), only text blocks are
    counted toward tokenisation; image / audio blocks are billed via
    a separate output_predictor pathway (out of SLICE_12 scope).
    """
    if isinstance(message, str):
        return _PER_MESSAGE_OVERHEAD + len(encoding.encode(message))

    if isinstance(message, dict):
        total = _PER_MESSAGE_OVERHEAD
        content = message.get("content", "")
        if isinstance(content, str):
            total += len(encoding.encode(content))
        elif isinstance(content, list):
            for blk in content:
                if isinstance(blk, dict) and "text" in blk:
                    total += len(encoding.encode(str(blk["text"])))
                # Non-text blocks (image_url, etc) don't add tokens
                # via tiktoken — those are billed downstream.
        else:
            total += len(encoding.encode(str(content)))

        if "role" in message:
            total += len(encoding.encode(str(message["role"])))
        if "name" in message:
            total += _PER_NAME_OVERHEAD + len(encoding.encode(str(message["name"])))
        return total

    # Duck-typed framework message objects: try .content attribute.
    content_attr = getattr(message, "content", None)
    if isinstance(content_attr, str):
        return _PER_MESSAGE_OVERHEAD + len(encoding.encode(content_attr))
    if content_attr is not None:
        return _PER_MESSAGE_OVERHEAD + len(encoding.encode(repr(content_attr)))

    # Unknown shape — use repr() as last-resort fallback so we don't
    # silently undercount.
    return _PER_MESSAGE_OVERHEAD + len(encoding.encode(repr(message)))


def make_count_input_tokens(
    family: TiktokenFamily,
) -> Callable[[list, str], int]:
    """Build a ``count_input_tokens(messages, model)`` callable for ``family``.

    The closure captures ``family`` so the dispatch returns a function
    that doesn't have to re-look up the encoder per call. The
    ``tiktoken.Encoding`` is loaded lazily on first call (re-used
    thereafter via tiktoken's internal LRU).
    """

    def count_input_tokens(messages: list, _model: str) -> int:
        encoding = _encoding(family)
        total = _REPLY_PRIMING_OVERHEAD  # assistant priming
        for msg in messages or []:
            total += _encode_message_to_token_count(encoding, msg)
        # Floor at 1 token so downstream Strategy A doesn't reserve 0.
        return max(1, total)

    return count_input_tokens


def count_output_tokens_max(max_tokens: int | None, model: str) -> int:
    """Strategy A formula for OpenAI models.

    Per ``tokenizer-service-spec-v1alpha1.md`` §3.7:

        reservation = min(max_tokens, context_window - input_tokens) × price

    The SDK estimator returns the ``max_tokens`` cap (or the
    family's default context window when ``max_tokens`` is None).
    The full ``context_window - input_tokens`` arithmetic happens
    server-side at the output_predictor.

    The ``model`` argument lets the function pick a family-specific
    context window for the None case; for known models we cap at the
    family default (o200k 16K, cl100k/p50k 4K). Unknown gpt-3.5 / gpt-4
    variants land here too (dispatched via dispatch.py); we use the
    cl100k_base default 4K as a conservative floor.
    """
    # Re-dispatch is cheap (regex first-match) and lets us pick the
    # right context-window default without threading the family through.
    from .dispatch import lookup

    if max_tokens is not None and max_tokens > 0:
        return max_tokens

    entry = lookup(model)
    if entry is not None and entry.tiktoken_family is not None:
        return _DEFAULT_CONTEXT_WINDOW[entry.tiktoken_family]
    return _DEFAULT_CONTEXT_WINDOW[TiktokenFamily.CL100K_BASE]


__all__ = [
    "count_output_tokens_max",
    "make_count_input_tokens",
]
