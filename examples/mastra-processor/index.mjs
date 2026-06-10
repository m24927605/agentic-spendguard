// COV_D38_05 — Node demo runner for the @spendguard/mastra SpendGuardProcessor.
//
// Drives 3 real `@mastra/core` Agent invocations with the SpendGuardProcessor
// mounted via `inputProcessors` (V5 pin) + `outputProcessors` (the §6.1
// backstop commit only fires for output-mounted instances — V4 pin), against
// the in-network counting-stub upstream (OpenAI-shape responses with
// non-zero usage tokens).
//
//   step 1 ALLOW  — `agent.generate(...)` small prompt within budget →
//                   counter +1, SUCCESS commit (estimate = usage token sum,
//                   design §6.7 dated amendment 2026-06-10).
//   step 2 DENY   — second SpendGuardProcessor whose `claimEstimator`
//                   projects 2_000_000_000 atomic (> the demo contract's
//                   1B-atomic hard cap); sidecar denies pre-call; the step
//                   aborts BEFORE the counting-stub fetch fires →
//                   counter UNCHANGED (design §7 fail-closed, TA-04).
//   step 3 STREAM — `agent.stream(...)` drained; whole-step bracket
//                   (design §8): exactly one reserve at step open + one
//                   commit after stream end; counter +1.
//
// Success line per D11/6 §6.7 LOCKED spelling pattern (design §10):
//   `[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)`
//
// ── [VERIFY-AT-IMPL: V6] PINNED (COV_D38_05, @mastra/core 1.41.0) ─────────
// Question: does the model-router string path honor a base-URL override
// (env or per-provider config) for `"openai/..."`?
// PIN: **NO — LOCKED explicit-instance fallback** (second pre-declared
// alternative, design §10/§12). Empirical probe against the installed
// package (node script, local chat/completions-only stub on :8765,
// OPENAI_BASE_URL=http://127.0.0.1:8765/v1):
//   - the router string resolves through ModelsDevGateway →
//     `createOpenAI({apiKey, headers}).responses(modelId)` (vendored
//     @ai-sdk/openai 2.0.106); the vendored provider DOES read
//     `OPENAI_BASE_URL` for its base URL, BUT `.responses()` speaks the
//     OpenAI **Responses API** — the probe's stub received
//     `POST /v1/responses` (verbatim hit log: ["POST /v1/responses"]) and
//     the call failed 404.
//   - the counting-stub is a LOCKED verbatim copy serving ONLY
//     `/v1/chat/completions` (+ `/_count`), so the router path cannot
//     reach it; teaching the stub /responses would break the per-overlay
//     verbatim-copy convention, and re-pointing the router at a different
//     wire shape would be a third wiring (forbidden by design §12 V6 row).
// Consequence: this runner uses the LOCKED fallback — an explicit
// counting-stub-backed `LanguageModelV2` instance with its base URL at the
// stub (same hand-rolled-model convention as examples/vercel-ai-mastra/
// `makeCountingStubModel`, lifted to the v2 spec `@mastra/core` 1.41
// accepts). The router-path ENFORCEMENT claim is carried by TP-22
// (sdk/typescript-mastra/tests/mastraIntegration.test.ts): the processor
// mounts on a model-router-string Agent and `processInputStep` fires —
// the Processor attach point is identical for both model sources.
//
// The script is launched inside the mastra-processor-runner container
// (deploy/demo/mastra_processor/docker-compose.yaml). It expects:
//   SPENDGUARD_SIDECAR_UDS         → /var/run/spendguard/adapter.sock
//   SPENDGUARD_TENANT_ID           → 00000000-0000-4000-8000-000000000001
//   SPENDGUARD_BUDGET_ID           → 44444444-4444-4444-8444-444444444444
//   SPENDGUARD_WINDOW_INSTANCE_ID  → 55555555-5555-4555-8555-555555555555
//   SPENDGUARD_UNIT_ID             → 66666666-6666-4666-8666-666666666666
//   OPENAI_BASE_URL                → http://counting-stub:8765/v1
//   SPENDGUARD_COUNTING_STUB_URL   → http://counting-stub:8765 (/_count probe)

