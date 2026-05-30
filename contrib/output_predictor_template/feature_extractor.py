"""Feature extraction for the Strategy C output predictor plugin template.

Converts a `PredictRequest` (per output-predictor-plugin-contract-v1alpha1.md
§2) into a fixed-shape numpy vector suitable for any sklearn-style
`model.predict(X)` call.

Customer guidance
-----------------

1. The schema this module emits is part of YOUR training pipeline.
   If you change the feature layout below, you MUST retrain the model.
2. The 7 ``prompt_class`` enum values and the model-family taxonomy
   are hard-coded against the SpendGuard contract surface as of
   v1alpha1. Adding a new value SHOULD be additive (append to the
   end of the relevant list); never reshuffle existing positions
   without a retrain + new ``plugin_version`` stamp.
3. ``ContextFeatures`` fields are optional from SpendGuard's side
   (proto3 defaults: 0 / False / ""). Treat missing values as
   "feature unavailable" — do not pretend otherwise.
4. This extractor is deterministic + side-effect-free. Call it from
   the gRPC handler on the hot path; ~10µs per invocation on a
   modern CPU.
"""
from __future__ import annotations

import math
from dataclasses import dataclass
from typing import TYPE_CHECKING

import numpy as np

if TYPE_CHECKING:  # pragma: no cover - typing only
    # Avoid importing generated proto at module load time so callers
    # that only want the extractor utilities (e.g. backtest harness
    # working off CSV) don't need the gRPC stubs on PYTHONPATH.
    from _proto.spendguard.output_predictor_plugin.v1 import plugin_pb2


# ---------------------------------------------------------------------------
# Schema constants
# ---------------------------------------------------------------------------

# Order matters: each index below is fixed for the lifetime of a trained
# model. Append-only evolution.
PROMPT_CLASSES: tuple[str, ...] = (
    "chat_short",
    "chat_long",
    "code_gen",
    "summarization",
    "rag",
    "tool_calling",
    "vision",
)

# Coarse model-family buckets keyed off prefix matching against the
# `model` field. SpendGuard's contract sends the canonical model
# string (e.g. "gpt-4o", "claude-3-5-sonnet-20240620"). Anything not
# matched maps to "other" so the vector shape stays stable.
MODEL_FAMILIES: tuple[str, ...] = (
    "openai_gpt4",
    "openai_gpt35",
    "anthropic_claude3",
    "google_gemini",
    "other",
)

USER_ROLE_HINTS: tuple[str, ...] = ("first", "continuation", "tool_response", "unknown")


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _one_hot(value: str, vocab: tuple[str, ...]) -> np.ndarray:
    """One-hot encode ``value`` against ``vocab``.

    Unknown values map to a zero vector. Callers who want a "default"
    bucket should ensure that bucket is appended to ``vocab`` last
    (e.g. ``"other"`` for ``MODEL_FAMILIES``).
    """
    vec = np.zeros(len(vocab), dtype=np.float32)
    try:
        idx = vocab.index(value)
    except ValueError:
        return vec
    vec[idx] = 1.0
    return vec


def model_family_of(model: str) -> str:
    """Classify a model string into the coarse training-corpus bucket."""
    if not model:
        return "other"
    m = model.lower()
    if m.startswith("gpt-4") or m.startswith("gpt4"):
        return "openai_gpt4"
    if m.startswith("gpt-3.5") or m.startswith("gpt3.5"):
        return "openai_gpt35"
    if m.startswith("claude"):
        return "anthropic_claude3"
    if m.startswith("gemini"):
        return "google_gemini"
    return "other"


def _safe_log1p(value: float) -> float:
    """``log1p`` with a clamp on negative inputs (proto3 defaults to 0)."""
    if value < 0 or math.isnan(value):
        return 0.0
    return float(math.log1p(value))


# ---------------------------------------------------------------------------
# Feature vector
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class FeatureSchema:
    """Description of the feature vector shape.

    Surfaces ``feature_hash`` (a stable identifier of the schema +
    field order) so a customer can correlate plugin audit drift with
    schema bumps. The hash is computed once at import time and never
    changes for a given module version.
    """

    fields: tuple[str, ...]
    feature_hash: str

    @property
    def width(self) -> int:
        return len(self.fields)


def _build_schema() -> FeatureSchema:
    fields: list[str] = []
    fields.extend(f"prompt_class={cls}" for cls in PROMPT_CLASSES)
    fields.extend(f"model_family={fam}" for fam in MODEL_FAMILIES)
    fields.extend(f"user_role_hint={hint}" for hint in USER_ROLE_HINTS)
    fields.extend(
        [
            "log1p_input_tokens",
            "log1p_max_tokens_requested",
            "has_max_tokens_requested",
            "log1p_conversation_depth",
            "has_tool_calls",
            "has_system_message",
            "log1p_num_tool_definitions",
        ]
    )
    # Stable, version-independent hash: hex digest of a delimiter-joined
    # schema. Avoids hashlib dependency on import; the customer's
    # training pipeline should embed this string in model artifacts.
    fields_tuple = tuple(fields)
    import hashlib

    feature_hash = hashlib.sha256("\n".join(fields_tuple).encode("utf-8")).hexdigest()[:16]
    return FeatureSchema(fields=fields_tuple, feature_hash=f"v1a1:{feature_hash}")


