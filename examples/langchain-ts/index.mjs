// D04 SLICE 5 — Node demo runner for the @spendguard/langchain adapter.
//
// Drives 3 LangChain.js ChatOpenAI invocations against the SpendGuard sidecar
// via SpendGuardCallbackHandler, against the in-network counting-stub
// upstream (OpenAI-shape responses with non-zero usage.completion_tokens).
//
//   step 1 ALLOW  — small message within budget → counter +1, SUCCESS commit
//   step 2 DENY   — `spendguard_estimate_override` (extra body) blows past
//                   the 1B-atomic hard-cap; sidecar contract evaluator emits
//                   SPENDGUARD_DENY pre-call; handler's reserve() throws
//                   DecisionDenied; ChatOpenAI HTTP call never fires →
//                   counter UNCHANGED.
//   step 3 STREAM — `streaming: true` keeps the SSE chunked path; PRE fires
//                   once at stream open, POST commits once at stream end.
//
// Success line per D11/6 §6.7 LOCKED spelling:
//   `[demo] langchain_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)`
//
// The script is launched by deploy/demo/demo/run_demo.py::run_langchain_ts_mode
// (or directly via `node index.mjs` inside the langchain-runner container).
// It expects:
//   SPENDGUARD_SIDECAR_UDS  → /var/run/spendguard/adapter.sock
//   SPENDGUARD_TENANT_ID    → 00000000-0000-4000-8000-000000000001
//   SPENDGUARD_BUDGET_ID    → 44444444-4444-4444-8444-444444444444
//   OPENAI_BASE_URL         → http://counting-stub:8765/v1
//   OPENAI_API_KEY          → demo-counting-stub-no-real-key
//   SPENDGUARD_COUNTING_STUB_URL → http://counting-stub:8765 (for /_count probe)

import { HumanMessage } from "@langchain/core/messages";
import { ChatOpenAI } from "@langchain/openai";
import { SpendGuardCallbackHandler } from "@spendguard/langchain";
import { SpendGuardClient } from "@spendguard/sdk";

const SOCKET_PATH = process.env.SPENDGUARD_SIDECAR_UDS ?? "/var/run/spendguard/adapter.sock";
const TENANT_ID = process.env.SPENDGUARD_TENANT_ID ?? "00000000-0000-4000-8000-000000000001";
const COUNTING_STUB_URL =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const OPENAI_BASE_URL = process.env.OPENAI_BASE_URL ?? `${COUNTING_STUB_URL}/v1`;
// Counting stub does not validate keys; a non-empty string keeps the OpenAI
// SDK happy.
const OPENAI_API_KEY = process.env.OPENAI_API_KEY ?? "demo-counting-stub-no-real-key";

const HANDSHAKE_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_HANDSHAKE_TIMEOUT_MS ?? "30000",
  10,
);

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
        runtimeKind: "langchain-js",
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

/** Build a ChatOpenAI model wired to the counting-stub upstream. */
function makeModel({ handler, streaming = false, extraBody = undefined }) {
  return new ChatOpenAI({
    model: "gpt-4o-mini",
    apiKey: OPENAI_API_KEY,
    streaming,
    callbacks: [handler],
    configuration: { baseURL: OPENAI_BASE_URL },
    // `extraBody` (or `modelKwargs`) lets us forward `spendguard_estimate_override`
    // to the proxy/upstream so the sidecar contract evaluator sees the override
    // and emits SPENDGUARD_DENY for the DENY step.
    ...(extraBody !== undefined ? { modelKwargs: extraBody } : {}),
  });
}

/**
 * Build a `SpendGuardCallbackHandler` wired against the demo seed's budget.
 * Routing the projected claim to the demo `SPENDGUARD_BUDGET_ID` lets the
 * sidecar contract evaluator match the right hard-cap rule on the DENY
 * step. The fuller `unitId` / `windowInstanceId` / pricing override
 * surface design.md §4 anticipates is deferred (see options.ts §SLICE 5
 * deviation #1) — when the TS SDK exposes `unit_id` on `UnitRef` the
 * demo will thread it through here.
 */
function buildHandler(client) {
  return new SpendGuardCallbackHandler({
    client,
    ...(process.env.SPENDGUARD_BUDGET_ID
      ? { budgetId: process.env.SPENDGUARD_BUDGET_ID }
      : {}),
  });
}

/**
 * Drive one ALLOW invocation. Returns `{ preCount, postCount, content }`.
 * `handleChatModelStart` fires PRE → `client.reserve` ALLOW → ChatOpenAI HTTP
 * call → counting-stub +1 → `handleLLMEnd` fires POST → SUCCESS commit.
 */
