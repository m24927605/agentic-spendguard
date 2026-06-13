// COV_D39 SLICE 3 — AG-UI spend-event demo runner (DEMO_MODE=ag_ui_events).
//
// Drives a REAL SpendGuard run against the sidecar UDS and renders each
// decision the enforcement plane made as a `spendguard.*` AG-UI CUSTOM
// event (display-only — see README.md; AG-UI can NOT gate anything, the
// sidecar already did):
//
//   step 0  connect + handshake (SpendGuardClient, sidecar UDS)
//   step 1  emit spendguard.budget.snapshot   (seed env values; design.md §9.2 —
//           cross-checked against the ledger by verify_step_ag_ui_events.sql)
//   step 2  ALLOW: client.reserve → emit spendguard.reservation.created
//           → POST http://counting-stub:8765/v1/chat/completions
//           → client.commitEstimated(SUCCESS) → emit spendguard.reservation.committed
//   step 3  DENY: client.reserve(amount > seeded hard-cap) → catch DecisionDenied
//           → emit spendguard.decision.denied
//           → assert counting-stub /_count UNCHANGED
//   step 4  serve :8077  GET /healthz → 200 "ok"
//                        GET /events  → replay recorded encodeSse frames, close
//
// Honesty rules (review-standards §7, HARDEN_04 lesson): every event field
// traces to a real RPC outcome (decision_id / reservation_id / ttl / reason
// codes straight off the wire) or to a ledger-cross-checked seed env value.
// Nothing is fabricated; D39 derives no IDs of its own (run/llm-call/decision
// IDs come from the substrate's newUuid7 / deriveIdempotencyKey).
//
// Spec: docs/specs/coverage/D39_ag_ui/design.md §9 (LOCKED) +
//       implementation.md §7.

import http from "node:http";

import {
  buildBudgetSnapshot,
  buildDecisionDenied,
  buildReservationCommitted,
  buildReservationCreated,
  encodeSse,
} from "@spendguard/ag-ui";
import { DecisionDenied, deriveIdempotencyKey, newUuid7, SpendGuardClient } from "@spendguard/sdk";

// ── Env (set by deploy/demo/ag_ui_events/docker-compose.yaml) ─────────────
const SOCKET_PATH = process.env.SPENDGUARD_SIDECAR_UDS ?? "/var/run/spendguard/adapter.sock";
const TENANT_ID = process.env.SPENDGUARD_TENANT_ID ?? "00000000-0000-4000-8000-000000000001";
const BUDGET_ID = process.env.SPENDGUARD_BUDGET_ID ?? "44444444-4444-4444-8444-444444444444";
const WINDOW_INSTANCE_ID =
  process.env.SPENDGUARD_WINDOW_INSTANCE_ID ?? "55555555-5555-4555-8555-555555555555";
const UNIT_ID = process.env.SPENDGUARD_UNIT_ID ?? "66666666-6666-4666-8666-666666666666";
// Seeded opening balance (deploy/demo/init/migrations/30_seed_demo_state.sh
// credits available_budget with exactly this amount at compose-up). The SQL
// gate cross-checks this value against the actual seed ledger entry so the
// snapshot is never a fabricated number.
const OPENING_BALANCE_ATOMIC = process.env.SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC ?? "500";
const COUNTING_STUB_URL =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const HANDSHAKE_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_HANDSHAKE_TIMEOUT_MS ?? "30000",
  10,
);

// ASP unit slug for the display payloads (claims on this demo lane are
// micro-dollar denominated — same convention as the langchain_ts demo).
const UNIT_SLUG = "usd_micros";
// Wire-side UnitRef for reserve/commit (HARDEN_D05_UR unitId threading).
const WIRE_UNIT = { unit: "USD_MICROS", denomination: 1, unitId: UNIT_ID };
// ALLOW-step claim: equal to the displayed fresh-stack snapshot balance and
// well under the seeded 1B hard-cap.
const ALLOW_AMOUNT_ATOMIC = "500";
// DENY-step claim: above the seeded `claim_amount_atomic_gt: "1000000000"`
// hard-cap rule, so the sidecar contract evaluator denies pre-dispatch.
const DENY_AMOUNT_ATOMIC = "2000000000";

