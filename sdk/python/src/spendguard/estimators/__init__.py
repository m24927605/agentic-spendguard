"""SpendGuard SDK default token estimators.

Public surface (import from ``spendguard.estimators``):

* :func:`estimator_for_model` — pick the right estimator for a model
  string (first-match dispatch).
* :func:`lookup` — return the underlying dispatch entry (encoder kind +
  tiktoken family + compiled regex).
* :class:`EncoderKind` — enum of provider families (mirrors Rust).
* :class:`EstimatorFns` — NamedTuple bundling ``count_input_tokens`` +
  ``count_output_tokens_max`` + ``encoder_name`` + ``kind``.

Per-provider modules (lazy-imported by ``estimator_for_model``):

* :mod:`spendguard.estimators.openai` — tiktoken-based (cl100k / o200k / p50k).
* :mod:`spendguard.estimators.anthropic` — vendored Claude 3 BPE.
* :mod:`spendguard.estimators.gemini` — vendored Gemma approximation.

Spec ref ``tokenizer-service-spec-v1alpha1.md`` §3 + §3.1; SLICE_12
SDK-side mirror of the Rust ``crates/spendguard-tokenizer`` dispatch
table. See ``docs/slices/SLICE_12_sdk_default_estimators.md``.
"""

from .dispatch import (
    DispatchEntry,
    EncoderKind,
    EstimatorFns,
    TiktokenFamily,
    dispatch_table,
    estimator_for_model,
    lookup,
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
