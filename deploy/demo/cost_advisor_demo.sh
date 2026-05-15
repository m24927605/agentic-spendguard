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

# Capture the bundle hash BEFORE any write path runs (codex CA-P3.5
# r3 P3 — was captured after the resolve, racing bundle_registry's
# fast-rotation). Step 5 polls this value for change.
CONTRACT_BUNDLE_ID="${CONTRACT_BUNDLE_ID:-11111111-1111-4111-8111-111111111111}"
BUNDLE_TGZ="/var/lib/spendguard/bundles/contract_bundle/${CONTRACT_BUNDLE_ID}.tgz"
RUNTIME_ENV="/var/lib/spendguard/bundles/runtime.env"
HASH_KEY="SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX"
HASH_KEY_PREFIX="${HASH_KEY}="
read_hash() {
    awk -v p="$HASH_KEY_PREFIX" 'index($0, p)==1 {print substr($0, length(p)+1)}' "$RUNTIME_ENV"
}
OLD_HASH=$(read_hash)
log "  baseline bundle hash (pre-write-path): ${OLD_HASH:-<unset>}"

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
log "step 4 (CA-P3.6): resolve approval via dashboard REST API..."
# Was psql resolve_approval_request — now POSTs to the operator UI's
# /api/approvals/:id/resolve endpoint so the full operator-facing
# path is exercised end-to-end. Dashboard handler ultimately calls
# the same SP under the hood.
DASHBOARD_URL="${SPENDGUARD_DASHBOARD_URL:?missing SPENDGUARD_DASHBOARD_URL}"
DASHBOARD_TOKEN="${SPENDGUARD_DASHBOARD_TOKEN:?missing SPENDGUARD_DASHBOARD_TOKEN}"

# Pre-check: detail endpoint should return the row.
DETAIL=$(curl -sS \
    -H "Authorization: Bearer $DASHBOARD_TOKEN" \
    "${DASHBOARD_URL}/api/approvals/${APPROVAL_ID}")
DETAIL_STATE=$(echo "$DETAIL" | jq -r '.state // "missing"')
[ "$DETAIL_STATE" = "pending" ] || \
    fail "dashboard detail says state=$DETAIL_STATE (expected pending); response=$DETAIL"
DETAIL_SOURCE=$(echo "$DETAIL" | jq -r '.proposal_source // "missing"')
[ "$DETAIL_SOURCE" = "cost_advisor" ] || \
    fail "dashboard detail proposal_source=$DETAIL_SOURCE (expected cost_advisor)"
# Assert evidence is actually returned (codex CA-P3.6 r1 P3 — was
# just logged 'finding evidence ✓' without checking the field exists).
EVIDENCE_NULL=$(echo "$DETAIL" | jq -r '.finding_evidence == null')
[ "$EVIDENCE_NULL" = "false" ] || \
    fail "dashboard detail.finding_evidence is null (expected the cost_findings.evidence JSONB)"
EVIDENCE_SCOPE=$(echo "$DETAIL" | jq -r '.finding_evidence.scope.scope_type // "missing"')
[ "$EVIDENCE_SCOPE" = "budget" ] || \
    fail "expected finding_evidence.scope.scope_type=budget, got $EVIDENCE_SCOPE"
log "  dashboard GET /api/approvals/${APPROVAL_ID} returned state=pending + finding_evidence.scope.scope_type=budget ✓"

# Resolve via POST.
RESOLVE_BODY='{"target_state":"approved","reason":"cost-advisor demo: auto-approve the rotated patch"}'
RESOLVE_RESP=$(curl -sS \
    -X POST \
    -H "Authorization: Bearer $DASHBOARD_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$RESOLVE_BODY" \
    "${DASHBOARD_URL}/api/approvals/${APPROVAL_ID}/resolve")
RESOLVE_FINAL=$(echo "$RESOLVE_RESP" | jq -r '.final_state // "missing"')
RESOLVE_TRANS=$(echo "$RESOLVE_RESP" | jq -r '.transitioned')
[ "$RESOLVE_FINAL" = "approved" ] || \
    fail "dashboard resolve final_state=$RESOLVE_FINAL (expected approved); response=$RESOLVE_RESP"
[ "$RESOLVE_TRANS" = "true" ] || \
    fail "dashboard resolve transitioned=$RESOLVE_TRANS (expected true)"
