#!/bin/bash
# SpendGuard Phase 3 Wedge — asciinema demo script.
# Assumes: docker compose stack already up + healthy.

set -e
COMPOSE="docker compose -f deploy/demo/compose.yaml"

step() { printf "\n\033[1;36m== %s ==\033[0m\n\n" "$1"; sleep 1; }
say()  { printf "\033[2m# %s\033[0m\n" "$1"; sleep 1; }

clear
cat <<'EOF'
============================================================
SpendGuard — Agent Runtime Spend Guardrails
Phase 3 Wedge: Contract DSL hot-path evaluator (POC)

  agent step → sidecar (<5ms) → reads contract.yaml → STOP / DEGRADE / pass
                                                    → audit row durable
                                                    → ledger reservation atomic
============================================================
EOF
sleep 3

step "1. The contract a customer ships (excerpt from bundle)"
say "Declarative when/then; POC subset of Contract DSL §6/§7."
grep -A 18 'apiVersion: contract' deploy/demo/init/bundles/generate.sh \
    | head -22 | sed 's/^/  /'
sleep 4

step "2. Send a \$2000 claim — twice the \$1000 hard cap"
say "Adapter raises DecisionStopped before any reservation is taken."
sleep 1
$COMPOSE run --rm --env SPENDGUARD_DEMO_MODE=deny demo 2>&1 \
    | grep -E '\[demo\]' \
    | head -5
sleep 3

step "3. The audit chain — ledger.audit_outbox"
say "Carrier ledger_transactions row (no entries) + 1 audit_outbox row."
$COMPOSE exec -T postgres psql -U spendguard -d spendguard_ledger -c \
    "SELECT operation_kind,
            posting_state,
            decision_id
       FROM ledger_transactions
      WHERE operation_kind = 'denied_decision'
      ORDER BY recorded_at DESC LIMIT 1;"
sleep 2

step "4. ... and what canonical_events received (compliance store)"
say "Outbox forwarder pushed the audit row immutably."
$COMPOSE exec -T postgres psql -U spendguard -d spendguard_canonical -c \
    "SELECT event_type,
            substring(source, 1, 38) AS source
       FROM canonical_events
      ORDER BY event_time DESC LIMIT 3;"
sleep 3

step "5. The wedge in one diagram"
cat <<'EOF'

  agent  ─►  sidecar  ─►  ledger  ─►  audit_outbox  ─►  canonical_events
              │             │             │
              │             │             └─ Postgres SERIALIZABLE
              │             └─ atomic reserve / denied_decision
              └─ contract evaluator (Stage 2, <5 ms hot-path)

  Demo modes (all green): decision · invoice · agent · release · ttl_sweep · deny
EOF
sleep 4

printf "\n\033[1;32mPhase 3 wedge — Predict ∩ Control intersection.\033[0m\n"
printf "\033[2mhttps://github.com/m24927605/agentic-flow-cost-evaluation/pull/1\033[0m\n\n"
