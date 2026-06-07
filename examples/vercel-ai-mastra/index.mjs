// D06 SLICE 7 — Node demo runner for the @spendguard/vercel-ai middleware.
//
// Drives 3 Vercel AI SDK v4 `generateText` / `streamText` invocations
// through `wrapLanguageModel({model, middleware: createSpendGuardMiddleware(...)})`
// against the in-network counting-stub upstream (OpenAI-shape responses
// with non-zero usage.completionTokens).
//
//   step 1 ALLOW  — small message within budget → counter +1, SUCCESS commit
//   step 2 DENY   — `spendguard_estimate_override` (extra body) blows past
//                   the 1B-atomic hard-cap; sidecar contract evaluator emits
//                   SPENDGUARD_DENY pre-call; middleware `transformParams`
//                   throws DecisionDenied; generateText call halts BEFORE
//                   the counting-stub fetch fires → counter UNCHANGED.
//   step 3 STREAM — uses `streamText` against the same wrapped model;
//                   PRE fires once at stream open, POST commits once at
//                   stream end via the TransformStream `flush()` hook.
//
// Success line per D11/6 §6.7 LOCKED spelling pattern (mirrors the
// langchain_ts demo so the CI grep stays uniform across adapters):
//   `[demo] vercel_ai_mastra ALL 3 steps PASS (ALLOW + DENY + STREAM)`
//
// **Mastra coverage proof**: this demo additionally imports the
// `@spendguard/vercel-ai/mastra` subpath alias and asserts that
// `createSpendGuardLanguageMiddleware === createSpendGuardMiddleware` —
// per D06 review-standards §1.6 LOCK. A Mastra `Agent.generate()` call
// resolves down to `generateText` from `ai`, so the wrapped model below
// already exercises the Mastra path byte-for-byte. The runtime equality
// check confirms the function-reference alias is real (not a wrapper).
//
// The script is launched by deploy/demo/demo/run_demo.py::run_vercel_ai_mastra_mode
// (or directly via `node index.mjs` inside the vercel-ai-mastra-runner
// container). It expects:
//   SPENDGUARD_SIDECAR_UDS  → /var/run/spendguard/adapter.sock
//   SPENDGUARD_TENANT_ID    → 00000000-0000-4000-8000-000000000001
//   SPENDGUARD_BUDGET_ID    → 44444444-4444-4444-8444-444444444444
//   OPENAI_BASE_URL         → http://counting-stub:8765/v1
//   OPENAI_API_KEY          → demo-counting-stub-no-real-key
//   SPENDGUARD_COUNTING_STUB_URL → http://counting-stub:8765 (for /_count probe)

import { generateText, streamText, wrapLanguageModel } from "ai";
import { SpendGuardClient } from "@spendguard/sdk";
import {
  createSpendGuardMiddleware,
  DecisionDenied,
} from "@spendguard/vercel-ai";
import { createSpendGuardLanguageMiddleware } from "@spendguard/vercel-ai/mastra";

const SOCKET_PATH = process.env.SPENDGUARD_SIDECAR_UDS ?? "/var/run/spendguard/adapter.sock";
const TENANT_ID = process.env.SPENDGUARD_TENANT_ID ?? "00000000-0000-4000-8000-000000000001";
const BUDGET_ID = process.env.SPENDGUARD_BUDGET_ID;
const COUNTING_STUB_URL =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const OPENAI_BASE_URL = process.env.OPENAI_BASE_URL ?? `${COUNTING_STUB_URL}/v1`;

const HANDSHAKE_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_HANDSHAKE_TIMEOUT_MS ?? "30000",
  10,
);

