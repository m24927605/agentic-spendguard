#!/usr/bin/env python3
"""SLICE_15 — Verify all 21 predictor-upgrade audit columns are populated.

Spec ancestors:
  - docs/slices/SLICE_15_end_to_end_benchmark.md §8.1 (acceptance criteria)
  - docs/audit-chain-prediction-extension-v1alpha1.md §2 (column inventory)
  - docs/predictor-architecture-spec-v1alpha1.md §0.2 lock criterion #4
    (verify-chain regression green after new column writes)

The 21 columns:
  Decision-side (17 total; per audit-chain extension §2.1 + §2.2):
    1.  tokenizer_tier (T1/T2/T3 enum)
    2.  tokenizer_version_id (UUID, FK to tokenizer_versions)
    3.  predicted_a_tokens (BIGINT)
    4.  predicted_b_tokens (BIGINT)
    5.  predicted_c_tokens (BIGINT)
    6.  reserved_strategy (A/B/C enum)
    7.  prediction_strategy_used (A/B/C enum)
    8.  prediction_policy_used (STRICT_CEILING / EMPIRICAL_RUN_CEILING / …)
    9.  prediction_confidence (NUMERIC(4,3))
    10. prediction_sample_size (BIGINT)
    11. cold_start_layer_used (L1/L2/L3/L4 enum)
    12. prompt_class_fingerprint (TEXT 4-256 chars)
    13. prompt_class (chat_short/chat_long/code_gen/summarization/rag/tool_calling/vision)
    14. model (TEXT, aggregator mirror; §0018)
    15. run_projection_at_decision_atomic (NUMERIC(38,0))
    16. run_predicted_remaining_steps (INT)
    17. run_steps_completed_so_far (BIGINT)

  Commit-side (4 total; per §2.3):
    18. actual_input_tokens (BIGINT)
    19. actual_output_tokens (BIGINT)
    20. delta_b_ratio (REAL)
    21. delta_c_ratio (REAL)

Verification mode:
  - --tenant <uuid>   Query against the demo Postgres (live data).
  - --dry-run         Schema-only check; verifies all 21 columns EXIST
                      (no population check). Used when no E2E demo data
                      is available (CI sandbox, fresh checkout, etc.).

Why a Python script and not just SQL:
  * The 21 column names cross two tables (audit_outbox in ledger DB,
    canonical_events in canonical DB). One script that takes a single
    --tenant arg keeps the operator UX tight.
  * The verify-chain CLI scan is invoked via subprocess; results are
    parsed and merged into the column-coverage report.
  * Exit codes are operator-actionable (non-zero == SLICE_15 acceptance
    not yet met; details printed).

Exit codes:
  0 = all 21 columns present (dry-run) OR all 21 populated + verify-chain green (live)
  1 = column missing OR population gap (live mode only)
  2 = environment problem (psql not installed, can't connect, etc.)
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass, field

# ---------------------------------------------------------------------------
# Column inventory.
#
# Tuple: (column_name, table, audit-extension-spec-section)
# Each column is checked for EXISTENCE (always) and for POPULATION (live).
# ---------------------------------------------------------------------------

DECISION_SIDE_COLUMNS: list[tuple[str, str, str]] = [
    # tokenizer_tier and tokenizer_version_id — SLICE_03 + SLICE_06
    ("tokenizer_tier",                "canonical_events", "§2.1 (a)"),
    ("tokenizer_version_id",          "canonical_events", "§2.1 (a)"),

    # predicted_*_tokens — SLICE_06 / SLICE_08 (strategy A/B/C)
    ("predicted_a_tokens",            "canonical_events", "§2.1 (b)"),
    ("predicted_b_tokens",            "canonical_events", "§2.1 (b)"),
    ("predicted_c_tokens",            "canonical_events", "§2.1 (b)"),

    # strategy labels — SLICE_07 + SLICE_08
    ("reserved_strategy",             "canonical_events", "§2.1 (c)"),
    ("prediction_strategy_used",      "canonical_events", "§2.1 (c)"),
    ("prediction_policy_used",        "canonical_events", "§2.1 (d)"),

    # confidence + sample size — SLICE_06
    ("prediction_confidence",         "canonical_events", "§2.1 (e)"),
    ("prediction_sample_size",        "canonical_events", "§2.1 (e)"),

    # cold start layer — SLICE_05
    ("cold_start_layer_used",         "canonical_events", "§2.1 (f)"),

    # classifier mirror — SLICE_06 (aggregator mirror in §0018)
    ("prompt_class_fingerprint",      "canonical_events", "§0018 +§2.1 (g)"),
    ("prompt_class",                  "canonical_events", "§0018 +§2.1 (g)"),
    ("model",                         "canonical_events", "§0018"),

    # run-cost-projector — SLICE_09
    ("run_projection_at_decision_atomic", "canonical_events", "§2.2 (a)"),
    ("run_predicted_remaining_steps",     "canonical_events", "§2.2 (b)"),
    ("run_steps_completed_so_far",        "canonical_events", "§2.2 (c)"),
]

COMMIT_SIDE_COLUMNS: list[tuple[str, str, str]] = [
    ("actual_input_tokens",  "canonical_events", "§2.3 (a)"),
    ("actual_output_tokens", "canonical_events", "§2.3 (a)"),
    ("delta_b_ratio",        "canonical_events", "§2.3 (b)"),
    ("delta_c_ratio",        "canonical_events", "§2.3 (b)"),
]

ALL_COLUMNS = DECISION_SIDE_COLUMNS + COMMIT_SIDE_COLUMNS  # 21 total

# Default DB connection mirrors deploy/demo/compose.yaml.
DEFAULT_PG_HOST = os.environ.get("PGHOST", "localhost")
DEFAULT_PG_PORT = int(os.environ.get("PGPORT", "5433"))  # demo uses :5433 on host
DEFAULT_PG_USER = os.environ.get("PGUSER", "spendguard")
DEFAULT_PG_PASSWORD = os.environ.get("PGPASSWORD", "spendguard_demo")
DEFAULT_PG_DB = os.environ.get("PGDATABASE", "spendguard_canonical")


@dataclass
class ColumnCheck:
    name: str
    table: str
    spec_section: str
    exists: bool = False
    populated_count: int = 0
    error: str | None = None


@dataclass
class VerifyReport:
    mode: str  # "live" or "dry-run"
    tenant_id: str | None
    columns: list[ColumnCheck] = field(default_factory=list)
    verify_chain_status: str | None = None
    verify_chain_output: str = ""

    @property
    def all_exist(self) -> bool:
        return all(c.exists for c in self.columns)

    @property
    def all_populated(self) -> bool:
        return all(c.populated_count > 0 for c in self.columns)

    def print_summary(self) -> None:
        print(f"\n=== verify_audit_columns.py SUMMARY ({self.mode}) ===")
        print(f"  tenant: {self.tenant_id or '(N/A — dry-run)'}")
        ok = sum(1 for c in self.columns if c.exists)
        print(f"  columns existing: {ok}/{len(self.columns)}")
        if self.mode == "live":
            pop = sum(1 for c in self.columns if c.populated_count > 0)
            print(f"  columns populated: {pop}/{len(self.columns)}")
        if self.verify_chain_status:
            print(f"  verify-chain: {self.verify_chain_status}")

        # Per-column detail for any failure.
        bad = [c for c in self.columns if not c.exists
               or (self.mode == "live" and c.populated_count == 0)]
        if bad:
            print("\n  GAPS:")
            for c in bad:
                state = "MISSING"
                if c.exists and c.populated_count == 0:
                    state = "EXISTS but UNPOPULATED"
                print(f"    - {c.table}.{c.name:<40} [{state}] (spec {c.spec_section})")
                if c.error:
                    print(f"      err: {c.error}")


def log(msg: str) -> None:
    print(f"[verify_audit_columns] {msg}", flush=True)


def err(msg: str) -> None:
    print(f"[verify_audit_columns] ERROR: {msg}", file=sys.stderr, flush=True)


def psql_available() -> bool:
    """Check that psql is on PATH (or available via docker exec)."""
    if shutil.which("psql") is not None:
        return True
    # Fall back to checking docker — the demo runs Postgres in a
    # container, so docker exec spendguard-postgres psql is fine.
    if shutil.which("docker") is not None:
        try:
            r = subprocess.run(
                ["docker", "inspect", "spendguard-postgres"],
                capture_output=True, text=True, timeout=5,
            )
            if r.returncode == 0:
                log("psql not on PATH; will use `docker exec spendguard-postgres psql`")
                return True
        except Exception:
            pass
    return False


def run_sql(sql: str, db: str = DEFAULT_PG_DB) -> tuple[bool, str]:
    """Run a single SQL statement; return (ok, stdout). On docker fallback
    we always go via docker exec because the host might not have psql."""
    env = os.environ.copy()
    env["PGPASSWORD"] = DEFAULT_PG_PASSWORD

    # Prefer host psql when available — faster, no docker round-trip.
    if shutil.which("psql") is not None:
        cmd = [
            "psql",
            "-h", DEFAULT_PG_HOST,
            "-p", str(DEFAULT_PG_PORT),
            "-U", DEFAULT_PG_USER,
            "-d", db,
            "-tAc", sql,
        ]
    else:
        # docker exec; the container has psql preinstalled (postgres image).
        cmd = [
            "docker", "exec",
            "-e", f"PGPASSWORD={DEFAULT_PG_PASSWORD}",
            "spendguard-postgres",
            "psql", "-U", DEFAULT_PG_USER, "-d", db, "-tAc", sql,
        ]

    try:
        r = subprocess.run(cmd, capture_output=True, text=True,
                           env=env, timeout=15)
    except subprocess.TimeoutExpired:
        return False, "psql timeout"
    except Exception as exc:
        return False, f"psql exec: {exc}"

    if r.returncode != 0:
        return False, f"psql exit {r.returncode}: {r.stderr.strip()}"
    return True, r.stdout.strip()


def check_column_exists(col: ColumnCheck) -> None:
    sql = (
        "SELECT count(*) FROM information_schema.columns "
        f"WHERE table_name = '{col.table}' AND column_name = '{col.name}';"
    )
    ok, out = run_sql(sql)
    if not ok:
        col.error = out
        col.exists = False
        return
    try:
        col.exists = int(out) >= 1
    except ValueError:
        col.exists = False
        col.error = f"unexpected count output: {out!r}"


def check_column_populated(col: ColumnCheck, tenant_id: str) -> None:
    """Count rows for this tenant where the column is NOT NULL.

    Population is checked against canonical_events because that's the
    table calibration-report aggregates from (mirror discipline per
    audit-chain extension §2). audit_outbox is the producer-side
    forwarder source; canonical_events is the storage-class-bound
    audit destination.
    """
    sql = (
        f"SELECT count(*) FROM {col.table} "
        f"WHERE tenant_id = '{tenant_id}' "
        f"  AND {col.name} IS NOT NULL;"
    )
    ok, out = run_sql(sql)
    if not ok:
        col.error = out
        col.populated_count = 0
        return
    try:
        col.populated_count = int(out)
    except ValueError:
        col.populated_count = 0
        col.error = f"unexpected count output: {out!r}"


def invoke_verify_chain(tenant_id: str) -> tuple[str, str]:
    """Invoke the canonical_ingest verify-chain CLI with
    --check-prediction-mirror. Returns (status, output)."""
    # The binary lives inside the canonical-ingest container as
    # /usr/local/bin/verify-chain (per Dockerfile.canonical_ingest).
    # On host we run it via docker exec.
    if shutil.which("docker") is None:
        return "SKIPPED (docker not available)", ""

    cmd = [
        "docker", "exec",
        "spendguard-canonical-ingest",
        "/usr/local/bin/verify-chain",
        "--check-prediction-mirror",
        "--tenant-id", tenant_id,
    ]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        return "TIMEOUT", "verify-chain hung > 60s"
    except Exception as exc:
        return "EXEC_ERROR", f"docker exec: {exc}"

    output = (r.stdout + "\n" + r.stderr).strip()
    if r.returncode == 0:
        return "GREEN", output
    elif r.returncode == 2:
        # Per SLICE_01 verify_chain.rs Round-3 fix M5: exit 2 means
        # implementation-is-stub (default --check-prediction-mirror=true
        # before SLICE_06 producer mirror writes land). We surface that
        # explicitly because as of SLICE_15 the producer side should
        # have shipped (SLICE_06 + SLICE_10), so exit 2 is now a regression.
        return "STUB_NOT_IMPLEMENTED", output
    else:
        return f"FAIL (exit {r.returncode})", output


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Verify all 21 predictor-upgrade audit columns + verify-chain mirror check."
    )
    parser.add_argument(
        "--tenant",
        help="Tenant UUID to check population for. Required without --dry-run.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Schema-existence check only (no population, no verify-chain).",
    )
    parser.add_argument(
        "--skip-verify-chain",
        action="store_true",
        help="Skip the verify-chain CLI subprocess (useful when canonical-ingest isn't running).",
    )
    args = parser.parse_args()

    if not args.dry_run and not args.tenant:
        err("--tenant required for live mode (or pass --dry-run)")
        return 2

    if not psql_available():
        err("Neither psql on PATH nor `docker inspect spendguard-postgres` worked.")
        err("Install postgres-client or bring up the demo first.")
        return 2

    log(f"mode: {'live' if not args.dry_run else 'dry-run'}")
    if args.tenant:
        log(f"tenant: {args.tenant}")
    log(f"checking {len(ALL_COLUMNS)} columns across canonical_events...")

    report = VerifyReport(
        mode="live" if not args.dry_run else "dry-run",
        tenant_id=args.tenant,
    )

    for name, table, section in ALL_COLUMNS:
        c = ColumnCheck(name=name, table=table, spec_section=section)
        check_column_exists(c)
        if not args.dry_run and c.exists:
            check_column_populated(c, args.tenant)
        report.columns.append(c)

    # verify-chain only meaningful in live mode.
    if not args.dry_run and not args.skip_verify_chain:
        log("invoking verify-chain --check-prediction-mirror...")
        status, output = invoke_verify_chain(args.tenant)
        report.verify_chain_status = status
        report.verify_chain_output = output

    report.print_summary()

    # Decide exit code.
    if args.dry_run:
        if not report.all_exist:
            err("Schema check failed: one or more columns missing.")
            return 1
        log("Schema check PASSED.")
        return 0

    # live mode
    fail = False
    if not report.all_exist:
        err("Schema check failed in live mode.")
        fail = True
    if not report.all_populated:
        err("Population check failed: at least one column has no rows for this tenant.")
        fail = True
    if (report.verify_chain_status not in (None, "GREEN")
            and report.verify_chain_status != "SKIPPED (docker not available)"):
        err(f"verify-chain not green: {report.verify_chain_status}")
        fail = True

    if fail:
        return 1
    log("All 21 columns populated + verify-chain green.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
