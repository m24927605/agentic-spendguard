#!/usr/bin/env python3
"""
D15 COV_72 — Generate the canonical sanitized manus_usage.json
fixture used by the SpendGuard Manus billing importer's tests +
demo verifier.

The fixture is committed at
`services/importer_manus/tests/fixtures/manus_usage.json`. Re-running
this script produces a byte-identical file (sorted keys, fixed
timestamps, no clock reads). The PROVENANCE.md sibling pins the
SHA-256 of THIS script.

Synthetic IDs only (review-standards T8 / A10.4 / A10.5):
    ws_FAKE_*    (workspace IDs)
    ses_FAKE_*   (session IDs)

8 sessions across 3 tiers per design implementation.md §5 +
acceptance A1.5:

    1. team_plan / completed     ws_FAKE_team_001 / 47 credits
    2. team_plan / failed        ws_FAKE_team_001 / 12 credits
    3. team_plan / cancelled     ws_FAKE_team_002 / 0 credits
    4. team_plan / in_progress   ws_FAKE_team_002 / 8 credits   (skipped)
    5. enterprise / completed    ws_FAKE_ent_001  / 350 credits
    6. enterprise_byok / done    ws_FAKE_byok_001 / 1024 credits
    7. team_plan / large         ws_FAKE_team_001 / 950 credits
    8. team_plan / minimal       ws_FAKE_team_003 / 1 credit

Five team_plan terminal rows total 47+12+0+950+1 = 1010 credits
  1010 × 20_526 micro-USD/credit = 20_731_260 micro-USD
That's the headline gate the demo verifier asserts (A5.4).

Run:
    python3 services/importer_manus/scripts/generate_fixture.py \\
        > services/importer_manus/tests/fixtures/manus_usage.json
"""
from __future__ import annotations

import json
import sys

SESSIONS = [
    {
        "session_id": "ses_FAKE_team_completed_001",
        "workspace_id": "ws_FAKE_team_001",
        "tier": "team_plan",
        "credits_consumed": 47,
        "status": "completed",
        "started_at": "2026-06-05T14:22:08Z",
        "completed_at": "2026-06-05T14:34:51Z",
    },
    {
        "session_id": "ses_FAKE_team_failed_002",
        "workspace_id": "ws_FAKE_team_001",
        "tier": "team_plan",
        "credits_consumed": 12,
        "status": "failed",
        "started_at": "2026-06-05T15:01:11Z",
        "completed_at": "2026-06-05T15:02:42Z",
    },
    {
        "session_id": "ses_FAKE_team_cancelled_003",
        "workspace_id": "ws_FAKE_team_002",
        "tier": "team_plan",
        "credits_consumed": 0,
        "status": "cancelled",
        "started_at": "2026-06-05T16:00:00Z",
        "completed_at": "2026-06-05T16:00:30Z",
    },
    {
        "session_id": "ses_FAKE_team_inprogress_004",
        "workspace_id": "ws_FAKE_team_002",
        "tier": "team_plan",
        "credits_consumed": 8,
        "status": "in_progress",
        "started_at": "2026-06-05T17:00:00Z",
        "completed_at": "2026-06-05T17:00:00Z",
    },
    {
        "session_id": "ses_FAKE_enterprise_005",
        "workspace_id": "ws_FAKE_ent_001",
        "tier": "enterprise",
        "credits_consumed": 350,
        "status": "completed",
        "started_at": "2026-06-05T09:11:00Z",
        "completed_at": "2026-06-05T11:48:00Z",
    },
    {
        "session_id": "ses_FAKE_byok_006",
        "workspace_id": "ws_FAKE_byok_001",
        "tier": "enterprise_byok",
        "credits_consumed": 1024,
        "status": "completed",
        "started_at": "2026-06-05T20:00:00Z",
        "completed_at": "2026-06-05T22:30:00Z",
    },
    {
        "session_id": "ses_FAKE_team_large_007",
        "workspace_id": "ws_FAKE_team_001",
        "tier": "team_plan",
        "credits_consumed": 950,
        "status": "completed",
        "started_at": "2026-06-04T08:00:00Z",
        "completed_at": "2026-06-04T18:00:00Z",
    },
    {
        "session_id": "ses_FAKE_team_minimal_008",
        "workspace_id": "ws_FAKE_team_003",
        "tier": "team_plan",
        "credits_consumed": 1,
        "status": "completed",
        "started_at": "2026-06-05T12:00:00Z",
        "completed_at": "2026-06-05T12:01:00Z",
    },
]


def main() -> int:
    doc = {
        "_meta": {
            "schema": "manus_usage_fixture_v1",
            "generated_at": "2026-06-08T00:00:00Z",
            "vendor_snapshot_url": "https://api.manus.ai/v1/usage",
            "synthetic_only": True,
        },
        "next_cursor": None,
        "sessions": SESSIONS,
    }
    json.dump(doc, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
