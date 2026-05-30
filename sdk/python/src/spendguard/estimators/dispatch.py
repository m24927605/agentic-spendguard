"""Python mirror of the Rust dispatch table.

Spec ref ``tokenizer-service-spec-v1alpha1.md`` §3.1 + §3.3.

The dispatch table maps a model-string pattern to an encoder kind. The
patterns are anchored regex (``^...$``) per §3.3 — no fuzzy match.
Unknown models fall through to the chars/4 SDK-side heuristic with a
``warnings.warn`` emission (SDK has no Tier 3 metric path; server-side
emits ``tokenizer_unknown_model``).

The table MUST be kept byte-identical with the Rust dispatch table at
``crates/spendguard-tokenizer/src/dispatch.rs``. Ordering is first-match
wins per spec §3.1; more-specific patterns come before broader ones.

SLICE_12 coverage (matches Rust SLICE_03 + SLICE_04 R2):

* OpenAI: gpt-4o (+ -mini, dated), gpt-4 / gpt-4-turbo / gpt-4-NNNN-preview,
  gpt-3.5-turbo (+ dated / -16k), gpt-3.5-turbo-instruct, text-davinci-003,
  code-davinci-002.
* Anthropic native: claude-3-(haiku|sonnet|opus), claude-3-5-(haiku|sonnet|opus)
  (+ optional dated suffix YYYYMMDD).
* Anthropic Bedrock: ``[REGION.]anthropic.claude-3-...-vN:N`` (cross-region
  inference profile prefix per Bedrock 2024-09+; supports us/eu/apac/us-gov/future).
* Gemini native: gemini-1.5-(flash|pro) (+ -NNN revision), gemini-2.0-flash
  (+ -exp).
* Llama Bedrock: ``[REGION.]meta.llama3-N-Mb-instruct-vN:N``.

Out of scope per SLICE_12 plan (no Cohere SDK estimator):

* Cohere ``command-r`` family — Rust crate ships behind ``cohere`` feature
  flag pending legal review. Python SDK does NOT ship a Cohere estimator
  in SLICE_12 (spec §3 + SLICE_04 R2 M6). Model IDs fall to chars/4.
* Llama via local file — Bedrock-routed only; Python SDK does not vendor
  the Llama SentencePiece asset (no first-party Bedrock invocation from
  Python; egress_proxy handles via tokenizer service).

Pattern ordering rule: when adding entries, more-specific patterns
(e.g. ``claude-3-5-``) MUST precede broader ones (``claude-3-``); the
``dispatch()`` loop returns on first match.
"""

from __future__ import annotations

import re
import warnings
from collections.abc import Callable
from enum import Enum
from typing import NamedTuple


class EncoderKind(str, Enum):
    """Identifies which provider family an estimator dispatches to.

    Mirrors the Rust ``crates/spendguard-tokenizer/src/encoders/mod.rs::EncoderKind``
    enum. The string value is the canonical kind name surfacing in the
    ``tokenizer_versions`` audit row (e.g. ``OPENAI_TIKTOKEN``).
    """

    OPENAI = "OPENAI_TIKTOKEN"
    ANTHROPIC = "ANTHROPIC_BPE"
    GEMINI = "GEMINI_BPE"
    # Llama is server-side only in SLICE_12 — Python SDK does not vendor.
    LLAMA = "LLAMA_SENTENCEPIECE"


class TiktokenFamily(str, Enum):
    """Which tiktoken encoder a row resolves to (OpenAI family only)."""

    CL100K_BASE = "cl100k_base"
    O200K_BASE = "o200k_base"
    P50K_BASE = "p50k_base"


class DispatchEntry(NamedTuple):
    """One row in the dispatch table.

    ``pattern_source`` is the raw regex string (kept for error messages
    + ordering inspection in tests). ``pattern`` is the compiled regex.
    ``kind`` is the provider family. ``tiktoken_family`` is set only
    for OpenAI rows so the OpenAI estimator can pick the encoder
    singleton without re-matching.
    """

    pattern_source: str
    pattern: re.Pattern[str]
    kind: EncoderKind
    tiktoken_family: TiktokenFamily | None


# ─────────────────────────────────────────────────────────────────────
# Raw entries — mirror of Rust ``RAW_ENTRIES`` const.
# Ordering: more-specific → broader (first-match-wins per spec §3.1).
# ─────────────────────────────────────────────────────────────────────