// ── Mastra-alias parity check ────────────────────────────────────────────
//
// Per D06 review-standards §1.6 LOCK: the `/mastra` subpath alias MUST
// be a function-reference alias (NOT a wrapper / NOT a copy) of
// `createSpendGuardMiddleware`. Asserting strict equality at boot time
// proves the build's `tsup` config kept the entries linked.
if (createSpendGuardLanguageMiddleware !== createSpendGuardMiddleware) {
  console.error(
    "[demo] FATAL: @spendguard/vercel-ai/mastra alias diverged from root export — " +
      "expected createSpendGuardLanguageMiddleware === createSpendGuardMiddleware",
  );
  process.exit(8);
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
        runtimeKind: "vercel-ai-js",
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

// ── Counting-stub-backed LanguageModelV1 ────────────────────────────────
//
// A minimal `LanguageModelV1` that posts each `doGenerate` /
// `doStream` call to `${OPENAI_BASE_URL}/chat/completions` (the
// counting-stub endpoint). The counting-stub returns the
// OpenAI-shape chat.completion envelope including
// `usage.{prompt_tokens, completion_tokens}` — the middleware's
// `extractUsageFromGenerate` accepts both camelCase AND snake_case
// (see middleware.ts `extractUsageFromBag`) so the snake_case relay
// from the counting-stub flows through cleanly.
//
// **Why not `@ai-sdk/openai`?**: the official adapter pulls auth +
// retry + structured-output glue that would dwarf the demo. The
// SLICE 7 acceptance criterion is "Vercel AI SDK middleware + the
// counting-stub upstream prove ALLOW/DENY/STREAM end-to-end" — this
// hand-rolled model has the exact `LanguageModelV1` surface
// `wrapLanguageModel` consumes, so the middleware exercises its real
// `transformParams` + `wrapGenerate` + `wrapStream` paths identically
// to what a `@ai-sdk/openai` install would surface.
function makeCountingStubModel({ extraBody = undefined } = {}) {
  const modelId = "gpt-4o-mini";
  return {
    specificationVersion: "v1",
    provider: "openai.chat",
    modelId,
    defaultObjectGenerationMode: "tool",
    supportsImageUrls: true,
    supportsStructuredOutputs: true,

    async doGenerate(options) {
      const body = buildOpenAiBody(options, extraBody, { stream: false });
      const r = await fetch(`${OPENAI_BASE_URL}/chat/completions`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          // Counting stub does not validate but the AI SDK rule of thumb
          // is "send something — never bare":
          Authorization: "Bearer demo-counting-stub-no-real-key",
        },
        body: JSON.stringify(body),
      });
      if (!r.ok) {
        throw new Error(`counting-stub returned HTTP ${r.status}`);
      }
      const payload = await r.json();
      const choice = payload.choices?.[0]?.message ?? {};
      const usage = payload.usage ?? {};
      return {
        text: typeof choice.content === "string" ? choice.content : "",
        // Map OpenAI snake_case → AI SDK canonical camelCase. The
        // SpendGuard middleware ALSO accepts snake_case via the
        // wrapper's `extractUsageFromBag` defensive path; we map here
        // so the rest of the AI SDK pipeline (`generateText`'s own
        // usage accounting) sees the canonical shape.
        usage: {
          promptTokens: Number(usage.prompt_tokens ?? 0),
          completionTokens: Number(usage.completion_tokens ?? 0),
        },
        finishReason: "stop",
        rawCall: {
          rawPrompt: options.prompt,
          rawSettings: { model: modelId, temperature: options.temperature ?? 0 },
        },
        rawResponse: { headers: { "x-mock-provider": "counting-stub" } },
        warnings: [],
      };
    },

    async doStream(options) {
      // The counting-stub's HTTP surface returns a single JSON payload
      // per POST — it does NOT serve SSE. For the SLICE 7 streaming
      // demo we issue a single fetch + synthesise the
      // `LanguageModelV1StreamPart` sequence in-process from the
      // returned payload. This is the exact shape `@ai-sdk/openai`
      // produces after its own SSE-to-part transformer runs.
      const body = buildOpenAiBody(options, extraBody, { stream: false });
      const r = await fetch(`${OPENAI_BASE_URL}/chat/completions`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: "Bearer demo-counting-stub-no-real-key",
        },
        body: JSON.stringify(body),
      });
      if (!r.ok) {
        throw new Error(`counting-stub returned HTTP ${r.status}`);
      }
      const payload = await r.json();
      const text =
        typeof payload.choices?.[0]?.message?.content === "string"
          ? payload.choices[0].message.content
          : "";
      const usage = payload.usage ?? {};
      const stream = new ReadableStream({
        start(controller) {
          if (text.length > 0) {
            controller.enqueue({ type: "text-delta", textDelta: text });
          }
          controller.enqueue({
            type: "finish",
            finishReason: "stop",
            usage: {
              promptTokens: Number(usage.prompt_tokens ?? 0),
              completionTokens: Number(usage.completion_tokens ?? 0),
            },
          });
          controller.close();
        },
      });
      return {
        stream,
        rawCall: {
          rawPrompt: options.prompt,
          rawSettings: { model: modelId, temperature: options.temperature ?? 0 },
        },
        rawResponse: { headers: { "x-mock-provider": "counting-stub" } },
        warnings: [],
      };
    },
  };
}

/** Build the OpenAI chat.completions HTTP body the counting-stub expects. */
function buildOpenAiBody(options, extraBody, { stream }) {
  const messages = [];
  for (const msg of options.prompt) {
    if (msg.role === "system") {
      messages.push({ role: "system", content: msg.content });
      continue;
    }
    if (msg.role === "tool") {
      continue;
    }
    const text = (msg.content || [])
      .filter((p) => p.type === "text")
      .map((p) => p.text)
      .join("");
    messages.push({ role: msg.role, content: text });
  }
  return {
    model: "gpt-4o-mini",
    messages,
    stream,
    ...(extraBody ?? {}),
  };
}

/**
 * Build a `wrapLanguageModel` instance using the SpendGuard middleware
 * routed to the demo seed's budget. Routing the projected claim to the
 * demo `SPENDGUARD_BUDGET_ID` lets the sidecar contract evaluator
 * match the right hard-cap rule on the DENY step.
 */