SCHEMA: FeatureSchema = _build_schema()


def vectorize(request: "plugin_pb2.PredictRequest") -> np.ndarray:
    """Encode a ``PredictRequest`` as a 1-D feature vector.

    Returns shape ``(SCHEMA.width,)`` ``float32``. The vector is dense;
    sklearn-style ``LinearRegression``, ``GradientBoosting*``, or
    ``HistGradientBoosting*`` all accept this layout directly.
    """
    parts: list[np.ndarray] = []

    # --- categorical: prompt_class -----------------------------------
    parts.append(_one_hot(request.prompt_class, PROMPT_CLASSES))

    # --- categorical: model family -----------------------------------
    parts.append(_one_hot(model_family_of(request.model), MODEL_FAMILIES))

    # --- categorical: user_role_hint ---------------------------------
    role_hint = "unknown"
    if request.HasField("features") and request.features.user_role_hint:
        role_hint = request.features.user_role_hint
    parts.append(_one_hot(role_hint, USER_ROLE_HINTS))

    # --- numeric: token counts ---------------------------------------
    input_tokens = max(0, int(request.input_tokens))
    max_tokens_requested = max(0, int(request.max_tokens_requested))
    parts.append(
        np.asarray(
            [
                _safe_log1p(input_tokens),
                _safe_log1p(max_tokens_requested),
                1.0 if max_tokens_requested > 0 else 0.0,
            ],
            dtype=np.float32,
        )
    )

    # --- context features (proto3 defaults map to "feature absent") --
    conversation_depth = 0
    has_tool_calls = False
    has_system_message = False
    num_tool_definitions = 0
    if request.HasField("features"):
        feats = request.features
        conversation_depth = max(0, int(feats.conversation_depth))
        has_tool_calls = bool(feats.has_tool_calls)
        has_system_message = bool(feats.has_system_message)
        num_tool_definitions = max(0, int(feats.num_tool_definitions))

    parts.append(
        np.asarray(
            [
                _safe_log1p(conversation_depth),
                1.0 if has_tool_calls else 0.0,
                1.0 if has_system_message else 0.0,
                _safe_log1p(num_tool_definitions),
            ],
            dtype=np.float32,
        )
    )

    vector = np.concatenate(parts)
    assert vector.shape == (SCHEMA.width,), (
        f"feature vector shape drift: got {vector.shape}, expected ({SCHEMA.width},). "
        "Check whether someone edited PROMPT_CLASSES / MODEL_FAMILIES / USER_ROLE_HINTS "
        "without retraining the model."
    )
    return vector


def vectorize_dict(record: dict) -> np.ndarray:
    """Vectorize a CSV-row-style dict (used by the backtest harness).

    Accepts the same keys the proto encodes:
      - ``prompt_class``, ``model``, ``user_role_hint`` (strings)
      - ``input_tokens``, ``max_tokens_requested``,
        ``conversation_depth``, ``num_tool_definitions`` (ints)
      - ``has_tool_calls``, ``has_system_message`` (bool-coercible)
    Missing keys are treated as proto3 defaults.
    """
    parts: list[np.ndarray] = []
    parts.append(_one_hot(str(record.get("prompt_class", "")), PROMPT_CLASSES))
    parts.append(_one_hot(model_family_of(str(record.get("model", ""))), MODEL_FAMILIES))
    parts.append(_one_hot(str(record.get("user_role_hint", "unknown")), USER_ROLE_HINTS))

    input_tokens = max(0, int(record.get("input_tokens", 0) or 0))
    max_tokens_requested = max(0, int(record.get("max_tokens_requested", 0) or 0))
    parts.append(
        np.asarray(
            [
                _safe_log1p(input_tokens),
                _safe_log1p(max_tokens_requested),
                1.0 if max_tokens_requested > 0 else 0.0,
            ],
            dtype=np.float32,
        )
    )

    conversation_depth = max(0, int(record.get("conversation_depth", 0) or 0))
    num_tool_definitions = max(0, int(record.get("num_tool_definitions", 0) or 0))

    def _truthy(v: object) -> bool:
        if isinstance(v, bool):
            return v
        if isinstance(v, (int, float)):
            return v != 0
        if isinstance(v, str):
            return v.strip().lower() in {"1", "true", "yes", "y", "t"}
        return bool(v)

    parts.append(
        np.asarray(
            [
                _safe_log1p(conversation_depth),
                1.0 if _truthy(record.get("has_tool_calls", False)) else 0.0,
                1.0 if _truthy(record.get("has_system_message", False)) else 0.0,
                _safe_log1p(num_tool_definitions),
            ],
            dtype=np.float32,
        )
    )
    return np.concatenate(parts)