_RAW_ENTRIES: list[tuple[str, EncoderKind, TiktokenFamily | None]] = [
    # ── OpenAI o200k_base (latest, narrowest patterns first) ───────────
    (r"^gpt-4o-mini(-\d{4}-\d{2}-\d{2})?$", EncoderKind.OPENAI, TiktokenFamily.O200K_BASE),
    (r"^gpt-4o(-\d{4}-\d{2}-\d{2})?$", EncoderKind.OPENAI, TiktokenFamily.O200K_BASE),
    # ── OpenAI cl100k_base ─────────────────────────────────────────────
    # gpt-4(-NNNN)-preview must precede broader gpt-4 patterns
    # (R2 M1 in Rust crate; same precedence rule here).
    (r"^gpt-4(-\d{4})?-preview$", EncoderKind.OPENAI, TiktokenFamily.CL100K_BASE),
    (r"^gpt-4-turbo(-preview)?(-\d{4}-\d{2}-\d{2})?$", EncoderKind.OPENAI, TiktokenFamily.CL100K_BASE),
    (r"^gpt-4(-\d{4})?(-\d{4}-\d{2}-\d{2})?$", EncoderKind.OPENAI, TiktokenFamily.CL100K_BASE),
    (r"^gpt-3\.5-turbo(-\d{4})?(-\d{2}k)?$", EncoderKind.OPENAI, TiktokenFamily.CL100K_BASE),
    # ── OpenAI p50k_base (legacy completion + instruct) ───────────────
    (r"^gpt-3\.5-turbo-instruct(-\d{4})?$", EncoderKind.OPENAI, TiktokenFamily.P50K_BASE),
    (r"^text-davinci-(002|003)$", EncoderKind.OPENAI, TiktokenFamily.P50K_BASE),
    (r"^code-davinci-(001|002)$", EncoderKind.OPENAI, TiktokenFamily.P50K_BASE),
    # ── Anthropic Claude 3.5 family (must precede 3.x catch-all) ─────
    # Real model IDs:
    #   claude-3-5-sonnet-20240620 / -20241022
    #   claude-3-5-haiku-20241022
    (r"^claude-3-5-(sonnet|haiku|opus)(-\d{8})?$", EncoderKind.ANTHROPIC, None),
    # ── Anthropic Claude 3.x native ───────────────────────────────────
    #   claude-3-opus-20240229, claude-3-sonnet-20240229, claude-3-haiku-20240307
    (r"^claude-3-(haiku|sonnet|opus)(-\d{8})?$", EncoderKind.ANTHROPIC, None),
    # ── Anthropic Bedrock + cross-region prefix ───────────────────────
    # Examples:
    #   anthropic.claude-3-5-sonnet-20240620-v1:0
    #   us.anthropic.claude-3-5-sonnet-20240620-v1:0  (cross-region)
    #   eu.anthropic.claude-3-haiku-20240307-v1:0
    #   us-gov.anthropic.claude-3-5-sonnet-20240620-v1:0
    # The optional ``(?:[a-z][a-z0-9-]*\.)?`` admits any current AND
    # future region prefix (Bedrock 2024-09+).
    (
        r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-5-(sonnet|haiku|opus)(-\d{8})?-v\d+:\d+$",
        EncoderKind.ANTHROPIC,
        None,
    ),
    (
        r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-(haiku|sonnet|opus)(-\d{8})?-v\d+:\d+$",
        EncoderKind.ANTHROPIC,
        None,
    ),
    # ── Gemini native ─────────────────────────────────────────────────
    (r"^gemini-2\.0-flash(-exp)?$", EncoderKind.GEMINI, None),
    (r"^gemini-1\.5-(flash|pro)(-\d{3})?$", EncoderKind.GEMINI, None),
    # ── Llama Bedrock + cross-region prefix (SERVER-SIDE ONLY) ─────────
    # Python SDK does not ship a Llama estimator; these rows are
    # provided so dispatch() returns ``EncoderKind.LLAMA`` and the
    # caller can decide to delegate to the tokenizer service (future
    # SDK gRPC fallback) or fall back to chars/4.
    (
        r"^(?:[a-z][a-z0-9-]*\.)?meta\.llama3(-\d+)?-\d+b-instruct-v\d+:\d+$",
        EncoderKind.LLAMA,
        None,
    ),
]