async function runAllowStep(client) {
  console.log("[demo] (1) ALLOW step — invoking ChatOpenAI within budget");
  const handler = buildHandler(client);
  const preCount = await readCountingStubHits();
  const model = makeModel({ handler });
  const res = await model.invoke([new HumanMessage("hello langchain_ts")]);
  const postCount = await readCountingStubHits();
  const content = typeof res.content === "string" ? res.content : JSON.stringify(res.content);
  console.log(
    `[demo] (1) ALLOW reply=${JSON.stringify(content).slice(0, 80)} ` +
      `counter pre=${preCount} post=${postCount}`,
  );
  if (postCount !== preCount + 1) {
    throw new Error(
      `[demo] FATAL: ALLOW counting-stub hit pre=${preCount} post=${postCount} (expected +1)`,
    );
  }
  return { preCount, postCount, content };
}

/**
 * Drive one DENY invocation. The body carries `spendguard_estimate_override`
 * far above the seeded 1B hard-cap; the sidecar contract evaluator emits
 * SPENDGUARD_DENY pre-call; the handler's reserve() throws DecisionDenied;
 * model.invoke() halts BEFORE the counting-stub is hit.
 */
async function runDenyStep(client) {
  console.log("[demo] (2) DENY step — forcing hard-cap overflow");
  const handler = buildHandler(client);
  const preCount = await readCountingStubHits();
  // `spendguard_estimate_override` is a demo-only opt-in the sidecar bundles
  // recognise — same convention as the litellm_guardrail + envoy_extproc
  // demos. Production bundles never honour it.
  const model = makeModel({
    handler,
    extraBody: { spendguard_estimate_override: "2000000000" },
  });
  let denied = false;
  let errKind = "";
  try {
    await model.invoke([new HumanMessage("trigger langchain_ts deny")]);
  } catch (err) {
    denied = true;
    errKind = err instanceof Error ? (err.name ?? "Error") : "non-Error";
    console.log(`[demo] (2) DENY caught ${errKind}: ${err instanceof Error ? err.message : err}`);
  }
  const postCount = await readCountingStubHits();
  console.log(
    `[demo] (2) DENY counter pre=${preCount} post=${postCount} threw=${denied} kind=${errKind}`,
  );
  if (!denied) {
    throw new Error("[demo] FATAL: DENY step did NOT raise — handler swallowed the deny");
  }
  if (postCount !== preCount) {
    throw new Error(
      `[demo] FATAL: DENY step counter changed pre=${preCount} post=${postCount} ` +
        "(upstream was hit even though SpendGuard should have blocked the call)",
    );
  }
}

/**
 * Drive one STREAM invocation. `streaming: true` keeps LangChain's SSE
 * chunked path; PRE fires once at stream open, POST commits once at stream
 * end. Counter +1 (one upstream call).
 */
async function runStreamStep(client) {
  console.log("[demo] (3) STREAM step — streaming chunks within budget");
  const handler = buildHandler(client);
  const preCount = await readCountingStubHits();
  const model = makeModel({ handler, streaming: true });
  let chunkCount = 0;
  for await (const _chunk of await model.stream([new HumanMessage("stream langchain_ts")])) {
    chunkCount += 1;
  }
  const postCount = await readCountingStubHits();
  console.log(
    `[demo] (3) STREAM chunks=${chunkCount} counter pre=${preCount} post=${postCount}`,
  );
  if (postCount !== preCount + 1) {
    throw new Error(
      `[demo] FATAL STREAM: counting-stub hit pre=${preCount} post=${postCount} (expected +1)`,
    );
  }
}

async function main() {
  console.log(
    `[demo] langchain_ts driver: socket=${SOCKET_PATH} ` +
      `tenant=${TENANT_ID} openai_base=${OPENAI_BASE_URL}`,
  );
  const client = await connectWithRetry();
  try {
    const step = process.env.SPENDGUARD_DEMO_STEP;
    if (step === "allow") {
      await runAllowStep(client);
    } else if (step === "deny") {
      await runDenyStep(client);
    } else if (step === "stream") {
      await runStreamStep(client);
    } else {
      await runAllowStep(client);
      await runDenyStep(client);
      await runStreamStep(client);
      // D11/6 §6.7 LOCKED — CI grep depends on the exact spelling.
      console.log("[demo] langchain_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)");
    }
  } finally {
    await client.close();
  }
}

main().catch((err) => {
  console.error(`[demo] FAIL: ${err instanceof Error ? err.stack ?? err.message : err}`);
  process.exit(7);
});
