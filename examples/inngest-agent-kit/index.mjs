// D29 SLICE 5 — Node demo runner for @spendguard/inngest-agent-kit.
//
// Two modes:
//   --mock   In-process SpendGuardClient + in-process @inngest/agent-kit
//            step.ai stand-in. No sidecar required. Drives 3 calls
//            (ALLOW + DENY + RETRY_DEDUP) and exits 0 on PASS / 7 on FAIL.
//
//   --real   Connect to a SpendGuard sidecar UDS + drive 3 calls through
//            wrapWithSpendGuard(step.ai) inside an Inngest function. The
//            DEMO_MODE=inngest_agent_kit Makefile target wires this up.
//
// 3-step matrix (mirrors D04 / D06 / D08 composite demos):
//   step 1 ALLOW         — small message within budget → counter +1.
//   step 2 DENY          — `spendguard_estimate_override` blows past
//                          hard-cap → sidecar emits SPENDGUARD_DENY →
//                          wrapWithSpendGuard's reserve() throws → inner
//                          step.ai.infer NEVER fires (counter unchanged).
//   step 3 RETRY_DEDUP   — instead of D04/D06/D08's STREAM step (Inngest
//                          AgentKit's step.ai.infer is non-streaming,
//                          design.md §3 non-goal), D29's headline:
//                          driver re-invokes the SAME step body with the
//                          SAME (runId, step.id, idempotencyKey) and
//                          ATTEMPT=1. With an InMemoryIdempotencyCache,
//                          the second attempt's reserve is short-circuited
//                          to the cached outcome — ONE sidecar reserve
//                          across N attempts. Counter increments by the
//                          number of attempts (2 in --mock, 1+`SPENDGUARD_DEMO_RETRIES`
//                          in --real because the OpenAI HTTP layer still
//                          fires on every step body).
//
// Success line (LOCKED — CI grep depends on the exact spelling, matches
// langchain_ts / vercel_ai_mastra / openai_agents_ts composite
// convention):
//
//     `[demo] inngest_agent_kit ALL 3 steps PASS (ALLOW + DENY + RETRY_DEDUP)`
//
// Launched by:
//   - direct `node index.mjs --mock` for laptop iteration.
//   - deploy/demo/demo/run_demo.py::run_inngest_agent_kit_mode in the
//     `DEMO_MODE=inngest_agent_kit` Makefile target.

import { parseArgs } from "node:util";

// ── Args ───────────────────────────────────────────────────────────────────

const { values } = parseArgs({
  options: {
    mock: { type: "boolean", default: false },
    real: { type: "boolean", default: false },
  },
  strict: false,
});

const useReal = Boolean(values.real);
const useMock = !useReal;

// ── Shared config ──────────────────────────────────────────────────────────

const SOCKET_PATH =
  process.env.SPENDGUARD_SIDECAR_UDS ?? "/var/run/spendguard/adapter.sock";
const TENANT_ID =
  process.env.SPENDGUARD_TENANT_ID ?? "00000000-0000-4000-8000-000000000001";
const BUDGET_ID =
  process.env.SPENDGUARD_BUDGET_ID ?? "44444444-4444-4444-8444-444444444444";
const UNIT_ID = process.env.SPENDGUARD_UNIT_ID;
const WINDOW_INSTANCE_ID = process.env.SPENDGUARD_WINDOW_INSTANCE_ID;
// HARDEN_D05_WI — pricing freeze tuple repeated on the commit path. Same
// env convention as the Python demos (run_demo.py): version + snapshot
// hash hex (from bundles runtime.env) + fx + unit-conversion versions.
const PRICING = process.env.SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX
  ? {
      pricingVersion: process.env.SPENDGUARD_PRICING_VERSION ?? "",
      pricingHash: Uint8Array.from(
        Buffer.from(process.env.SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX, "hex"),
      ),
      fxRateVersion: process.env.SPENDGUARD_FX_RATE_VERSION ?? "",
      unitConversionVersion: process.env.SPENDGUARD_UNIT_CONVERSION_VERSION ?? "",
    }
  : undefined;
