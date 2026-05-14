#!/bin/bash
# =====================================================================
# DEMO_MODE=cost_advisor — end-to-end closed-loop driver
# =====================================================================
#
# Seeds 10 budget-scoped TTL'd reservations, invokes spendguard-advise
# with --write-proposals, verifies cost_findings + approval_requests
# rows landed with correct shape, resolves the approval via
# resolve_approval_request SP, and checks the resulting state
# transition fired the pg_notify trigger.
#
# Exit codes:
#   0  full closed loop verified
#   1  any step failed (psql/binary error or assertion mismatch)
#
# Usage (run by `make demo-up DEMO_MODE=cost_advisor`):
#   cost_advisor_demo.sh
#
# Env (compose injects):
#   SPENDGUARD_COST_ADVISOR_LEDGER_DATABASE_URL
#   SPENDGUARD_COST_ADVISOR_CANONICAL_DATABASE_URL
#   SPENDGUARD_DEMO_TENANT_ID                 (default existing demo tenant)
#   SPENDGUARD_DEMO_BUDGET_ID                 (default existing demo budget)
#   SPENDGUARD_DEMO_DATE                      (default: today UTC)

set -euo pipefail

LEDGER_DB_URL="${SPENDGUARD_COST_ADVISOR_LEDGER_DATABASE_URL:?missing LEDGER_DB_URL}"
CANONICAL_DB_URL="${SPENDGUARD_COST_ADVISOR_CANONICAL_DATABASE_URL:?missing CANONICAL_DB_URL}"
DEMO_TENANT="${SPENDGUARD_DEMO_TENANT_ID:-00000000-0000-4000-8000-000000000001}"
DEMO_BUDGET="${SPENDGUARD_DEMO_BUDGET_ID:-44444444-4444-4444-8444-444444444444}"
DEMO_DATE="${SPENDGUARD_DEMO_DATE:-$(date -u +%Y-%m-%d)}"

