# ruff: noqa: S101
"""TA-27 (P0) — cross-language byte-equivalence against the FROZEN corpus
``sdk/fixtures/cross-language/ag_ui_v1.json`` (mirror twin of TP-27).

The corpus was minted in slice COV_D39_01 by the TS reference generator
and is FROZEN (D05 corpus discipline: never edit in place; new vectors →
``ag_ui_v2.json``). This suite CONSUMES it byte-for-byte: for every
vector, the Python builders + ``canonical_event_json`` must reproduce
``expected_canonical_json`` exactly, and ``encode_sse`` must reproduce
``expected_sse`` exactly. Python == corpus ⇒ Python == TS.

If any vector "needs" an edit, that means slice 1 was wrong and slice 1
goes back to review — the corpus is NEVER edited here (tests.md §10).
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from spendguard.integrations.ag_ui import (
    BudgetSnapshotInput,
    DecisionDeniedInput,
    ReservationCommittedInput,
    ReservationCreatedInput,
    ReservationReleasedInput,
    build_budget_snapshot,
    build_decision_denied,
    build_reservation_committed,
    build_reservation_created,
    build_reservation_released,
    canonical_event_json,
    encode_sse,
)

_CORPUS_PATH = (
    Path(__file__).resolve().parents[3].parent
    / "fixtures" / "cross-language" / "ag_ui_v1.json"
)

# Corpus `builder` field uses the TS reference names.
_BUILDER_MAP: dict[str, tuple[Any, Any]] = {
    "buildBudgetSnapshot": (build_budget_snapshot, BudgetSnapshotInput),
    "buildReservationCreated": (build_reservation_created, ReservationCreatedInput),
    "buildReservationCommitted": (build_reservation_committed, ReservationCommittedInput),
    "buildReservationReleased": (build_reservation_released, ReservationReleasedInput),
    "buildDecisionDenied": (build_decision_denied, DecisionDeniedInput),
}


def _load_corpus() -> dict[str, Any]:
    return json.loads(_CORPUS_PATH.read_text(encoding="utf-8"))


_CORPUS = _load_corpus()
_FIXTURES = _CORPUS["fixtures"]


def test_corpus_shape():
    """Corpus sanity: version 1, >= 20 vectors (tests.md §4), all five
    builders represented."""
    assert _CORPUS["version"] == 1
    assert len(_FIXTURES) >= 20
    assert {fx["builder"] for fx in _FIXTURES} == set(_BUILDER_MAP)


@pytest.mark.parametrize("fx", _FIXTURES, ids=[fx["id"] for fx in _FIXTURES])
def test_ta27_byte_equivalence(fx):
    """TA-27: every frozen vector — Python builder + canonical_event_json
    == expected_canonical_json byte-for-byte; encode_sse == expected_sse."""
    build, input_cls = _BUILDER_MAP[fx["builder"]]
    inp = input_cls(**fx["inputs"])  # corpus inputs are snake_case already
    evt = build(inp, timestamp_ms=fx.get("timestamp_ms"))

    got_json = canonical_event_json(evt)
    assert got_json == fx["expected_canonical_json"]
    assert got_json.encode("utf-8") == fx["expected_canonical_json"].encode("utf-8")

    got_sse = encode_sse(evt)
    assert got_sse == fx["expected_sse"]
    assert got_sse.encode("utf-8") == fx["expected_sse"].encode("utf-8")