import { Agent } from "@mastra/core/agent";
import { SpendGuardClient } from "@spendguard/sdk";
import { DecisionDenied, SpendGuardProcessor } from "@spendguard/mastra";

const SOCKET_PATH = process.env.SPENDGUARD_SIDECAR_UDS ?? "/var/run/spendguard/adapter.sock";
const TENANT_ID = process.env.SPENDGUARD_TENANT_ID ?? "00000000-0000-4000-8000-000000000001";
const BUDGET_ID = process.env.SPENDGUARD_BUDGET_ID ?? "44444444-4444-4444-8444-444444444444";
// HARDEN_D05_UR / HARDEN_D05_WI — day-1 unitId + windowInstanceId threading.
// `SpendGuardProcessorOptions` carries `unitId`; `windowInstanceId` rides the
// estimator claims (TP-17 family: estimator claims forward verbatim onto the
// reserve wire). The ledger rejects ledger-backed reserves whose claims omit
// either (`INVALID_REQUEST: claim[0].unit.unit_id empty` /
// `claim[0].window_instance_id empty`) — so the demo threads BOTH through a
// custom claimEstimator on every step.
const UNIT_ID = process.env.SPENDGUARD_UNIT_ID ?? "";
const WINDOW_INSTANCE_ID = process.env.SPENDGUARD_WINDOW_INSTANCE_ID ?? "";
const COUNTING_STUB_URL =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const OPENAI_BASE_URL = process.env.OPENAI_BASE_URL ?? `${COUNTING_STUB_URL}/v1`;

const HANDSHAKE_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_HANDSHAKE_TIMEOUT_MS ?? "30000",
  10,
);

// HARDEN_D05_WI — pricing freeze tuple repeated on the commit path. Same
// env convention as the sibling demos (vercel_ai_mastra / langchain_ts):
// version + snapshot hash hex (from bundles runtime.env, sourced by the
// container entrypoint) + fx + unit-conversion versions.
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

// Demo claim amounts (atomic): ALLOW/STREAM stay far under the seeded
// 1B-atomic hard cap; DENY blows past it (design §10 step 2).
const ALLOW_AMOUNT_ATOMIC = "100000";
const DENY_AMOUNT_ATOMIC = "2000000000";

/** Probe the counting stub's `/_count` endpoint; returns the running tally. */
async function readCountingStubHits() {
  const r = await fetch(`${COUNTING_STUB_URL}/_count`);
  if (!r.ok) {
    throw new Error(`counting-stub /_count returned HTTP ${r.status}`);
  }
  const body = await r.json();
  return Number(body.calls);
}

/** Sleep for `ms` milliseconds (Promise wrapper around setTimeout). */
function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
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
        runtimeKind: "mastra-js",
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

/**
 * Instrument the substrate client so the runner can assert reserve/commit
 * counts per step (design §10 step 3: "one reserve at step open, one commit
 * after stream end"). Count-only passthrough — the pricing freeze rides the
 * adapter's `pricing` option (design §6.7 amendment #3, 2026-06-11; the
 * earlier client-boundary pricing rewrite found live by this demo is gone).
 */
function instrumentClient(client) {
  const counters = { reserve: 0, commit: 0 };
  const baseReserve = client.reserve.bind(client);
  const baseCommit = client.commitEstimated.bind(client);
  client.reserve = async (req) => {
    counters.reserve += 1;
    return baseReserve(req);
  };
  client.commitEstimated = async (req) => {
    counters.commit += 1;
    return baseCommit(req);
  };
  return counters;
}