log() { echo "[cost-advisor-demo] $*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

PSQL_LEDGER="psql -v ON_ERROR_STOP=1 -X $LEDGER_DB_URL"

log "config: tenant=$DEMO_TENANT budget=$DEMO_BUDGET date=$DEMO_DATE"

# ---------------------------------------------------------------------
# Step 0: prior-state pre-check (codex CA-demo r1 P1)
# ---------------------------------------------------------------------
# Terminal approvals (approved/denied/expired/cancelled) are
# audit-protected by approval_events FK — they can't be cleaned up
# between runs. The seed's cleanup deletes only PENDING approvals;
# if a prior run left a terminal cost_advisor approval on this DB,
# the new run would silently fall through to outcome='already_exists'
# from cost_advisor_create_proposal (same finding fingerprint →
# same decision_id), and the driver's `inserted` assertion would
# fail in confusing ways. Detect + fail loud with the fix instruction.
log "step 0: pre-check for prior demo state..."
PRIOR_TERMINAL=$(psql -tA -X "$LEDGER_DB_URL" -c "
    SELECT COUNT(*)
      FROM approval_requests
     WHERE tenant_id = '$DEMO_TENANT'
       AND proposal_source = 'cost_advisor'
       AND state IN ('approved', 'denied', 'expired', 'cancelled')
")
if [ "${PRIOR_TERMINAL:-0}" -gt 0 ]; then
    fail "$PRIOR_TERMINAL prior terminal cost_advisor approval(s) detected on this DB. The demo is not re-runnable on the same volume because audit-chain immutability protects terminal approvals. Run \`make demo-down -v\` to reset and try again."
fi
log "step 0 OK: no prior terminal state"

# ---------------------------------------------------------------------
# Step 1: seed reservations
# ---------------------------------------------------------------------
log "step 1: seed 10 reservations (7 TTL'd) for the demo bucket..."
$PSQL_LEDGER \
    -v demo_date="$DEMO_DATE" \
    -v tenant="$DEMO_TENANT" \
    -v budget="$DEMO_BUDGET" \
    -f /demo/cost_advisor_demo_seed.sql

# ---------------------------------------------------------------------
# Step 2: invoke spendguard-advise --write-proposals
# ---------------------------------------------------------------------
log "step 2: invoke spendguard-advise..."
ADVISE_OUTPUT=$(/usr/local/bin/spendguard-advise \
    --tenant "$DEMO_TENANT" \
    --date "$DEMO_DATE" \
    --show-proposed-patches \
    --write-proposals \
    --ledger-db "$LEDGER_DB_URL" \
    --canonical-db "$CANONICAL_DB_URL")

echo "$ADVISE_OUTPUT" | jq . >&2

# Assertion: 1 finding emitted with proposal_outcome=inserted (for the
# offending budget).
NUM_FINDINGS=$(echo "$ADVISE_OUTPUT" | jq '.findings | length')
[ "$NUM_FINDINGS" = "1" ] || \
    fail "expected 1 finding for the offending budget, got $NUM_FINDINGS"

PROPOSAL_OUTCOME=$(echo "$ADVISE_OUTPUT" | jq -r '.findings[0].proposal_outcome.outcome // "none"')
[ "$PROPOSAL_OUTCOME" = "inserted" ] || \
    fail "expected proposal_outcome=inserted, got $PROPOSAL_OUTCOME"

APPROVAL_ID=$(echo "$ADVISE_OUTPUT" | jq -r '.findings[0].proposal_outcome.approval_id')
[ -n "$APPROVAL_ID" ] && [ "$APPROVAL_ID" != "null" ] || \
    fail "advise output missing approval_id"

# Patch shape: must be 2-op (test+replace).
PATCH=$(echo "$ADVISE_OUTPUT" | jq '.findings[0].proposed_dsl_patch')
PATCH_OPS=$(echo "$PATCH" | jq 'length')
[ "$PATCH_OPS" = "2" ] || \
    fail "expected 2-op patch, got $PATCH_OPS"

OP0=$(echo "$PATCH" | jq -r '.[0].op')
PATH0=$(echo "$PATCH" | jq -r '.[0].path')
VALUE0=$(echo "$PATCH" | jq -r '.[0].value')
OP1=$(echo "$PATCH" | jq -r '.[1].op')
PATH1=$(echo "$PATCH" | jq -r '.[1].path')
VALUE1=$(echo "$PATCH" | jq -r '.[1].value')

[ "$OP0" = "test" ] || fail "patch op[0] should be 'test', got '$OP0'"
[ "$PATH0" = "/spec/budgets/0/id" ] || fail "patch path[0] should be /spec/budgets/0/id, got '$PATH0'"
[ "$VALUE0" = "$DEMO_BUDGET" ] || \
    fail "patch test op pins wrong budget_id (expected $DEMO_BUDGET, got $VALUE0)"

[ "$OP1" = "replace" ] || fail "patch op[1] should be 'replace', got '$OP1'"
[ "$PATH1" = "/spec/budgets/0/reservation_ttl_seconds" ] || \
    fail "patch path[1] should be /spec/budgets/0/reservation_ttl_seconds, got '$PATH1'"

# recommended TTL = (30 * 3 / 2).clamp(1, 86400) = 45
[ "$VALUE1" = "45" ] || \
    fail "patch replace value should be 45 (1.5x median 30s TTL), got '$VALUE1'"

log "step 2 OK: 1 budget-scoped finding emitted + 2-op identity-pinned patch inserted (approval_id=$APPROVAL_ID)"

# ---------------------------------------------------------------------
# Step 3: verify cost_findings + approval_requests in DB
# ---------------------------------------------------------------------
log "step 3: verify cost_findings + approval_requests state..."
# Pass values WITHOUT extra shell single-quotes — psql's :'name'
# substitution adds the quotes itself. Wire-time bug caught by the
# demo: doubling them produced literal "'00000000-..'" strings that
# couldn't cast to uuid.
$PSQL_LEDGER \
    -v tenant="$DEMO_TENANT" \
    -v budget="$DEMO_BUDGET" \
    -v approval_id="$APPROVAL_ID" \
    -v demo_date="$DEMO_DATE" \
    -f /demo/cost_advisor_demo_verify.sql

log "step 3 OK"

# ---------------------------------------------------------------------
# Step 4: resolve approval + verify state transition + trigger
# ---------------------------------------------------------------------
log "step 4: resolve approval via resolve_approval_request SP..."
$PSQL_LEDGER \
    -v approval_id="$APPROVAL_ID" \
    -f /demo/cost_advisor_demo_resolve.sql

log "step 4 OK"

# Note on wire-level NOTIFY verification: the
# `approval_requests_state_change_notify` trigger is verified to exist
# in step 3, and step 4 demonstrated a real state transition through
# `resolve_approval_request` that fires it. Wire-level NOTIFY delivery
# is independently proved by the Rust integration test
# `services/cost_advisor/tests/proposal_writer_smoke.rs::notify_fires_on_state_change`,
# which uses `sqlx::PgListener` to round-trip the actual payload.
# Re-proving it here would require either staging a second pending
# approval (which the cleanup logic can't reset because terminal
# approvals are audit-protected) or running a parallel LISTEN session
# from the demo container — both of which add wire complexity for no
# new evidence. We don't claim what we don't demonstrate.

log "PASS — Cost Advisor closed loop verified end-to-end."
log "      seed → cost_advisor binary → cost_findings → approval_requests"
log "      → resolve_approval_request → state=approved + approval_events audit"
log "      pg_notify trigger present (wire-level delivery: see proposal_writer_smoke.rs)"
