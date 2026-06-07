#!/usr/bin/env python3
"""
D14 COV_72 — translate `importer_devin` CloudEvent envelopes (read
from stdin) into a single `audit_outbox` INSERT statement (printed to
stdout). Used by `import_devin_fixture_demo.sh`.

The mapping is purely cosmetic — the actual contract is the CloudEvent
shape that the importer emits. This script exists so the demo runner
can land rows in postgres without standing up the full
canonical_ingest gRPC service.
"""
from __future__ import annotations

import json
import sys


def main() -> int:
    envs = json.load(sys.stdin)
    if not envs:
        print("-- no envelopes emitted; nothing to insert", file=sys.stderr)
        return 0
    rows = []
    for env in envs:
        d = env["data"]
        plan = (
            "enterprise"
            if d["amount_micro_usd"] is None
            else "team"
        )
        cells = [
            f"'{env['id']}'",
            f"'{d['tenant_id']}'",
            f"'{d['budget_id']}'",
            f"'{d['reservation_source']}'",
            f"'{d['import_source']}'",
            f"'devin/acu/{plan}'",
            str(d["acu_consumed"]),
            (
                str(d["amount_micro_usd"])
                if d["amount_micro_usd"] is not None
                else "NULL"
            ),
            f"'{d['pricing_version']}'",
            (
                "'devin_enterprise_negotiated_rate'"
                if d["amount_micro_usd"] is None
                else "NULL"
            ),
            f"'{d['window_end']}'::timestamptz",
            f"'{d['ingestion_mode']}'",
            (
                f"'{d['fixture_provenance_sha256']}'"
                if d["fixture_provenance_sha256"]
                else "NULL"
            ),
        ]
        rows.append("(" + ",".join(cells) + ")")
    print(
        "INSERT INTO audit_outbox (event_id, tenant_id, budget_id, "
        "reservation_source, import_source, model, acu_consumed, "
        "amount_micro_usd, pricing_version, reason_code, occurred_at, "
        "ingestion_mode, fixture_provenance_sha256) VALUES "
        + ", ".join(rows)
        + " ON CONFLICT (event_id) DO NOTHING;"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