# Compile once at import time. Pre-compiled regex objects are reused
# for every dispatch call → ``re.compile`` cost amortized to import.
_DISPATCH_TABLE: list[DispatchEntry] = [
    DispatchEntry(
        pattern_source=src,
        pattern=re.compile(src),
        kind=kind,
        tiktoken_family=family,
    )
    for src, kind, family in _RAW_ENTRIES
]


def dispatch_table() -> list[DispatchEntry]:
    """Return the compiled dispatch table.

    Returned list is a fresh shallow copy so callers cannot mutate
    the singleton (e.g. ``.pop()`` in a test would break subsequent
    tests). The underlying ``DispatchEntry`` tuples are immutable.
    """
    return list(_DISPATCH_TABLE)


def lookup(model: str) -> DispatchEntry | None:
    """First-match lookup. Returns ``None`` for unknown models.

    Unknown → caller falls through to chars/4 heuristic + warnings.warn
    (the SDK-side equivalent of the server-side
    ``tokenizer_unknown_model`` metric).
    """
    if not model:
        return None
    for entry in _DISPATCH_TABLE:
        if entry.pattern.match(model):
            return entry
    return None


# ─────────────────────────────────────────────────────────────────────
# Estimator dispatch (Phase A returns callables; Phase B+C wire them
# into the integrations).
# ─────────────────────────────────────────────────────────────────────


class EstimatorFns(NamedTuple):
    """Returned by ``estimator_for_model``.

    Both callables accept generic ``messages: list`` / ``max_tokens:
    int | None`` and return ``int`` (token count). The estimator is
    framework-agnostic — each integration adapts ``messages`` into the
    form the dispatch'd encoder expects.

    The ``encoder_name`` mirrors the Rust ``EncoderResolver::encoder_name``
    string and surfaces in audit context for parity with the server-side
    tokenizer service.

    ``kind`` is the provider family; ``UNKNOWN`` => chars/4 fallback was
    selected (model didn't match any dispatch entry).
    """

    count_input_tokens: Callable[[list, str], int]
    count_output_tokens_max: Callable[[int | None, str], int]
    encoder_name: str
    kind: EncoderKind | None  # None ⇒ unknown / fallback heuristic


# Sentinel encoder name for the chars/4 fallback path.
_FALLBACK_ENCODER_NAME = "chars-per-4-fallback"

# Default context window for unknown models, used when ``max_tokens`` is
# None to bound the Strategy A output projection.
# Per ``run-cost-projector-spec-v1alpha1.md`` §5 conservative default,
# matches Rust ``output_predictor`` cold-start default.
_DEFAULT_CONTEXT_WINDOW = 4096


def estimator_for_model(model: str) -> EstimatorFns:
    """Return token-counting callables for ``model``.

    Algorithm:

    1. First-match lookup via ``lookup(model)``.
    2. Dispatch to the matching encoder's ``count_input_tokens`` /
       ``count_output_tokens_max`` (Phase A: imported lazily so a
       missing optional dep — e.g. ``tiktoken`` for OpenAI — only
       breaks the OpenAI path, not every estimator).
    3. Unknown model → fallback chars/4 estimator + ``warnings.warn``
       (SDK has no Tier 3 metric).

    Lazy imports per estimator: each provider's encoder library is
    optional (declared in ``pyproject.toml`` extras). The lazy import
    only happens when the user actually invokes ``dispatch_to`` for
    that provider — keeping the SDK import-time light when only one
    provider is used.
    """
    entry = lookup(model)
    if entry is None:
        warnings.warn(
            f"spendguard.estimators: unknown model {model!r}; falling "
            f"back to chars/4 heuristic. Add the model to the dispatch "
            f"table or supply an explicit `claim_estimator=`.",
            stacklevel=2,
        )
        return _fallback_estimator()

    if entry.kind is EncoderKind.OPENAI:
        from . import openai as _openai

        assert entry.tiktoken_family is not None
        return EstimatorFns(
            count_input_tokens=_openai.make_count_input_tokens(entry.tiktoken_family),
            count_output_tokens_max=_openai.count_output_tokens_max,
            encoder_name=entry.tiktoken_family.value,
            kind=EncoderKind.OPENAI,
        )

    if entry.kind is EncoderKind.ANTHROPIC:
        from . import anthropic as _anthropic

        return EstimatorFns(
            count_input_tokens=_anthropic.count_input_tokens,
            count_output_tokens_max=_anthropic.count_output_tokens_max,
            encoder_name="anthropic-v3-bpe",
            kind=EncoderKind.ANTHROPIC,
        )

    if entry.kind is EncoderKind.GEMINI:
        from . import gemini as _gemini

        return EstimatorFns(
            count_input_tokens=_gemini.count_input_tokens,
            count_output_tokens_max=_gemini.count_output_tokens_max,
            encoder_name="gemini-1.5-bpe",
            kind=EncoderKind.GEMINI,
        )

    # Llama: Python SDK does not vendor SentencePiece asset (Bedrock-only
    # routing happens server-side via tokenizer service). Fall back to
    # chars/4 with a Llama-specific warning so the operator sees
    # "model recognised but no SDK estimator" rather than total miss.
    if entry.kind is EncoderKind.LLAMA:
        warnings.warn(
            f"spendguard.estimators: model {model!r} dispatches to "
            f"Llama SentencePiece (Bedrock), but the Python SDK does "
            f"NOT vendor the Llama tokenizer asset (server-side only). "
            f"Falling back to chars/4 heuristic for SDK-side estimation. "
            f"For exact counts, route through egress_proxy + tokenizer service.",
            stacklevel=2,
        )
        return _fallback_estimator()

    # Unreachable: every EncoderKind variant is handled above.
    raise RuntimeError(f"unhandled EncoderKind {entry.kind!r}")