// HARDEN_D05_WI — pricing freeze tuple sourced from bundles runtime.env by
// the container entrypoint (version + snapshot hash hex + fx + units).
const PRICING = {
  pricingVersion: process.env.SPENDGUARD_PRICING_VERSION ?? "",
  pricingHash: process.env.SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX
    ? Uint8Array.from(Buffer.from(process.env.SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX, "hex"))
    : new Uint8Array(0),
  fxRateVersion: process.env.SPENDGUARD_FX_RATE_VERSION ?? "",
  unitConversionVersion: process.env.SPENDGUARD_UNIT_CONVERSION_VERSION ?? "",
};

// ── Event recording: in-memory array of encodeSse(event) strings, appended
//    in emission order (implementation.md §7). ────────────────────────────
const frames = [];
function emit(event) {
  frames.push(encodeSse(event));
  console.log(`[demo] emitted ${event.name}`);
}

function nowRfc3339() {
  return new Date().toISOString();
}

function rfc3339FromSeconds(seconds) {
  return new Date(seconds * 1000).toISOString();
}

/** Probe the counting stub's `/_count` endpoint; returns the running tally. */
async function readCountingStubHits() {
  const r = await fetch(`${COUNTING_STUB_URL}/_count`);
  if (!r.ok) {
    throw new Error(`counting-stub /_count returned HTTP ${r.status}`);
  }
  const body = await r.json();
  return Number(body.calls);
}

