"""Cross-language fixture harness — Python side (SLICE 9 / COV_S05_09).

P0 invariant — review-standards §2.1 / §2.2 / §2.5 + design.md §11: the
Python implementations of ``derive_idempotency_key``, ``compute_prompt_hash``,
and ``derive_uuid_from_signature`` MUST produce byte-identical output to the
TS implementations for every fixture in
``sdk/fixtures/cross-language/v1.json``.

The fixture file is the SINGLE SOURCE OF TRUTH for cross-language parity.
It is generated against the Python reference implementation
(``sdk/fixtures/cross-language/generate.py``) and is consumed UNCHANGED by
both the TS suite (``sdk/typescript/tests/crossLanguage.test.ts``) and this
Python suite.

Why test Python against fixtures it generated? Two reasons:
1. Catches accidental edits to v1.json that drift from the live Python
   implementation. The hand-edit case is exactly what audit-chain
   immutability forbids; this harness detects it deterministically.
2. Provides a regression gate. If a future patch to ``ids.py`` /
   ``prompt_hash.py`` changes a function's output for the same input, this
   suite fails loudly. The fix is NEVER to edit v1.json — it is to mint
   v2.json (see README.md) and migrate audit-row consumers across the
   compat window.

Spec refs:
- docs/specs/coverage/D05_ts_sdk_substrate/design.md §11.
- docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md §2.5
  (shared fixture file consumed by Python + Rust + TS suites).
- docs/internal/slices/COV_S05_09_d05_cross_language_fixtures.md.
"""

from __future__ import annotations

import json
import pathlib
from typing import Any

import pytest

from spendguard.ids import derive_idempotency_key, derive_uuid_from_signature
from spendguard.prompt_hash import compute as compute_prompt_hash

# `sdk/python/tests/test_cross_language_fixtures.py` → `sdk/fixtures/...`
# is three parents up + into fixtures/. Use a fixed-offset path so the suite
# runs whether pytest is invoked from the repo root or from `sdk/python/`.
_HERE = pathlib.Path(__file__).resolve().parent
FIXTURES_PATH = (
    _HERE.parent.parent / "fixtures" / "cross-language" / "v1.json"
)


def _load_corpus() -> dict[str, Any]:
    return json.loads(FIXTURES_PATH.read_text(encoding="utf-8"))


CORPUS = _load_corpus()
FIXTURES: list[dict[str, Any]] = CORPUS["fixtures"]


def _evaluate(fixture: dict[str, Any]) -> str:
    """Dispatch a fixture to its function and return the actual output."""
    fn = fixture["fn"]
    inputs = fixture["inputs"]
    if fn == "derive_idempotency_key":
        return derive_idempotency_key(**inputs)
    if fn == "compute_prompt_hash":
        return compute_prompt_hash(inputs["prompt_text"], inputs["tenant_id"])
    if fn == "derive_uuid_from_signature":
        # Python uses kw-only `scope`; `signature` is positional. Unpack the
        # JSON map explicitly.
        return str(
            derive_uuid_from_signature(
                inputs["signature"], scope=inputs["scope"]
            )
        )
    # Unknown fn means v1.json grew a function the Python harness doesn't
    # dispatch yet. Fail loudly so the fixture isn't silently skipped.
    raise AssertionError(
        f"Unknown cross-language fixture fn for {fixture['id']}: {fn!r}. "
        "Update the Python harness to dispatch this fn or revert the "
        "v1.json change."
    )


# ---------------------------------------------------------------------------
# Corpus shape gates
# ---------------------------------------------------------------------------


def test_corpus_version_is_v1() -> None:
    """v1.json MUST report `version: 1`. A future v2.json lives at a
    different filename + bumps this number."""
    assert CORPUS["version"] == 1


def test_corpus_volume_floor() -> None:
    """COV_S05_09 requires ≥20 fixtures across the three locked
    functions."""
    assert len(FIXTURES) >= 20


def test_corpus_covers_all_three_locked_functions() -> None:
    fns = {f["fn"] for f in FIXTURES}
    assert "derive_idempotency_key" in fns
    assert "compute_prompt_hash" in fns
    assert "derive_uuid_from_signature" in fns


def test_corpus_fixture_ids_are_unique() -> None:
    ids = [f["id"] for f in FIXTURES]
    assert len(set(ids)) == len(ids), (
        f"Duplicate fixture ids in v1.json: {ids}"
    )


# ---------------------------------------------------------------------------
# Per-fixture byte-equivalence sweep
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "fixture",
    FIXTURES,
    ids=[f["id"] for f in FIXTURES],
)
def test_python_matches_pinned_output(fixture: dict[str, Any]) -> None:
    """Every fixture's Python output equals its pinned ``expected_output``.

    If this fails, EITHER the Python implementation drifted (a regression
    that breaks audit-chain rule dedup, P0 blocker), OR v1.json was
    hand-edited (an audit-chain immutability violation per README.md).
    Both are blockers — do NOT "fix" by rewriting v1.json. Mint v2.json or
    revert the implementation change.
    """
    actual = _evaluate(fixture)
    expected = fixture["expected_output"]
    if actual != expected:
        # Surface the exact mismatched vector for triage.
        pytest.fail(
            f"CROSS-LANGUAGE DRIFT for fixture {fixture['id']} "
            f"({fixture['fn']}):\n"
            f"  inputs:   {json.dumps(fixture['inputs'], ensure_ascii=False)}\n"
            f"  expected: {expected}\n"
            f"  actual:   {actual}\n"
            "Python implementation has diverged from v1.json. This is a P0 "
            "review-standards §2 blocker — drift here breaks audit-chain "
            "rule dedup and idempotency replay collapse."
        )
    assert actual == expected


# ---------------------------------------------------------------------------
# Canonicalisation invariants (cross-checks the fixture-encoded contracts)
# ---------------------------------------------------------------------------


def _find(fixture_id: str) -> dict[str, Any]:
    for fx in FIXTURES:
        if fx["id"] == fixture_id:
            return fx
    raise AssertionError(f"Fixture {fixture_id} missing from v1.json")


def test_fxp8_mixed_case_uuid_equals_lowercase() -> None:
    """FXP8 encodes the mixed-case UUID canonicalisation contract: the
    hash for ``ABCDEF12-...`` MUST equal the hash for ``abcdef12-...``.
    Independent recompute pinned for defence-in-depth."""
    fxp8 = _find("FXP8")
    tenant = fxp8["inputs"]["tenant_id"]
    prompt = fxp8["inputs"]["prompt_text"]
    lowered = compute_prompt_hash(prompt, tenant.lower())
    assert lowered == fxp8["expected_output"]


def test_fx5_all_empty_derive_idempotency_key_is_repeatable() -> None:
    """FX5 (all empty strings) is the degraded-but-stable contract. Two
    independent calls produce the same output, equal to the fixture."""
    fx5 = _find("FX5")
    a = _evaluate(fx5)
    b = _evaluate(fx5)
    assert a == b
    assert a == fx5["expected_output"]
