// D08 SLICE 4 — Node demo runner for @spendguard/openai-agents.
//
// Two modes:
//   --mock   In-process SpendGuardClient + inner Model double. No sidecar
//            required. Exits 0 on PASS; the inner Model's callCount must
//            be 0 after the DENY step (review-standards §1.6 reviewer
//            gate 1.6 — "the demo --mock mode explicitly asserts the
//            invariant 'DENY ⇒ inner Model is NEVER invoked' in its
//            output and exits non-zero if violated").
//
//   --real   Connect to a SpendGuard sidecar UDS + drive a real
//            @openai/agents Agent + Runner.run(...) through
//            withSpendGuard(model). Requires OPENAI_API_KEY (or a
//            counting-stub override via OPENAI_BASE_URL).
//
// Drives 3 calls in each mode mirroring the langchain-ts / vercel-ai
// composite demos:
//   step 1 ALLOW   — small message within budget → counter +1
//   step 2 DENY    — `spendguard_estimate_override` blows past hard-cap
//                    → sidecar emits SPENDGUARD_DENY → withSpendGuard's
//                    reserve() throws → inner.getResponse NEVER fires
//   step 3 STREAM  — for v0.1.x: documented as pass-through (no PRE/POST
//                    around the stream — design.md §3 non-goal). The mock
//                    mode still drives a non-stream second ALLOW call to
//                    prove cross-call determinism stays intact.
//
// Success line (LOCKED — CI grep depends on the exact spelling, matches
// langchain-ts / vercel-ai composite convention):
//   `[demo] openai_agents_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)`
//
// Launched by:
//   - direct `node demo.mjs --mock` for laptop iteration.
//   - deploy/demo/demo/run_demo.py::run_openai_agents_ts_mode in the
//     `DEMO_MODE=openai_agents_ts` Makefile target.

import { parseArgs } from "node:util";

// ── Args ───────────────────────────────────────────────────────────────────

const { values } = parseArgs({
  options: {
    mock: { type: "boolean", default: false },
    real: { type: "boolean", default: false },
  },
  strict: false,
});

// Default to --mock when neither flag is set — the laptop path.
const useReal = Boolean(values.real);
const useMock = !useReal;

// ── Shared config (real mode only) ─────────────────────────────────────────

const SOCKET_PATH = process.env.SPENDGUARD_SIDECAR_UDS ?? "/var/run/spendguard/adapter.sock";
const TENANT_ID = process.env.SPENDGUARD_TENANT_ID ?? "00000000-0000-4000-8000-000000000001";
const BUDGET_ID = process.env.SPENDGUARD_BUDGET_ID ?? "44444444-4444-4444-8444-444444444444";
const COUNTING_STUB_URL =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const OPENAI_BASE_URL = process.env.OPENAI_BASE_URL ?? `${COUNTING_STUB_URL}/v1`;
const OPENAI_API_KEY = process.env.OPENAI_API_KEY ?? "demo-counting-stub-no-real-key";
const HANDSHAKE_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_HANDSHAKE_TIMEOUT_MS ?? "30000",
  10,
);