/** Poll the sidecar socket until handshake completes or timeout elapses. */
async function connectWithRetry() {
  const deadline = Date.now() + HANDSHAKE_TIMEOUT_MS;
  let lastErr = "";
  while (Date.now() < deadline) {
    try {
      const client = new SpendGuardClient({
        socketPath: SOCKET_PATH,
        tenantId: TENANT_ID,
        runtimeKind: "ag-ui-events-demo",
      });
      await client.connect();
      await client.handshake();
      console.log(`[demo] handshake ok session_id=${client.sessionId}`);
      return client;
    } catch (err) {
      lastErr = err instanceof Error ? err.message : String(err);
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
  }
  throw new Error(`handshake timeout after ${HANDSHAKE_TIMEOUT_MS}ms: ${lastErr}`);
}

/** Substrate-derived per-call identity bundle — D39 derives nothing itself. */
function newCallIds(client, trigger) {
  const runId = newUuid7();
  const stepId = "llm_call";
  const llmCallId = newUuid7();
  const decisionId = newUuid7();
  const idempotencyKey = deriveIdempotencyKey({
    tenantId: TENANT_ID,
    sessionId: client.sessionId,
    runId,
    stepId,
    llmCallId,
    trigger,
  });
  return { runId, stepId, llmCallId, decisionId, idempotencyKey };
}

/** Step 1 — budget snapshot from the ledger-cross-checked seed env values. */
function runSnapshotStep() {
  console.log("[demo] (1) SNAPSHOT step — seeded budget state");
  const event = buildBudgetSnapshot(
    {
      budgetId: BUDGET_ID,
      windowInstanceId: WINDOW_INSTANCE_ID,
      unit: UNIT_SLUG,
      unitId: UNIT_ID,
      remainingAtomic: OPENING_BALANCE_ATOMIC,
      // True at fresh-stack start (design.md §9.2): nothing reserved,
      // nothing spent — the verify gate's ledger join + SQL gates run
      // against the same fresh stack.
      reservedAtomic: "0",
      spentAtomic: "0",
      asOf: nowRfc3339(),
    },
    { timestampMs: Date.now() },
  );
  emit(event);
}

/** Step 2 — ALLOW: real reserve → provider call → real commitEstimated. */
async function runAllowStep(client) {
  console.log("[demo] (2) ALLOW step — reserve + provider call + commit");
  const ids = newCallIds(client, "LLM_CALL_PRE");
  const outcome = await client.reserve({
    trigger: "LLM_CALL_PRE",
    runId: ids.runId,
    stepId: ids.stepId,
    llmCallId: ids.llmCallId,
    decisionId: ids.decisionId,
    route: "ag-ui-events-demo",
    projectedClaims: [
      {
        scopeId: BUDGET_ID,
        amountAtomic: ALLOW_AMOUNT_ATOMIC,
        unit: WIRE_UNIT,
        windowInstanceId: WINDOW_INSTANCE_ID,
      },
    ],
    idempotencyKey: ids.idempotencyKey,
  });
  const reservationId = outcome.reservationIds[0];
  if (reservationId === undefined || reservationId === "") {
    throw new Error("[demo] FATAL: reserve() returned no reservation_id");
  }
  console.log(
    `[demo] (2) reserve ${outcome.decision} decision_id=${outcome.decisionId} ` +
      `reservation_id=${reservationId}`,
  );
  emit(
    buildReservationCreated(
      {
        decisionId: outcome.decisionId,
        reservationId,
        budgetId: BUDGET_ID,
        windowInstanceId: WINDOW_INSTANCE_ID,
        unit: UNIT_SLUG,
        unitId: UNIT_ID,
        amountAtomicReserved: ALLOW_AMOUNT_ATOMIC,
        // ASP decision enum mapping (design.md §5.4): wire CONTINUE → ALLOW,
        // DEGRADE → ALLOW_WITH_CAPS.
        decision: outcome.decision === "DEGRADE" ? "ALLOW_WITH_CAPS" : "ALLOW",
        ttlExpiresAt: rfc3339FromSeconds(outcome.ttlExpiresAtSeconds),
        ...(outcome.reasonCodes.length > 0 ? { reasonCodes: outcome.reasonCodes } : {}),
        ...(outcome.matchedRuleIds.length > 0 ? { matchedRuleIds: outcome.matchedRuleIds } : {}),
        runId: ids.runId,
        llmCallId: ids.llmCallId,
        eventTime: nowRfc3339(),
      },
      { timestampMs: Date.now() },
    ),
  );

  // Provider dispatch — happens AFTER the sidecar reservation (INV-2).
  const r = await fetch(`${COUNTING_STUB_URL}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      model: "gpt-4o-mini",
      messages: [{ role: "user", content: "hello ag_ui_events" }],
    }),
  });
  if (!r.ok) {
    throw new Error(`[demo] FATAL: counting-stub returned HTTP ${r.status}`);
  }
  const providerBody = await r.json();
  const providerEventId = String(providerBody.id ?? "");
  console.log(`[demo] (2) provider call ok provider_event_id=${providerEventId}`);

  await client.commitEstimated({
    runId: ids.runId,
    stepId: ids.stepId,
    llmCallId: ids.llmCallId,
    decisionId: outcome.decisionId,
    reservationId,
    estimatedAmountAtomic: ALLOW_AMOUNT_ATOMIC,
    unit: WIRE_UNIT,
    pricing: PRICING,
    providerEventId,
    outcome: "SUCCESS",
  });
  console.log(`[demo] (2) commitEstimated SUCCESS reservation_id=${reservationId}`);
  emit(
    buildReservationCommitted(
      {
        decisionId: outcome.decisionId,
        reservationId,
        budgetId: BUDGET_ID,
        windowInstanceId: WINDOW_INSTANCE_ID,
        unit: UNIT_SLUG,
        unitId: UNIT_ID,
        amountAtomicEstimated: ALLOW_AMOUNT_ATOMIC,
        outcome: "SUCCESS",
        runId: ids.runId,
        llmCallId: ids.llmCallId,
        eventTime: nowRfc3339(),
      },
      { timestampMs: Date.now() },
    ),
  );
}

/**
 * Map the SDK's decision-typed error class onto the §5.7 `denied_kind`
 * taxonomy. Marker resolution (design.md §5.7 [VERIFY-AT-IMPL], demo-mapping
 * half): the TS `DecisionStopped` error does NOT expose the wire decision
 * enum, so STOP vs STOP_RUN_PROJECTION is not distinguishable here — callers
 * emit "STOP" and the projection nuance stays visible in `reason_codes`
 * (matches the slice-2 Python err-class finding).
 */
function deniedKindFromError(err) {
  switch (err.name) {
    case "DecisionStopped":
      return "STOP";
    case "DecisionSkipped":
      return "SKIP";
    case "ApprovalRequired":
      return "APPROVAL_REQUIRED";
    default:
      return "DENY";
  }
}

/** Step 3 — DENY: over-cap reserve, denied by the sidecar pre-dispatch. */
async function runDenyStep(client) {
  console.log("[demo] (3) DENY step — claim above the seeded hard-cap");
  const ids = newCallIds(client, "LLM_CALL_PRE");
  const preCount = await readCountingStubHits();
  let deniedErr;
  try {
    await client.reserve({
      trigger: "LLM_CALL_PRE",
      runId: ids.runId,
      stepId: ids.stepId,
      llmCallId: ids.llmCallId,
      decisionId: ids.decisionId,
      route: "ag-ui-events-demo",
      projectedClaims: [
        {
          scopeId: BUDGET_ID,
          amountAtomic: DENY_AMOUNT_ATOMIC,
          unit: WIRE_UNIT,
          windowInstanceId: WINDOW_INSTANCE_ID,
        },
      ],
      idempotencyKey: ids.idempotencyKey,
    });
  } catch (err) {
    if (!(err instanceof DecisionDenied)) {
      throw err;
    }
    deniedErr = err;
  }
  if (deniedErr === undefined) {
    throw new Error("[demo] FATAL: DENY step reserve() was NOT denied by the sidecar");
  }
  const postCount = await readCountingStubHits();
  console.log(
    `[demo] (3) ${deniedErr.name} decision_id=${deniedErr.decisionId} ` +
      `reason_codes=${JSON.stringify(deniedErr.reasonCodes)} ` +
      `counter pre=${preCount} post=${postCount}`,
  );
  if (postCount !== preCount) {
    throw new Error(
      `[demo] FATAL: DENY step counter moved pre=${preCount} post=${postCount} ` +
        "(provider was hit even though the sidecar denied the call)",
    );
  }
  // Runner-side proof that enforcement happened at the sidecar, not in
  // AG-UI (design.md §9 step 4 — the log line says exactly that).
  console.log("[demo] deny enforced by sidecar pre-dispatch; AG-UI event is display-only");
  emit(
    buildDecisionDenied(
      {
        decisionId: deniedErr.decisionId,
        deniedKind: deniedKindFromError(deniedErr),
        // Straight off the real RPC error — never fabricated. If the sidecar
        // ever returned an empty array the builder throws (≥ 1 required) and
        // the demo fails honestly.
        reasonCodes: deniedErr.reasonCodes,
        ...(deniedErr.matchedRuleIds.length > 0
          ? { matchedRuleIds: deniedErr.matchedRuleIds }
          : {}),
        budgetId: BUDGET_ID,
        windowInstanceId: WINDOW_INSTANCE_ID,
        unit: UNIT_SLUG,
        unitId: UNIT_ID,
        runId: ids.runId,
        llmCallId: ids.llmCallId,
        eventTime: nowRfc3339(),
      },
      { timestampMs: Date.now() },
    ),
  );
}

/** Step 4 — replay server. Healthy only after all steps succeeded. */
function serve() {
  const server = http.createServer((req, res) => {
    if (req.method === "GET" && req.url === "/healthz") {
      res.writeHead(200, { "Content-Type": "text/plain", "Content-Length": "2" });
      res.end("ok");
      return;
    }
    if (req.method === "GET" && req.url === "/events") {
      const body = frames.join("");
      res.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Content-Length": String(Buffer.byteLength(body)),
      });
      res.end(body);
      return;
    }
    res.writeHead(404, { "Content-Length": "0" });
    res.end();
  });
  server.listen(8077, "0.0.0.0", () => {
    console.log(
      `[demo] ag_ui_events ALL 4 events RECORDED (snapshot + created + committed + denied); ` +
        `serving SSE replay on :8077 (${frames.length} frames)`,
    );
  });
}

async function main() {
  console.log(
    `[demo] ag_ui_events runner: socket=${SOCKET_PATH} tenant=${TENANT_ID} ` +
      `budget=${BUDGET_ID} counting_stub=${COUNTING_STUB_URL}`,
  );
  const client = await connectWithRetry();
  try {
    runSnapshotStep();
    await runAllowStep(client);
    await runDenyStep(client);
  } finally {
    await client.close();
  }
  if (frames.length !== 4) {
    throw new Error(`[demo] FATAL: expected 4 recorded frames, got ${frames.length}`);
  }
  serve();
}

main().catch((err) => {
  console.error(`[demo] FAIL: ${err instanceof Error ? (err.stack ?? err.message) : err}`);
  process.exit(7);
});