# ─────────────────────────────────────────────────────────────────────
# Chars/4 fallback estimator. SDK-only — server-side equivalent is the
# Tier 3 heuristic at ``services/tokenizer/src/tier3.rs``.
# ─────────────────────────────────────────────────────────────────────


def _extract_text_from_messages(messages: list) -> str:
    """Best-effort concatenation of message content to a single string.

    Supports common framework message shapes:

    * ``dict`` with ``"content"`` key (OpenAI chat completions style)
    * Object with ``content`` attribute (LangChain BaseMessage, Pydantic-AI
      ModelMessage)
    * Plain string

    Falls back to ``repr()`` for unknown shapes — this is the
    last-resort heuristic so we keep the count vaguely sensible rather
    than crashing.
    """
    parts: list[str] = []
    for m in messages:
        if m is None:
            continue
        if isinstance(m, str):
            parts.append(m)
            continue
        if isinstance(m, dict):
            c = m.get("content", "")
            if isinstance(c, str):
                parts.append(c)
            elif isinstance(c, list):
                # multi-modal content blocks: extract any "text" fields
                for blk in c:
                    if isinstance(blk, dict) and "text" in blk:
                        parts.append(str(blk["text"]))
                    else:
                        parts.append(repr(blk))
            else:
                parts.append(repr(c))
            continue
        c = getattr(m, "content", None)
        if isinstance(c, str):
            parts.append(c)
        elif c is not None:
            parts.append(repr(c))
        else:
            parts.append(repr(m))
    return "\n".join(parts)


def _chars_per_4_input(messages: list, _model: str) -> int:
    """chars/4 fallback for unknown models.

    Returns at least 1 token so downstream Strategy A doesn't reserve
    zero (which would silently bypass enforcement).
    """
    text = _extract_text_from_messages(messages)
    return max(1, len(text) // 4)


def _chars_per_4_output(max_tokens: int | None, _model: str) -> int:
    """Strategy A formula for unknown models.

    Per ``tokenizer-service-spec-v1alpha1.md`` §3.7:
        reservation = min(max_tokens, context_window - input_tokens) × price
    The SDK estimator returns the ``max_tokens`` cap (capped at
    ``_DEFAULT_CONTEXT_WINDOW`` when ``max_tokens`` is None). The
    sidecar / output_predictor refines via context_window lookup.
    """
    if max_tokens is None or max_tokens <= 0:
        return _DEFAULT_CONTEXT_WINDOW
    return min(max_tokens, _DEFAULT_CONTEXT_WINDOW)


def _fallback_estimator() -> EstimatorFns:
    """Return the chars/4 fallback EstimatorFns tuple."""
    return EstimatorFns(
        count_input_tokens=_chars_per_4_input,
        count_output_tokens_max=_chars_per_4_output,
        encoder_name=_FALLBACK_ENCODER_NAME,
        kind=None,
    )


__all__ = [
    "DispatchEntry",
    "EncoderKind",
    "EstimatorFns",
    "TiktokenFamily",
    "dispatch_table",
    "estimator_for_model",
    "lookup",
]