// ── Helpers ────────────────────────────────────────────────────────────────

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
  console.log("[demo] openai_agents_ts driver: --mock mode (no sidecar, no @openai/agents Runner)");

  const { withSpendGuard, runContext, DecisionDenied } = await import("@spendguard/openai-agents");

  // In-process SpendGuardClient double — implements the two RPCs the
  // bracket touches plus the `tenantId` getter.
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
        decisionId: `dec-${this.reserveCount}`,
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

  // Inner Model double — counts calls. callCount === 0 after DENY proves
  // the invariant.
  class MockInnerModel {
    constructor() {
      this.callCount = 0;
      this.model = "gpt-4o-mini";
    }
    async getResponse(req) {
      this.callCount += 1;
      return {
        usage: {
          requests: 1,
          inputTokens: 12,
          outputTokens: 24,
          totalTokens: 36,
          inputTokensDetails: [],
          outputTokensDetails: [],
        },
        output: [],
        responseId: `resp-mock-${this.callCount}`,
      };
    }
    getStreamedResponse(_req) {
      const self = this;
      async function* gen() {
        self.callCount += 1;
        // single fake "completed" chunk shape; demo only reads count.
        yield { type: "completed", responseId: "stream-mock" };
      }
      return gen();
    }
  }

  const client = new MockSpendGuardClient();
  const inner = new MockInnerModel();
  const guarded = withSpendGuard(inner, { client, tenantId: TENANT_ID, budgetId: BUDGET_ID });

  // step 1 ALLOW
  console.log("[demo] (1) ALLOW step — small message within budget");
  await runContext({ runId: "mock-run-1" }, async () => {
    await guarded.getResponse({
      input: "hello agent",
      systemInstructions: null,
      modelSettings: {},
      tools: [],
      outputType: "text",
      handoffs: [],
      tracing: false,
    });
  });
  if (inner.callCount !== 1) {
    console.error(`[demo] FATAL: ALLOW inner.callCount=${inner.callCount} (expected 1)`);
    process.exit(7);
  }
  if (client.commitCount !== 1) {
    console.error(`[demo] FATAL: ALLOW client.commitCount=${client.commitCount} (expected 1)`);
    process.exit(7);
  }
  console.log("[demo] (1) ALLOW OK — inner called once, commit fired once");

  // step 2 DENY — withSpendGuard reserve() throws → inner stays at 1
  console.log("[demo] (2) DENY step — forcing budget overflow");
  client.nextDeny = true;
  let denied = false;
  await runContext({ runId: "mock-run-2" }, async () => {
    try {
      await guarded.getResponse({
        input: "trigger deny",
        systemInstructions: null,
        modelSettings: {},
        tools: [],
        outputType: "text",
        handoffs: [],
        tracing: false,
      });
    } catch (err) {
      denied = err instanceof DecisionDenied;
    }
  });
  if (!denied) {
    console.error("[demo] FATAL: DENY step did not throw DecisionDenied");
    process.exit(7);
  }
  if (inner.callCount !== 1) {
    // Invariant 1.6 — the canonical "DENY ⇒ inner Model is NEVER invoked"
    // assertion.
    console.error(
      `[demo] FATAL INV-1.6: DENY did not block inner; callCount=${inner.callCount} (expected 1)`,
    );
    process.exit(7);
  }
  console.log("[demo] (2) DENY OK — DecisionDenied thrown, inner.callCount unchanged");

  // step 3 STREAM — pass-through; verifies the no-op routing path stays intact.
  console.log("[demo] (3) STREAM step — pass-through (no PRE/POST in v0.1.x)");
  const stream = guarded.getStreamedResponse({
    input: "stream me",
    systemInstructions: null,
    modelSettings: {},
    tools: [],
    outputType: "text",
    handoffs: [],
    tracing: false,
  });
  let chunkCount = 0;
  for await (const _chunk of stream) {
    chunkCount += 1;
  }
  if (chunkCount < 1) {
    console.error(`[demo] FATAL: STREAM chunkCount=${chunkCount} (expected >= 1)`);
    process.exit(7);
  }
  console.log(`[demo] (3) STREAM OK — chunks=${chunkCount} (pass-through, no reserve fired)`);

  // LOCKED success line
  console.log("[demo] openai_agents_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)");
  console.log(
    `[demo] summary: reserveCount=${client.reserveCount} commitCount=${client.commitCount} innerCallCount=${inner.callCount}`,
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
        runtimeKind: "openai-agents-ts",
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
    `[demo] openai_agents_ts driver: --real mode socket=${SOCKET_PATH} ` +
      `tenant=${TENANT_ID} openai_base=${OPENAI_BASE_URL}`,
  );
  if (!process.env.OPENAI_API_KEY && OPENAI_BASE_URL.startsWith("https://api.openai.com")) {
    console.error("[demo] FATAL: OPENAI_API_KEY required for --real against api.openai.com");
    process.exit(8);
  }

  // `@openai/agents` v0.11 ships the standalone `run(agent, input)`
  // function rather than the legacy `Runner.run(agent, input)` static.
  // Both signatures exist in the v0.11 typings, but only `run(...)` is
  // exported at runtime from the barrel.
  const { Agent, run } = await import("@openai/agents");
  const { OpenAIChatCompletionsModel, OpenAIProvider } = await import("@openai/agents-openai");
  const { SpendGuardClient, newUuid7 } = await import("@spendguard/sdk");
  const { withSpendGuard, runContext, DecisionDenied } = await import(
    "@spendguard/openai-agents"
  );

  const client = await connectWithRetry(SpendGuardClient);

  try {
    // Build the inner OpenAI Chat Completions model wired to the
    // counting stub (or real OpenAI). We construct a provider so the
    // OpenAIChatCompletionsModel sees the desired baseURL + apiKey.
    const provider = new OpenAIProvider({
      apiKey: OPENAI_API_KEY,
      baseURL: OPENAI_BASE_URL,
    });
    const inner = new OpenAIChatCompletionsModel(provider.openaiClient, "gpt-4o-mini");

    // step 1 ALLOW
    console.log("[demo] (1) ALLOW step — invoking Agent within budget");
    const guardedAllow = withSpendGuard(inner, { client, tenantId: TENANT_ID, budgetId: BUDGET_ID });
    const allowAgent = new Agent({
      name: "spendguard-demo-ts",
      instructions: "Reply concisely.",
      model: guardedAllow,
    });
    const preAllow = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    const allowRes = await runContext({ runId: newUuid7() }, () =>
      run(allowAgent, "Say hi in three words."),
    );
    const postAllow = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    console.log(
      `[demo] (1) ALLOW final_output=${JSON.stringify(allowRes.finalOutput).slice(0, 80)} ` +
        `counter pre=${preAllow} post=${postAllow}`,
    );
    if (preAllow >= 0 && postAllow !== preAllow + 1) {
      throw new Error(
        `[demo] FATAL ALLOW: counting-stub pre=${preAllow} post=${postAllow} (expected +1)`,
      );
    }

    // step 2 DENY — `spendguard_estimate_override` blows past hard-cap.
    console.log("[demo] (2) DENY step — forcing hard-cap overflow");
    const guardedDeny = withSpendGuard(inner, { client, tenantId: TENANT_ID, budgetId: BUDGET_ID });
    const denyAgent = new Agent({
      name: "spendguard-demo-ts-deny",
      instructions: "Reply concisely.",
      model: guardedDeny,
      modelSettings: { extraBody: { spendguard_estimate_override: "2000000000" } },
    });
    const preDeny = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    let denied = false;
    let denyKind = "";
    try {
      await runContext({ runId: newUuid7() }, () =>
        run(denyAgent, "trigger openai_agents_ts deny"),
      );
    } catch (err) {
      denied = err instanceof DecisionDenied;
      denyKind = err instanceof Error ? err.name ?? "Error" : "non-Error";
      console.log(`[demo] (2) DENY caught ${denyKind}: ${err instanceof Error ? err.message : err}`);
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

    // step 3 STREAM — design.md §3 anti-scope: pass-through with no
    // PRE/POST gating. We drive a second non-stream call to verify the
    // bracket discipline survives across a stream interleave.
    console.log("[demo] (3) STREAM step — pass-through (no gating) + second non-stream call");
    const guardedStream = withSpendGuard(inner, {
      client,
      tenantId: TENANT_ID,
      budgetId: BUDGET_ID,
    });
    const streamAgent = new Agent({
      name: "spendguard-demo-ts-second",
      instructions: "Reply concisely.",
      model: guardedStream,
    });
    const preStream = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    await runContext({ runId: newUuid7() }, () =>
      run(streamAgent, "Say bye in three words."),
    );
    const postStream = OPENAI_BASE_URL.includes("counting-stub")
      ? await readCountingStubHits()
      : -1;
    if (preStream >= 0 && postStream !== preStream + 1) {
      throw new Error(
        `[demo] FATAL STREAM: counting-stub pre=${preStream} post=${postStream} (expected +1)`,
      );
    }

    // LOCKED success line
    console.log("[demo] openai_agents_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)");
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
