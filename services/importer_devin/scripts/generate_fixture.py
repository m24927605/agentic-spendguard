#!/usr/bin/env python3
"""
D14 COV_69 — Generate the canonical sanitized devin_usage.json
fixture used by the SpendGuard Devin billing importer's tests +
demo verifier.

The fixture is committed at
`services/importer_devin/tests/fixtures/devin_usage.json`. Re-running
this script produces a byte-identical file (sorted keys, fixed
timestamps, no clock reads). The PROVENANCE.md sibling pins the
SHA-256 of THIS script — review-standards T6 verifies the pin matches
on every review.

Synthetic IDs only (review-standards T5):
    TEAM_FIXTURE_NNN     (N = ascii digit, exactly 3 digits)
    SESSION_FIXTURE_NNN  (same shape)

Run:
    python3 services/importer_devin/scripts/generate_fixture.py \\
        > services/importer_devin/tests/fixtures/devin_usage.json
"""
from __future__ import annotations

import json
import sys

USAGE = [
    # ── team plan: full-price conversion exercised ──
    {
        "tenant_id": "demo",
        "budget_id": "devin-budget",
        "devin_team_id": "TEAM_FIXTURE_001",
        "devin_session_id": "SESSION_FIXTURE_001",
        "acu_consumed": 12.5,
        "plan": "team",
        "window_start": "2026-06-01T00:00:00Z",
        "window_end": "2026-06-01T01:00:00Z",
    },
    {
        "tenant_id": "demo",
        "budget_id": "devin-budget",
        "devin_team_id": "TEAM_FIXTURE_001",
        "devin_session_id": "SESSION_FIXTURE_002",
        "acu_consumed": 4.0,
        "plan": "team",
        "window_start": "2026-06-01T01:00:00Z",
        "window_end": "2026-06-01T02:00:00Z",
    },
    # ── enterprise plan: NULL micro_usd path exercised ──
    {
        "tenant_id": "demo",
        "budget_id": "devin-budget",
        "devin_team_id": "TEAM_FIXTURE_002",
        "devin_session_id": "SESSION_FIXTURE_003",
        "acu_consumed": 100.0,
        "plan": "enterprise",
        "window_start": "2026-06-01T00:00:00Z",
        "window_end": "2026-06-01T01:00:00Z",
    },
]


def main() -> int:
    doc = {
        "_meta": {
            "schema": "devin_usage_fixture_v1",
            "generated_at": "2026-06-08T00:00:00Z",
            "vendor_snapshot_url": "https://api.devin.ai/api/v1/teams/TEAM_FIXTURE_001/usage",
            "synthetic_only": True,
        },
        "usage": USAGE,
    }
    json.dump(doc, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
