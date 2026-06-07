#!/usr/bin/env python3
"""
D15 COV_74 — translate `importer_manus` CloudEvent envelopes (read
from stdin) into a single `audit_outbox` INSERT statement (printed to
stdout). Used by `import_manus_fixture_demo.sh`.

The mapping is purely cosmetic — the actual contract is the CloudEvent
shape that the importer emits. This script exists so the demo runner
can land rows in postgres without standing up the full
canonical_ingest gRPC service.
"""
from __future__ import annotations

import json
import sys


def sql_str(v):
    """SQL-quote a string with naïve single-quote escaping. The fixture
    is sanitized synthetic-only so this is sufficient; production INSERT
    goes through canonical_ingest's parameterized API."""
    if v is None:
        return "NULL"
    return "'" + str(v).replace("'", "''") + "'"


def main() -> int:
    envs = json.load(sys.stdin)
    if not envs:
        print("-- no envelopes emitted; nothing to insert", file=sys.stderr)
        return 0
    rows = []
    for env in envs:
        d = env["data"]
        cells = [
            sql_str(env["id"]),
            sql_str(d["tenant_id"]),
            sql_str(d["reservation_source"]),
            sql_str(d["import_source"]),
            sql_str(d["model"]),
            str(d["credits_consumed"]),
            str(d["credit_cost_micro_usd"]),
            str(d["amount_micro_usd"]),
            sql_str(d["pricing_version"]),
            sql_str(d["tier"]),
            sql_str(d["status"]),
            sql_str(d["session_id"]),
            sql_str(d["workspace_id"]),
            str(d["input_tokens"]),
            str(d["output_tokens"]),
            sql_str(d["window_end"]) + "::timestamptz",
            sql_str(d["ingestion_mode"]),
            sql_str(d["fixture_provenance_sha256"]),
            sql_str(d["dedupe_key"]),
        ]
        rows.append("(" + ",".join(cells) + ")")
    print(
        "INSERT INTO audit_outbox (event_id, tenant_id, "
        "reservation_source, import_source, model, "
        "credits_consumed, credit_cost_micro_usd, amount_micro_usd, "
        "pricing_version, tier, status, session_id, workspace_id, "
        "input_tokens, output_tokens, occurred_at, "
        "ingestion_mode, fixture_provenance_sha256, dedupe_key) VALUES "
        + ", ".join(rows)
        + " ON CONFLICT (event_id) DO NOTHING;"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
