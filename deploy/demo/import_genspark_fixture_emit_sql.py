#!/usr/bin/env python3
"""
D16 COV_88 — translate `importer_genspark` CloudEvent envelopes (read
from stdin) into a single `audit_outbox` INSERT statement (printed to
stdout). Used by `import_genspark_fixture_demo.sh`.

The mapping is purely cosmetic — the actual contract is the CloudEvent
shape that the importer emits. This script exists so the demo runner
can land rows in postgres without standing up the full
canonical_ingest gRPC service.
"""
from __future__ import annotations

import json
import sys


def sql_quote(s: str) -> str:
    # Minimal escaping for SQL string literal — replace single quotes
    # with two single quotes. Synthetic fixture IDs only; no untrusted
    # input.
    return s.replace("'", "''")


def main() -> int:
    envs = json.load(sys.stdin)
    if not envs:
        print("-- no envelopes emitted; nothing to insert", file=sys.stderr)
        return 0
    rows = []
    for env in envs:
        d = env["data"]
        plan_slug = d.get("plan", "unknown")
        cells = [
            f"'{sql_quote(env['id'])}'",
            f"'{sql_quote(d['tenant_id'])}'",
            f"'{sql_quote(d['budget_id'])}'",
            f"'{sql_quote(d['reservation_source'])}'",
            f"'{sql_quote(d['import_source'])}'",
            f"'genspark/credit/{sql_quote(plan_slug)}'",
            str(d["credits_consumed"]),
            str(d["amount_micro_usd"]),
            f"'{sql_quote(d['pricing_version'])}'",
            (
                f"'{sql_quote(d['reason_code'])}'"
                if d["reason_code"] is not None
                else "NULL"
            ),
            f"'{d['window_end']}'::timestamptz",
            f"'{sql_quote(d['ingestion_mode'])}'",
            (
                f"'{sql_quote(d['fixture_provenance_sha256'])}'"
                if d["fixture_provenance_sha256"]
                else "NULL"
            ),
            (
                f"'{sql_quote(d['task_category'])}'"
                if d.get("task_category") is not None
                else "NULL"
            ),
        ]
        rows.append("(" + ",".join(cells) + ")")
    print(
        "INSERT INTO audit_outbox (event_id, tenant_id, budget_id, "
        "reservation_source, import_source, model, credits_consumed, "
        "amount_micro_usd, pricing_version, reason_code, occurred_at, "
        "ingestion_mode, fixture_provenance_sha256, task_category) VALUES "
        + ", ".join(rows)
        + " ON CONFLICT (event_id) DO NOTHING;"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