log "  dashboard POST /api/approvals/${APPROVAL_ID}/resolve → approved (transitioned=true) ✓"

# Verify the DB-level state changed + approval_events audit row written.
# Psql variable interpolation doesn't penetrate DO $$ blocks (wire bug
# from the CA-demo work — see cost_advisor_demo_seed.sql); inline the
# approval_id as a literal via shell instead.
$PSQL_LEDGER -c "
DO \$\$
DECLARE
    v_state TEXT;
    v_events INT;
BEGIN
    SELECT state INTO v_state FROM approval_requests
     WHERE approval_id = '${APPROVAL_ID}'::uuid;
    IF v_state <> 'approved' THEN
        RAISE EXCEPTION 'expected state=approved after dashboard resolve, got %', v_state;
    END IF;
    SELECT COUNT(*) INTO v_events FROM approval_events
     WHERE approval_id = '${APPROVAL_ID}'::uuid AND to_state = 'approved';
    IF v_events = 0 THEN
        RAISE EXCEPTION 'approval_events missing the pending→approved audit row';
    END IF;
    RAISE NOTICE 'DB state confirmed: approved + % audit row(s)', v_events;
END \$\$;
"

log "step 4 OK"

# ---------------------------------------------------------------------
# Step 5 (CA-P3.5): verify bundle_registry applied the patch
# ---------------------------------------------------------------------
# bundle_registry is LISTENing on approval_requests_state_change.
# Step 4's resolve fired the trigger; bundle_registry should now be
# extracting the active bundle, applying the 2-op test+replace patch,
# re-packing the .tgz, and updating runtime.env. Poll the bundle file
# for the patched value (reservation_ttl_seconds: 45).
log "step 5: poll bundle file for bundle_registry's applied patch..."

# Use the OLD_HASH captured at step-2 entry (pre-write-path).
log "  baseline bundle hash (re-display): ${OLD_HASH:-<unset>}"

# Poll for up to 10s (bundle_registry latency is dominated by
# postgres NOTIFY dispatch + a few file syscalls, typically <1s).
WAIT_SECS=0
NEW_HASH=""
while [ "$WAIT_SECS" -lt 10 ]; do
    sleep 1
    WAIT_SECS=$((WAIT_SECS + 1))
    NEW_HASH=$(read_hash)
    if [ -n "$NEW_HASH" ] && [ "$NEW_HASH" != "$OLD_HASH" ]; then
        break
    fi
done

if [ -z "$NEW_HASH" ] || [ "$NEW_HASH" = "$OLD_HASH" ]; then
    fail "bundle_registry did not rotate the bundle within ${WAIT_SECS}s (old hash unchanged: $OLD_HASH)"
fi
log "  bundle hash rotated: $OLD_HASH → $NEW_HASH (after ${WAIT_SECS}s)"

# Extract the new contract.yaml from the rotated bundle and assert
# the patched value landed.
EXTRACTED_YAML=$(mktemp -d)
tar -xzf "$BUNDLE_TGZ" -C "$EXTRACTED_YAML"
NEW_TTL=$(grep -E 'reservation_ttl_seconds:' "$EXTRACTED_YAML/contract.yaml" | head -1 | awk '{print $2}')
NEW_BUDGET=$(grep -E '^\s+id:' "$EXTRACTED_YAML/contract.yaml" | grep -F "$DEMO_BUDGET" | head -1)
rm -rf "$EXTRACTED_YAML"

[ "$NEW_TTL" = "45" ] || \
    fail "bundle_registry's patched contract.yaml has reservation_ttl_seconds=$NEW_TTL, expected 45"
[ -n "$NEW_BUDGET" ] || \
    fail "patched contract.yaml does not contain the demo budget id (test op identity should have preserved it)"

log "step 5 OK: contract.yaml has reservation_ttl_seconds=45 + demo budget id preserved"

log "PASS — Cost Advisor closed loop verified end-to-end."
log "      seed → cost_advisor binary → cost_findings → approval_requests"
log "      → dashboard GET (operator UI lists + shows evidence)"
log "      → dashboard POST /api/approvals/:id/resolve → state=approved"
log "      → bundle_registry LISTEN → patched bundle + new hash published"
log "      (wire-level NOTIFY delivery also covered by proposal_writer_smoke.rs)"
