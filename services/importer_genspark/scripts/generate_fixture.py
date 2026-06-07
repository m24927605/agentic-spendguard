#!/usr/bin/env python3
"""
D16 COV_86 — Generate the canonical sanitized genspark_usage.json
fixture used by the SpendGuard Genspark billing importer's tests +
demo verifier.

The fixture is committed at
`services/importer_genspark/tests/fixtures/genspark_usage.json`.
Re-running this script produces a byte-identical file (sorted keys,
fixed timestamps, no clock reads). The PROVENANCE.md sibling pins the
SHA-256 of THIS script — review-standards P2 verifies the pin matches
on every review.

Synthetic IDs only (review-standards T9):
    FAKE_ws_NNN    (N = ascii digit, exactly 3 digits)
    FAKE_task_NNN  (same shape)

Plan coverage (per acceptance gates):
    - "plus"       full-price conversion exercised
    - "premium"    higher-tier conversion exercised
    - "enterprise" unknown-plan fallback exercised (NOT in price table)

Run:
    python3 services/importer_genspark/scripts/generate_fixture.py \\
        > services/importer_genspark/tests/fixtures/genspark_usage.json
"""
from __future__ import annotations

import json
import sys

USAGE = [
    # ── plus plan: full-price conversion exercised ──
    {
        "tenant_id": "demo",
        "budget_id": "genspark-budget",
        "workspace_id": "FAKE_ws_001",
        "task_id": "FAKE_task_001",
        "credits_consumed": 3200.0,
        "plan": "plus",
        "task_category": "research",
        "window_start": "2026-06-01T00:00:00Z",
        "window_end": "2026-06-01T01:00:00Z",
    },
    {
        "tenant_id": "demo",
        "budget_id": "genspark-budget",
        "workspace_id": "FAKE_ws_001",
        "task_id": "FAKE_task_002",
        "credits_consumed": 1850.0,
        "plan": "plus",
        "task_category": "code_generation",
        "window_start": "2026-06-01T01:00:00Z",
        "window_end": "2026-06-01T02:00:00Z",
    },
    # ── premium plan: higher-tier conversion exercised ──
    {
        "tenant_id": "demo",
        "budget_id": "genspark-budget",
        "workspace_id": "FAKE_ws_002",
        "task_id": "FAKE_task_003",
        "credits_consumed": 50000.0,
        "plan": "premium",
        "task_category": "research",
        "window_start": "2026-06-01T00:00:00Z",
        "window_end": "2026-06-01T01:00:00Z",
    },
    # ── enterprise plan: unknown-plan fallback exercised ──
    # NOTE: "enterprise" is NOT in the embedded price table; this row
    # forces the genspark_plan_unknown reason_code path (T7, F4).
    {
        "tenant_id": "demo",
        "budget_id": "genspark-budget",
        "workspace_id": "FAKE_ws_003",
        "task_id": "FAKE_task_004",
        "credits_consumed": 1000.0,
        "plan": "enterprise",
        "task_category": None,
        "window_start": "2026-06-01T00:00:00Z",
        "window_end": "2026-06-01T01:00:00Z",
    },
]


def main() -> int:
    doc = {
        "_meta": {
            "schema": "genspark_usage_fixture_v1",
            "generated_at": "2026-06-08T00:00:00Z",
            "vendor_snapshot_url": "https://api.genspark.ai/v1/admin/usage?workspace=FAKE_ws_001",
            "synthetic_only": True,
        },
        "usage": USAGE,
    }
    json.dump(doc, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