function buildWrappedModel(client, { extraBody = undefined } = {}) {
  const middleware = createSpendGuardMiddleware({
    client,
    tenantId: TENANT_ID,
    ...(BUDGET_ID ? { budgetId: BUDGET_ID } : {}),
  });
  return wrapLanguageModel({
    model: makeCountingStubModel({ extraBody }),
    middleware,
  });
}

/**
 * Drive one ALLOW invocation via `generateText` (non-streaming path).
 * Returns `{ preCount, postCount, content }`.
 * `transformParams` fires PRE → `client.reserve` ALLOW →
 * `doGenerate()` HTTP call → counting-stub +1 → `wrapGenerate`
 * SUCCESS commit.
 */
async function runAllowStep(client) {
  console.log("[demo] (1) ALLOW step — invoking generateText within budget");
  const model = buildWrappedModel(client);
  const preCount = await readCountingStubHits();
  const res = await generateText({
    model,
    prompt: "hello vercel_ai_mastra",
  });
  const postCount = await readCountingStubHits();
  const content = typeof res.text === "string" ? res.text : JSON.stringify(res.text);
  console.log(
    `[demo] (1) ALLOW reply=${JSON.stringify(content).slice(0, 80)} ` +
      `counter pre=${preCount} post=${postCount} ` +
      `usage={promptTokens:${res.usage?.promptTokens},completionTokens:${res.usage?.completionTokens}}`,
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
 * SPENDGUARD_DENY pre-call; the middleware's `transformParams` throws
 * `DecisionDenied`; `generateText` halts BEFORE the counting-stub is hit.
 */
async function runDenyStep(client) {
  console.log("[demo] (2) DENY step — forcing hard-cap overflow");
  const preCount = await readCountingStubHits();
  const model = buildWrappedModel(client, {
    extraBody: { spendguard_estimate_override: "2000000000" },
  });
  let denied = false;
  let errKind = "";
  try {
    await generateText({
      model,
      prompt: "trigger vercel_ai_mastra deny",
    });
  } catch (err) {
    denied = true;
    errKind = err instanceof Error ? (err.name ?? "Error") : "non-Error";
    const isTypedDeny = err instanceof DecisionDenied;
    console.log(
      `[demo] (2) DENY caught ${errKind} (instanceof DecisionDenied=${isTypedDeny}): ` +
        `${err instanceof Error ? err.message : err}`,
    );
  }
  const postCount = await readCountingStubHits();
  console.log(
    `[demo] (2) DENY counter pre=${preCount} post=${postCount} threw=${denied} kind=${errKind}`,
  );
  if (!denied) {
    throw new Error("[demo] FATAL: DENY step did NOT raise — middleware swallowed the deny");
  }
  if (postCount !== preCount) {
    throw new Error(
      `[demo] FATAL: DENY step counter changed pre=${preCount} post=${postCount} ` +
        "(counting-stub was hit even though SpendGuard should have blocked the call)",
    );
  }
}

/**
 * Drive one STREAM invocation via `streamText`. The middleware's
 * `wrapStream` instruments the `ReadableStream` so the SUCCESS commit
 * fires after the consumer drains the final `finish` part. Counter +1
 * (one upstream counting-stub call).
 */
async function runStreamStep(client) {
  console.log("[demo] (3) STREAM step — streaming chunks within budget");
  const model = buildWrappedModel(client);
  const preCount = await readCountingStubHits();
  const result = streamText({
    model,
    prompt: "stream vercel_ai_mastra",
  });
  let chunkCount = 0;
  let collected = "";
  for await (const delta of result.textStream) {
    chunkCount += 1;
    collected += delta;
  }
  const final = await result.usage;
  const postCount = await readCountingStubHits();
  console.log(
    `[demo] (3) STREAM chunks=${chunkCount} ` +
      `text=${JSON.stringify(collected).slice(0, 60)} ` +
      `counter pre=${preCount} post=${postCount} ` +
      `usage={promptTokens:${final?.promptTokens},completionTokens:${final?.completionTokens}}`,
  );
  if (postCount !== preCount + 1) {
    throw new Error(
      `[demo] FATAL STREAM: counting-stub hit pre=${preCount} post=${postCount} (expected +1)`,
    );
  }
}

async function main() {
  console.log(
    `[demo] vercel_ai_mastra driver: socket=${SOCKET_PATH} ` +
      `tenant=${TENANT_ID} openai_base=${OPENAI_BASE_URL}`,
  );
  console.log(
    `[demo] mastra alias parity: createSpendGuardLanguageMiddleware === createSpendGuardMiddleware`,
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
      console.log(
        "[demo] vercel_ai_mastra ALL 3 steps PASS (ALLOW + DENY + STREAM)",
      );
    }
  } finally {
    await client.close();
  }
}

main().catch((err) => {
  console.error(`[demo] FAIL: ${err instanceof Error ? err.stack ?? err.message : err}`);
  process.exit(7);
});