// ── Counting-stub-backed LanguageModelV2 (V6 LOCKED fallback) ────────────
//
// A minimal `LanguageModelV2` (`specificationVersion: "v2"` — one of the
// two model specs the installed `@mastra/core` 1.41.0 agent loop accepts)
// that posts each `doGenerate` / `doStream` call to
// `${OPENAI_BASE_URL}/chat/completions` (the counting-stub endpoint) and
// maps the OpenAI-shape envelope onto v2 content parts / stream parts.
// Same hand-rolled-model convention as examples/vercel-ai-mastra/index.mjs
// `makeCountingStubModel` (which is v1-spec for the AI SDK v4 demo); the
// counting-stub serves a single JSON payload per POST (no SSE), so
// `doStream` issues one fetch and synthesizes the v2 stream parts —
// exactly the post-SSE-transform shape a real provider adapter emits.
function makeCountingStubModel() {
  const modelId = "gpt-4o-mini";

  async function callStub(prompt) {
    const messages = [];
    for (const msg of Array.isArray(prompt) ? prompt : []) {
      if (msg.role === "system") {
        messages.push({ role: "system", content: msg.content });
        continue;
      }
      if (msg.role === "tool") {
        continue;
      }
      const text = (Array.isArray(msg.content) ? msg.content : [])
        .filter((p) => p.type === "text")
        .map((p) => p.text)
        .join("");
      messages.push({ role: msg.role, content: text });
    }
    const r = await fetch(`${OPENAI_BASE_URL}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        // Counting stub does not validate but never send bare:
        Authorization: "Bearer demo-counting-stub-no-real-key",
      },
      body: JSON.stringify({ model: modelId, messages, stream: false }),
    });
    if (!r.ok) {
      throw new Error(`counting-stub returned HTTP ${r.status}`);
    }
    const payload = await r.json();
    const text =
      typeof payload.choices?.[0]?.message?.content === "string"
        ? payload.choices[0].message.content
        : "";
    const u = payload.usage ?? {};
    return {
      text,
      usage: {
        inputTokens: Number(u.prompt_tokens ?? 0),
        outputTokens: Number(u.completion_tokens ?? 0),
        totalTokens: Number(u.total_tokens ?? 0),
      },
    };
  }

  return {
    specificationVersion: "v2",
    provider: "openai.chat",
    modelId,
    supportedUrls: {},

    async doGenerate(options) {
      const { text, usage } = await callStub(options?.prompt);
      return {
        content: [{ type: "text", text }],
        finishReason: "stop",
        usage,
        warnings: [],
      };
    },

    async doStream(options) {
      const { text, usage } = await callStub(options?.prompt);
      const stream = new ReadableStream({
        start(controller) {
          controller.enqueue({ type: "stream-start", warnings: [] });
          controller.enqueue({ type: "text-start", id: "stub-text-1" });
          // Split the reply so the consumer observes >1 delta chunk.
          const mid = Math.max(1, Math.ceil(text.length / 2));
          for (const piece of [text.slice(0, mid), text.slice(mid)]) {
            if (piece.length > 0) {
              controller.enqueue({ type: "text-delta", id: "stub-text-1", delta: piece });
            }
          }
          controller.enqueue({ type: "text-end", id: "stub-text-1" });
          controller.enqueue({ type: "finish", finishReason: "stop", usage });
          controller.close();
        },
      });
      return { stream, warnings: [] };
    },
  };
}

/**
 * Demo claim projection: scopeId at the demo budget, fixed atomic amount,
 * unit row + window instance from the seeded constants. The estimator's
 * claims forward verbatim onto `ReserveRequest.projectedClaims` (TP-17),
 * which is the only surface that carries `windowInstanceId` (HARDEN_D05_WI).
 */
function makeClaimEstimator(amountAtomic) {
  return () => [
    {
      scopeId: BUDGET_ID,
      amountAtomic,
      unit: { unit: "USD_MICROS", denomination: 1, ...(UNIT_ID ? { unitId: UNIT_ID } : {}) },
      ...(WINDOW_INSTANCE_ID ? { windowInstanceId: WINDOW_INSTANCE_ID } : {}),
    },
  ];
}

/**
 * Build a real `@mastra/core` Agent with the SpendGuardProcessor mounted.
 * `inputProcessors` drives the reserve (`processInputStep`) and the SUCCESS
 * commit (`processLLMResponse`); the SAME instance on `outputProcessors`
 * arms the §6.1 backstop commit (`processOutputStep`) — V4/V5 pins.
 */
function buildAgent(client, { id, amountAtomic }) {
  const guard = new SpendGuardProcessor({
    client,
    tenantId: TENANT_ID,
    budgetId: BUDGET_ID,
    ...(UNIT_ID ? { unitId: UNIT_ID } : {}),
    // §6.7 amendment #3 (2026-06-11): the sidecar stamps reservations with
    // the loaded bundle's pricing freeze; the adapter's `pricing` option
    // repeats it on the commit wire (same construction as the sibling
    // vercel_ai_mastra / langchain_ts demos).
    ...(PRICING ? { pricing: PRICING } : {}),
    claimEstimator: makeClaimEstimator(amountAtomic),
  });
  return new Agent({
    id,
    name: id,
    instructions: "You are the SpendGuard mastra_processor demo agent.",
    // V6 PINNED NO (header block): LOCKED explicit-instance fallback —
    // counting-stub-backed LanguageModelV2 with its base URL at the stub.
    // The router-string mount is separately proven by TP-22 (vitest).
    model: makeCountingStubModel(),
    inputProcessors: [guard],
    outputProcessors: [guard],
  });
}

/** Walk `err` and its cause chain looking for the typed deny. */
function findDenialEvidence(err) {
  let node = err;
  const seen = new Set();
  while (node !== null && node !== undefined && !seen.has(node)) {
    seen.add(node);
    if (node instanceof DecisionDenied) {
      return { kind: "instanceof DecisionDenied", message: node.message };
    }
    const name = node instanceof Error ? node.name : undefined;
    if (name === "DecisionDenied" || name === "DecisionStopped" || name === "ApprovalRequired") {
      return { kind: `name=${name}`, message: node.message };
    }
    // V2 PIN residual (gh #181): Mastra 1.41.0's workflow engine serializes
    // processor errors, so the consumer-facing rejection preserves the typed
    // error's MESSAGE but not the class instance. Message-match is the
    // documented consumer contract at the agent boundary.
    const message = node instanceof Error ? node.message : String(node);
    if (/sidecar (DENY|STOP|SKIP|REQUIRE_APPROVAL)|denied|DecisionDenied/i.test(message)) {
      return { kind: "message-match (V2 pin: Mastra serializes step errors)", message };
    }
    node = node instanceof Error ? node.cause : undefined;
  }
  return undefined;
}

/**
 * Step 1 ALLOW — `agent.generate(...)` small prompt within budget.
 * reserve (ALLOW) → counting-stub fetch (+1) → SUCCESS commit.
 */
async function runAllowStep(client, counters) {
  console.log("[demo] (1) ALLOW step — agent.generate within budget");
  const agent = buildAgent(client, {
    id: "mastra-processor-allow",
    amountAtomic: ALLOW_AMOUNT_ATOMIC,
  });
  const preCount = await readCountingStubHits();
  const preReserve = counters.reserve;
  const preCommit = counters.commit;
  const res = await agent.generate("ping mastra_processor");
  const postCount = await readCountingStubHits();
  console.log(
    `[demo] (1) ALLOW reply=${JSON.stringify(res.text).slice(0, 80)} ` +
      `counter pre=${preCount} post=${postCount} ` +
      `reserves=+${counters.reserve - preReserve} commits=+${counters.commit - preCommit}`,
  );
  if (postCount !== preCount + 1) {
    throw new Error(
      `[demo] FATAL: ALLOW counting-stub hit pre=${preCount} post=${postCount} (expected +1)`,
    );
  }
  if (counters.reserve - preReserve !== 1 || counters.commit - preCommit !== 1) {
    throw new Error(
      `[demo] FATAL: ALLOW expected exactly 1 reserve + 1 commit, got ` +
        `reserves=+${counters.reserve - preReserve} commits=+${counters.commit - preCommit}`,
    );
  }
}

/**
 * Step 2 DENY — second SpendGuardProcessor whose claimEstimator projects a
 * claim past the demo contract's 1B-atomic hard cap. The sidecar denies
 * pre-call, `processInputStep` throws (fail-closed, design §7) and the
 * step aborts BEFORE the counting-stub fetch fires (TA-04: zero provider
 * HTTP on DENY).
 */
async function runDenyStep(client, counters) {
  console.log("[demo] (2) DENY step — claimEstimator projects past the 1B-atomic hard cap");
  const agent = buildAgent(client, {
    id: "mastra-processor-deny",
    amountAtomic: DENY_AMOUNT_ATOMIC,
  });
  const preCount = await readCountingStubHits();
  const preCommit = counters.commit;
  let denial;
  let threw = false;
  try {
    await agent.generate("trigger mastra_processor deny");
  } catch (err) {
    threw = true;
    denial = findDenialEvidence(err);
    console.log(
      `[demo] (2) DENY caught ${err instanceof Error ? err.name : "non-Error"}: ` +
        `${err instanceof Error ? err.message : err}`,
    );
  }
  const postCount = await readCountingStubHits();
  console.log(
    `[demo] (2) DENY counter pre=${preCount} post=${postCount} threw=${threw} ` +
      `evidence=${denial ? JSON.stringify(denial.kind) : "NONE"}`,
  );
  if (!threw) {
    throw new Error("[demo] FATAL: DENY step did NOT raise — processor swallowed the deny");
  }
  if (denial === undefined) {
    throw new Error(
      "[demo] FATAL: DENY rejection carries no DecisionDenied evidence (direct, cause chain, or V2 message-match)",
    );
  }
  if (postCount !== preCount) {
    throw new Error(
      `[demo] FATAL: DENY step counter changed pre=${preCount} post=${postCount} ` +
        "(counting-stub was hit even though SpendGuard should have blocked the call)",
    );
  }
  if (counters.commit !== preCommit) {
    throw new Error("[demo] FATAL: DENY step emitted a commit (no reservation exists to settle)");
  }
}

/**
 * Step 3 STREAM — `agent.stream(...)` drained. Whole-step bracket
 * (design §8): exactly one reserve at step open + one commit after the
 * stream ends; counter +1; no per-chunk RPCs.
 */
async function runStreamStep(client, counters) {
  console.log("[demo] (3) STREAM step — agent.stream within budget");
  const agent = buildAgent(client, {
    id: "mastra-processor-stream",
    amountAtomic: ALLOW_AMOUNT_ATOMIC,
  });
  const preCount = await readCountingStubHits();
  const preReserve = counters.reserve;
  const preCommit = counters.commit;
  const out = await agent.stream("count to 3");
  let chunkCount = 0;
  let collected = "";
  for await (const piece of out.textStream) {
    chunkCount += 1;
    collected += piece;
  }
  await out.getFullOutput();
  const postCount = await readCountingStubHits();
  console.log(
    `[demo] (3) STREAM chunks=${chunkCount} text=${JSON.stringify(collected).slice(0, 60)} ` +
      `counter pre=${preCount} post=${postCount} ` +
      `reserves=+${counters.reserve - preReserve} commits=+${counters.commit - preCommit}`,
  );
  if (postCount !== preCount + 1) {
    throw new Error(
      `[demo] FATAL STREAM: counting-stub hit pre=${preCount} post=${postCount} (expected +1)`,
    );
  }
  if (counters.reserve - preReserve !== 1 || counters.commit - preCommit !== 1) {
    throw new Error(
      `[demo] FATAL STREAM: expected exactly 1 reserve + 1 commit for the step, got ` +
        `reserves=+${counters.reserve - preReserve} commits=+${counters.commit - preCommit}`,
    );
  }
}

async function main() {
  console.log(
    `[demo] mastra_processor driver: socket=${SOCKET_PATH} ` +
      `tenant=${TENANT_ID} openai_base=${OPENAI_BASE_URL} node=${process.version}`,
  );
  const client = await connectWithRetry();
  const counters = instrumentClient(client);
  try {
    const step = process.env.SPENDGUARD_DEMO_STEP;
    if (step === "allow") {
      await runAllowStep(client, counters);
    } else if (step === "deny") {
      await runDenyStep(client, counters);
    } else if (step === "stream") {
      await runStreamStep(client, counters);
    } else {
      await runAllowStep(client, counters);
      await runDenyStep(client, counters);
      await runStreamStep(client, counters);
      // D11/6 §6.7 LOCKED — CI grep depends on the exact spelling.
      console.log("[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)");
    }
  } finally {
    await client.close();
  }
}

main().catch((err) => {
  console.error(`[demo] FAIL: ${err instanceof Error ? err.stack ?? err.message : err}`);
  process.exit(7);
});