const COUNTING_STUB_URL =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const OPENAI_BASE_URL = process.env.OPENAI_BASE_URL ?? `${COUNTING_STUB_URL}/v1`;
const OPENAI_API_KEY = process.env.OPENAI_API_KEY ?? "demo-counting-stub-no-real-key";
const HANDSHAKE_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_HANDSHAKE_TIMEOUT_MS ?? "30000",
  10,
);
const DEMO_RETRIES = Number.parseInt(
  process.env.SPENDGUARD_DEMO_RETRIES ?? "2",
  10,
);

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function readCountingStubHits() {
  const r = await fetch(`${COUNTING_STUB_URL}/_count`);
  if (!r.ok) throw new Error(`counting-stub /_count returned HTTP ${r.status}`);
  const body = await r.json();
  return Number(body.calls);
}

// ── --mock implementation ──────────────────────────────────────────────────

async function mockMain() {
  console.log(
    "[demo] inngest_agent_kit driver: --mock mode (no sidecar, no real Inngest dev runtime)",
  );

  const { wrapWithSpendGuard, DecisionDenied } = await import(
    "@spendguard/inngest-agent-kit"
  );
  const { InMemoryIdempotencyCache } = await import("@spendguard/sdk");

  // In-process SpendGuardClient double — implements the two RPCs the wrap
  // touches plus the `tenantId` getter.
  class MockSpendGuardClient {
    constructor() {
      this.tenantId = TENANT_ID;
      this.reserveCount = 0;
      this.commitCount = 0;
      this.nextDeny = false;
    }
    async reserve(req) {
      this.reserveCount += 1;
      if (this.nextDeny) {
        this.nextDeny = false;
        throw new DecisionDenied("budget exceeded", {
          decisionId: `dec-${this.reserveCount}`,
          reasonCodes: ["BUDGET_EXCEEDED"],
        });
      }
      return {
        decisionId: req.decisionId,
        auditDecisionEventId: `aud-${this.reserveCount}`,
        decision: "CONTINUE",
        mutationPatchJson: "{}",
        effectHash: new Uint8Array(0),
        ledgerTransactionId: `lgr-${this.reserveCount}`,
        reservationIds: [`res-${this.reserveCount}`],
        ttlExpiresAtSeconds: 0,
        reasonCodes: [],
        matchedRuleIds: [],
      };
    }
    async commitEstimated(_req) {
      this.commitCount += 1;
    }
  }

  // Mock step.ai — counts calls + simulates Inngest's retry semantics
  // (same step.id + same idempotencyKey + attempt advances on replay).
  class MockStepAi {
    constructor() {
      this.callCount = 0;
      this.throwOnAttempts = new Set();
    }
    setThrowOnAttempts(attempts) {
      this.throwOnAttempts = new Set(attempts);
    }
    async infer(_name, opts, ctx) {
      this.callCount += 1;
      const attempt = ctx?.step?.attempt ?? 0;
      if (this.throwOnAttempts.has(attempt)) {
        throw new Error(`provider-error-attempt-${attempt}`);
      }
      return {
        id: `chatcmpl-mock-${this.callCount}`,
        usage: { total_tokens: 12 },
        choices: [{ message: { role: "assistant", content: "ok from mock" } }],
        model: opts.model,
      };
    }
    async wrap(_name, fn, ...args) {
      this.callCount += 1;
      return fn(...args);
    }
  }

  function makeCtx({ runId, stepId, attempt, idempotencyKey, eventId }) {
    const step = { id: stepId, attempt };
    if (idempotencyKey !== undefined) step.idempotencyKey = idempotencyKey;
    const ctx = { runId, step };
    if (eventId !== undefined) ctx.eventId = eventId;
    return ctx;
  }

  const client = new MockSpendGuardClient();
  const stepAi = new MockStepAi();
  const cache = new InMemoryIdempotencyCache();
  const sg = wrapWithSpendGuard(stepAi, client, {
    tenantId: TENANT_ID,
    budgetId: BUDGET_ID,
    idempotencyCache: cache,
    claimEstimator: () => [
      {
        scopeId: BUDGET_ID,
        amountAtomic: "1000000",
        unit: { unit: "USD_MICROS", denomination: 1 },
      },
    ],
  });

  // step 1 ALLOW
  console.log("[demo] (1) ALLOW step — small message within budget");
  await sg.infer(
    "call-openai",
    { model: { kind: "openai" }, body: { messages: [{ role: "user", content: "hi" }] } },
    makeCtx({ runId: "run-mock-1", stepId: "step-mock-1", attempt: 0 }),
  );
  if (stepAi.callCount !== 1) {
    console.error(`[demo] FATAL: ALLOW stepAi.callCount=${stepAi.callCount} (expected 1)`);
    process.exit(7);
  }
  if (client.commitCount !== 1) {
    console.error(`[demo] FATAL: ALLOW client.commitCount=${client.commitCount} (expected 1)`);
    process.exit(7);
  }
  console.log("[demo] (1) ALLOW OK — inner called once, commit fired once");

  // step 2 DENY — wrapWithSpendGuard reserve() throws → inner stays at 1.
  console.log("[demo] (2) DENY step — forcing budget overflow");
  client.nextDeny = true;
  let denied = false;
  try {
    await sg.infer(
      "call-openai",
      { model: { kind: "openai" }, body: { messages: [{ role: "user", content: "deny" }] } },
      makeCtx({ runId: "run-mock-2", stepId: "step-mock-2", attempt: 0 }),
    );
  } catch (err) {
    denied = err instanceof DecisionDenied;
  }
  if (!denied) {
    console.error("[demo] FATAL: DENY step did not throw DecisionDenied");
    process.exit(7);
  }
  if (stepAi.callCount !== 1) {
    console.error(
      `[demo] FATAL INV-1.6: DENY did not block inner; stepAi.callCount=${stepAi.callCount} (expected 1)`,
    );
    process.exit(7);
  }
  console.log("[demo] (2) DENY OK — DecisionDenied thrown, stepAi.callCount unchanged");

  // step 3 RETRY_DEDUP — same (runId, step.id, idempotencyKey) replayed
  // with attempt = 0..2. The first attempt throws; the second + third
  // succeed. With the in-process cache supplied, reserve fires EXACTLY
  // ONCE across all 3 attempts even though the provider fires 3 times.
  // This is the D29 headline: retry-safe reserve dedup driven by Inngest's
  // own step identity.
  console.log(
    "[demo] (3) RETRY_DEDUP step — 3 attempts with shared (runId, step.id, idempotencyKey)",
  );
  const baseReserve = client.reserveCount;
  const baseCommit = client.commitCount;
  const baseInfer = stepAi.callCount;
  stepAi.setThrowOnAttempts([0, 1]); // attempts 0 + 1 throw, attempt 2 succeeds.

  const retryArgs = { runId: "run-mock-3", stepId: "step-mock-3", idempotencyKey: "I-key-3" };
  // attempt 0 — throws provider error → PROVIDER_ERROR commit
  try {
    await sg.infer(
      "call-openai",
      { model: { kind: "openai" }, body: { messages: [{ role: "user", content: "retry" }] } },
      makeCtx({ ...retryArgs, attempt: 0 }),
    );
  } catch (err) {
    if (!/provider-error/.test(String(err))) throw err;
  }
  // attempt 1 — throws → PROVIDER_ERROR commit; reserve cache hit
  try {
    await sg.infer(
      "call-openai",
      { model: { kind: "openai" }, body: { messages: [{ role: "user", content: "retry" }] } },
      makeCtx({ ...retryArgs, attempt: 1 }),
    );
  } catch (err) {
    if (!/provider-error/.test(String(err))) throw err;
  }
  // attempt 2 — succeeds; reserve cache hit
  await sg.infer(
    "call-openai",
    { model: { kind: "openai" }, body: { messages: [{ role: "user", content: "retry" }] } },
    makeCtx({ ...retryArgs, attempt: 2 }),
  );
  stepAi.setThrowOnAttempts([]);

  const dedupReserves = client.reserveCount - baseReserve;
  const dedupCommits = client.commitCount - baseCommit;
  const dedupInfers = stepAi.callCount - baseInfer;
  if (dedupReserves !== 1) {
    console.error(
      `[demo] FATAL RETRY_DEDUP: reserveCount delta=${dedupReserves} (expected 1; cache should absorb attempts 1 + 2)`,
    );
    process.exit(7);
  }
  if (dedupCommits !== 3) {
    console.error(
      `[demo] FATAL RETRY_DEDUP: commitCount delta=${dedupCommits} (expected 3; one per attempt)`,
    );
    process.exit(7);
  }
  if (dedupInfers !== 3) {
    console.error(
      `[demo] FATAL RETRY_DEDUP: stepAi.callCount delta=${dedupInfers} (expected 3)`,
    );
    process.exit(7);
  }
  console.log(
    `[demo] (3) RETRY_DEDUP OK — reserves=${dedupReserves} commits=${dedupCommits} provider=${dedupInfers}`,
  );

  console.log("[demo] inngest_agent_kit ALL 3 steps PASS (ALLOW + DENY + RETRY_DEDUP)");
  console.log(
    `[demo] summary: reserveCount=${client.reserveCount} commitCount=${client.commitCount} inferCount=${stepAi.callCount}`,
  );
}

// ── --real implementation ──────────────────────────────────────────────────

async function connectWithRetry(SpendGuardClient) {
  const deadline = Date.now() + HANDSHAKE_TIMEOUT_MS;
  let lastErr = "";
  while (Date.now() < deadline) {
    try {
      const client = new SpendGuardClient({
        socketPath: SOCKET_PATH,
        tenantId: TENANT_ID,
        runtimeKind: "inngest-agent-kit",
      });
      await client.connect();
      await client.handshake();
      console.log(`[demo] handshake ok session_id=${client.sessionId}`);
      return client;
    } catch (err) {
      lastErr = err instanceof Error ? err.message : String(err);
      await sleep(1000);
    }
  }
  throw new Error(`handshake timeout after ${HANDSHAKE_TIMEOUT_MS}ms: ${lastErr}`);
}

async function realMain() {
  console.log(
    `[demo] inngest_agent_kit driver: --real mode socket=${SOCKET_PATH} ` +
      `tenant=${TENANT_ID} openai_base=${OPENAI_BASE_URL} retries=${DEMO_RETRIES}`,
  );

  const { SpendGuardClient, InMemoryIdempotencyCache, newUuid7 } = await import(
    "@spendguard/sdk"
  );
  const { wrapWithSpendGuard, DecisionDenied } = await import(
    "@spendguard/inngest-agent-kit"
  );

  const client = await connectWithRetry(SpendGuardClient);

  try {
    // Build a minimal step.ai stand-in that hits the counting stub via
    // raw fetch — the @inngest/agent-kit `step.ai.infer` is a thin wrapper
    // over the provider HTTP, and shielding the demo from the AgentKit
    // dev-runtime install path keeps the run deterministic in the
    // overlay's offline container.
    const stepAi = {
      async infer(_name, opts) {
        const body = opts?.body ?? { messages: [] };
        const overrideDeny = body?.messages?.some(
          (m) => typeof m?.content === "string" && m.content.includes("trigger-deny"),
        );
        const reqBody = overrideDeny
          ? { ...body, spendguard_estimate_override: "2000000000" }
          : body;
        const res = await fetch(`${OPENAI_BASE_URL}/chat/completions`, {
          method: "POST",
          headers: {
            "content-type": "application/json",
            authorization: `Bearer ${OPENAI_API_KEY}`,
          },
          body: JSON.stringify({ model: "gpt-4o-mini", ...reqBody }),
        });
        if (!res.ok) {
          throw new Error(`provider HTTP ${res.status}`);
        }
        return await res.json();
      },
      async wrap(_name, fn, ...args) {
        return fn(...args);
      },
    };

    const cache = new InMemoryIdempotencyCache();
    // HARDEN_D05_WI / HARDEN_D05_UR — the demo claims carry the seed's
    // canonical-truth `unitId` + `windowInstanceId`, and the wrap repeats
    // the bundles pricing freeze on the commit path.
    const claimEstimator = () => [
      {
        scopeId: BUDGET_ID,
        // µUSD claim against the USD monetary unit (88888888, funded 100000
        // µUSD). Was "1000000" ($1) — exceeds every seeded unit's funding, so
        // the migration-0063 budget floor raises BUDGET_EXHAUSTED (now a
        // fail-closed STOP). ALLOW + the single deduped retry stay well under.
        amountAtomic: "5000",
        unit: {
          unit: "USD_MICROS",
          denomination: 1,
          ...(UNIT_ID ? { unitId: UNIT_ID } : {}),
        },
        ...(WINDOW_INSTANCE_ID ? { windowInstanceId: WINDOW_INSTANCE_ID } : {}),
      },
    ];
    const baseOptions = {
      tenantId: TENANT_ID,
      budgetId: BUDGET_ID,
      idempotencyCache: cache,
      claimEstimator,
      ...(PRICING ? { pricing: PRICING } : {}),
    };
    const sg = wrapWithSpendGuard(stepAi, client, baseOptions);
    // Demo-only: DENY step blows past the seeded 1B hard-cap via the
    // adapter-side estimate override (mirrors Python litellm convention).
    const sgDeny = wrapWithSpendGuard(stepAi, client, {
      ...baseOptions,
      estimateOverrideAtomic: "2000000000",
    });

    function makeCtx({ runId, stepId, attempt, idempotencyKey }) {
      const step = { id: stepId, attempt };
      if (idempotencyKey !== undefined) step.idempotencyKey = idempotencyKey;
      return { runId, step };
    }

    // step 1 ALLOW
    console.log("[demo] (1) ALLOW step — small message within budget");
    const preAllow = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    await sg.infer(
      "call-openai",
      { model: "gpt-4o-mini", body: { messages: [{ role: "user", content: "hi" }] } },
      makeCtx({ runId: newUuid7(), stepId: "step-real-allow", attempt: 0 }),
    );
    const postAllow = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    if (preAllow >= 0 && postAllow !== preAllow + 1) {
      throw new Error(
        `[demo] FATAL ALLOW: counting-stub pre=${preAllow} post=${postAllow} (expected +1)`,
      );
    }

    // step 2 DENY — the body carries a `trigger-deny` marker and the
    // stand-in step.ai forwards `spendguard_estimate_override` to the
    // counting stub; the sidecar contract evaluator emits SPENDGUARD_DENY,
    // wrapWithSpendGuard's reserve() throws → inner stepAi.infer NEVER
    // fires.
    console.log("[demo] (2) DENY step — forcing hard-cap overflow");
    const preDeny = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    let denied = false;
    try {
      await sgDeny.infer(
        "call-openai",
        {
          model: "gpt-4o-mini",
          body: {
            messages: [{ role: "user", content: "trigger-deny" }],
            spendguard_estimate_override: "2000000000",
          },
        },
        makeCtx({ runId: newUuid7(), stepId: "step-real-deny", attempt: 0 }),
      );
    } catch (err) {
      // Structural fail-closed recognition (dual-package hazard): the
      // adapter's DecisionStopped may come from another @spendguard/sdk realm,
      // so `instanceof DecisionDenied` can be false even for a genuine deny;
      // every deny subclass locks `statusCode === 403`, so accept that too.
      denied =
        err instanceof DecisionDenied ||
        (typeof err === "object" && err !== null && err.statusCode === 403);
      console.log(
        `[demo] (2) DENY caught ${err?.name ?? "Error"}: ${err instanceof Error ? err.message : err}`,
      );
    }
    const postDeny = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    if (!denied) {
      throw new Error("[demo] FATAL: DENY step did NOT raise DecisionDenied");
    }
    if (preDeny >= 0 && postDeny !== preDeny) {
      throw new Error(
        `[demo] FATAL DENY: counting-stub pre=${preDeny} post=${postDeny} (expected 0)`,
      );
    }

    // step 3 RETRY_DEDUP — drive 1 + DEMO_RETRIES attempts with the
    // SAME (runId, step.id, idempotencyKey). Each attempt's body fires
    // the upstream HTTP, but the SpendGuard reserve fires EXACTLY ONCE
    // across all attempts thanks to the in-process cache.
    console.log(
      `[demo] (3) RETRY_DEDUP step — 1 + ${DEMO_RETRIES} attempts with shared step identity`,
    );
    const retryRunId = newUuid7();
    const preRetry = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    for (let attempt = 0; attempt <= DEMO_RETRIES; attempt += 1) {
      await sg.infer(
        "call-openai",
        {
          model: "gpt-4o-mini",
          body: { messages: [{ role: "user", content: `retry-attempt-${attempt}` }] },
        },
        makeCtx({
          runId: retryRunId,
          stepId: "step-real-retry",
          attempt,
          idempotencyKey: "I-retry-key",
        }),
      );
    }
    const postRetry = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    // counting-stub counter must rise by `1 + DEMO_RETRIES` because each
    // step body fires the upstream HTTP; the SpendGuard reserve dedup
    // operates above that layer and is verified by the SQL gate.
    if (preRetry >= 0) {
      const delta = postRetry - preRetry;
      if (delta !== 1 + DEMO_RETRIES) {
        throw new Error(
          `[demo] FATAL RETRY_DEDUP: counting-stub delta=${delta} (expected ${1 + DEMO_RETRIES})`,
        );
      }
    }

    console.log("[demo] inngest_agent_kit ALL 3 steps PASS (ALLOW + DENY + RETRY_DEDUP)");
  } finally {
    await client.close();
  }
}

// ── Entry point ────────────────────────────────────────────────────────────

async function main() {
  if (useMock) {
    await mockMain();
  } else {
    await realMain();
  }
}

main().catch((err) => {
  console.error(`[demo] FAIL: ${err instanceof Error ? (err.stack ?? err.message) : err}`);
  process.exit(7);
});
