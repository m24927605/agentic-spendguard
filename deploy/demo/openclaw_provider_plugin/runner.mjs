// COV_D40B_05 - OpenClaw provider-plugin demo runner.
//
// Drives the real @spendguard/openclaw-provider-plugin wrapper against the
// demo sidecar UDS and an in-network OpenAI-compatible counting stub:
//
//   1. ALLOW          reserve -> upstream fetch -> SUCCESS settlement
//   2. DENY           reserve denial -> no upstream fetch
//   3. STREAM         reserve -> async iterable drain -> SUCCESS settlement
//   4. PROVIDER_ERROR reserve -> upstream fetch returns 500 -> release lane
//
// Success line locked by D40b implementation.md §5:
//   [demo] openclaw_provider_plugin ALL 4 steps PASS (ALLOW + DENY + STREAM + PROVIDER_ERROR)

import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";

import { DecisionDenied, SpendGuardClient } from "@spendguard/sdk";
import { createSpendGuardOpenClawProvider } from "@spendguard/openclaw-provider-plugin";

const CONFIG_PATH =
  process.env.OPENCLAW_CONFIG_PATH ??
  "/opt/spendguard/deploy/demo/openclaw_provider_plugin/openclaw.config.json";
const SOCKET_PATH = process.env.SPENDGUARD_SIDECAR_UDS ?? "/var/run/spendguard/adapter.sock";
const TENANT_ID = process.env.SPENDGUARD_TENANT_ID ?? "00000000-0000-4000-8000-000000000001";
const BUDGET_ID = process.env.SPENDGUARD_BUDGET_ID ?? "44444444-4444-4444-8444-444444444444";
const WINDOW_INSTANCE_ID =
  process.env.SPENDGUARD_WINDOW_INSTANCE_ID ?? "55555555-5555-4555-8555-555555555555";
const UNIT_ID = process.env.SPENDGUARD_UNIT_ID ?? "66666666-6666-4666-8666-666666666666";
const COUNTING_STUB_URL =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const HANDSHAKE_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_HANDSHAKE_TIMEOUT_MS ?? "30000",
  10,
);
const REQUEST_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_OPENCLAW_REQUEST_TIMEOUT_MS ?? "30000",
  10,
);

const ALLOW_AMOUNT_ATOMIC = "50";
const DENY_AMOUNT_ATOMIC = "1000";

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

function fail(message) {
  throw new Error(`[demo] FATAL: ${message}`);
}

function requireString(value, name) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${name} must be a non-empty string`);
  }
  return value;
}

function requireObject(value, name) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${name} must be an object`);
  }
  return value;
}

function requireRuntimeEnv() {
  for (const name of [
    "SPENDGUARD_TENANT_ID",
    "SPENDGUARD_BUDGET_ID",
    "SPENDGUARD_WINDOW_INSTANCE_ID",
    "SPENDGUARD_UNIT_ID",
    "SPENDGUARD_PRICING_VERSION",
    "SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX",
    "SPENDGUARD_FX_RATE_VERSION",
    "SPENDGUARD_UNIT_CONVERSION_VERSION",
  ]) {
    requireString(process.env[name], name);
  }
  if (PRICING === undefined) {
    fail("pricing freeze tuple is required");
  }
}

async function loadAndValidateOpenClawPluginConfig() {
  const raw = await readFile(CONFIG_PATH, "utf8");
  const config = JSON.parse(raw);
  const plugins = requireObject(config.plugins, "plugins");
  const plugin = requireObject(
    plugins.spendguardProviderPlugin,
    "plugins.spendguardProviderPlugin",
  );
  const provider = requireObject(plugin.provider, "plugins.spendguardProviderPlugin.provider");
  const sidecar = requireObject(plugin.sidecar, "plugins.spendguardProviderPlugin.sidecar");
  const budget = requireObject(plugin.budget, "plugins.spendguardProviderPlugin.budget");

  const packageName = requireString(plugin.package, "plugin.package");
  const factory = requireString(plugin.factory, "plugin.factory");
  const hook = requireString(plugin.hook, "plugin.hook");
  const mode = requireString(plugin.mode, "plugin.mode");
  const providerId = requireString(provider.id, "plugin.provider.id");

  if (packageName !== "@spendguard/openclaw-provider-plugin") {
    fail(`plugin.package expected @spendguard/openclaw-provider-plugin, got ${packageName}`);
  }
  if (factory !== "createSpendGuardOpenClawProvider") {
    fail(`plugin.factory expected createSpendGuardOpenClawProvider, got ${factory}`);
  }
  if (hook !== "wrapStreamFn") {
    fail(`plugin.hook expected wrapStreamFn, got ${hook}`);
  }
  if (mode !== "in-process-provider-wrapper") {
    fail(`plugin.mode expected in-process-provider-wrapper, got ${mode}`);
  }
  for (const [name, value] of Object.entries({
    "sidecar.socketEnv": sidecar.socketEnv,
    "budget.tenantEnv": budget.tenantEnv,
    "budget.budgetEnv": budget.budgetEnv,
    "budget.windowInstanceEnv": budget.windowInstanceEnv,
    "budget.unitEnv": budget.unitEnv,
    "budget.pricingHashEnv": budget.pricingHashEnv,
  })) {
    requireString(value, name);
  }

  console.log(
    `[demo] OpenClaw provider-plugin config fixture OK provider=${providerId} ` +
      `package=${packageName} hook=${hook} mode=${mode}`,
  );
}

async function fetchWithTimeout(url, init = {}) {
  return fetch(url, {
    ...init,
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
}

async function readCountingStubHits() {
  const r = await fetchWithTimeout(`${COUNTING_STUB_URL}/_count`);
  if (!r.ok) {
    fail(`counting-stub /_count returned HTTP ${r.status}`);
  }
  const body = await r.json();
  const calls = Number(body.calls);
  if (!Number.isFinite(calls)) {
    fail(`counting-stub /_count returned invalid payload ${JSON.stringify(body)}`);
  }
  return calls;
}

async function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function connectWithRetry() {
  const deadline = Date.now() + HANDSHAKE_TIMEOUT_MS;
  let lastErr = "";
  while (Date.now() < deadline) {
    try {
      const client = new SpendGuardClient({
        socketPath: SOCKET_PATH,
        tenantId: TENANT_ID,
        runtimeKind: "openclaw-provider-plugin",
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
  fail(`handshake timeout after ${HANDSHAKE_TIMEOUT_MS}ms: ${lastErr}`);
}

function instrumentClient(client) {
  const counters = { reserve: 0, commit: 0, outcomes: [] };
  const baseReserve = client.reserve.bind(client);
  const baseCommit = client.commitEstimated.bind(client);
  client.reserve = async (req) => {
    counters.reserve += 1;
    return baseReserve(req);
  };
  client.commitEstimated = async (req) => {
    counters.commit += 1;
    counters.outcomes.push(req?.outcome);
    return baseCommit(req);
  };
  return counters;
}

function assertNewOutcomes(counters, startIndex, expected, label) {
  const observed = counters.outcomes.slice(startIndex);
  if (observed.length !== expected.length || observed.some((value, i) => value !== expected[i])) {
    fail(`${label} expected settlement outcomes ${JSON.stringify(expected)}, got ${JSON.stringify(observed)}`);
  }
}

function makeClaimEstimator() {
  return ({ flattenedPrompt }) => {
    const deny = flattenedPrompt.includes("D40B_DENY");
    return [
      {
        scopeId: BUDGET_ID,
        amountAtomic: deny ? DENY_AMOUNT_ATOMIC : ALLOW_AMOUNT_ATOMIC,
        unit: { unit: "USD_MICROS", denomination: 1, unitId: UNIT_ID },
        windowInstanceId: WINDOW_INSTANCE_ID,
      },
    ];
  };
}

function buildProvider(client) {
  return createSpendGuardOpenClawProvider(
    {
      id: "spendguard-demo-upstream-openai",
      label: "SpendGuard demo upstream OpenAI-compatible provider",
      auth: [],
    },
    {
      client,
      tenantId: TENANT_ID,
      budgetId: BUDGET_ID,
      windowInstanceId: WINDOW_INSTANCE_ID,
      unitId: UNIT_ID,
      pricing: PRICING,
      route: "openclaw-provider-plugin-demo",
      claimEstimator: makeClaimEstimator(),
      runIdProvider: (ctx) => ctx?.spendguardRunId ?? `openclaw-provider-plugin-${randomUUID()}`,
    },
  );
}

function makeContext(stepName) {
  return {
    provider: "openai",
    modelId: "gpt-4o-mini",
    spendguardRunId: `openclaw-provider-plugin-${stepName}-${randomUUID()}`,
    streamFn: callCountingStub,
  };
}

async function callCountingStub(request) {
  const r = await fetchWithTimeout(`${COUNTING_STUB_URL}/v1/chat/completions`, {
    method: "POST",
    headers: {
      Authorization: "Bearer demo-counting-stub-no-real-key",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model: "gpt-4o-mini",
      messages: request?.messages ?? [],
      stream: request?.stream === true,
      spendguard_demo_provider_error: request?.spendguardDemoProviderError === true,
    }),
  });

  if (!r.ok) {
    const text = await r.text();
    throw new Error(`provider HTTP ${r.status}: ${text.slice(0, 200)}`);
  }

  if (request?.stream === true) {
    return readOpenAiSse(r);
  }

  return r.json();
}

async function* readOpenAiSse(response) {
  if (!response.body) {
    fail("STREAM response body is missing");
  }
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let pending = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    pending += decoder.decode(value, { stream: true });
    yield* parseSseFrames(pending, false);
    pending = keepTrailingPartialFrame(pending);
  }
  pending += decoder.decode();
  yield* parseSseFrames(pending, true);
}

function* parseSseFrames(buffer, final) {
  const frames = buffer.split(/\r?\n\r?\n/);
  const completeFrames = final ? frames : frames.slice(0, -1);
  for (const frame of completeFrames) {
    for (const line of frame.split(/\r?\n/)) {
      if (!line.startsWith("data: ")) continue;
      const payload = line.slice("data: ".length);
      if (payload === "[DONE]") continue;
      yield JSON.parse(payload);
    }
  }
}

function keepTrailingPartialFrame(buffer) {
  const frames = buffer.split(/\r?\n\r?\n/);
  return frames.at(-1) ?? "";
}

async function callProvider(provider, stepName, request) {
  const streamFn = provider.wrapStreamFn?.(makeContext(stepName));
  if (typeof streamFn !== "function") {
    fail("OpenClaw provider wrapper did not return a stream function");
  }
  return streamFn(request);
}

async function runAllowStep(provider, counters) {
  console.log("[demo] (1) ALLOW step - wrapper reserves before provider dispatch");
  const preCount = await readCountingStubHits();
  const preReserve = counters.reserve;
  const preCommit = counters.commit;
  const preOutcomes = counters.outcomes.length;
  const result = await callProvider(provider, "allow", {
    messages: [{ role: "user", content: "D40B_ALLOW say hi." }],
  });
  const postCount = await readCountingStubHits();
  const content = result?.choices?.[0]?.message?.content;
  if (typeof content !== "string" || content.length === 0) {
    fail(`ALLOW response missing assistant content: ${JSON.stringify(result)}`);
  }
  if (postCount !== preCount + 1) {
    fail(`ALLOW counting-stub expected +1, got pre=${preCount} post=${postCount}`);
  }
  if (counters.reserve - preReserve !== 1 || counters.commit - preCommit !== 1) {
    fail(
      `ALLOW expected exactly 1 reserve + 1 settlement, got reserves=+${
        counters.reserve - preReserve
      } commits=+${counters.commit - preCommit}`,
    );
  }
  assertNewOutcomes(counters, preOutcomes, ["SUCCESS"], "ALLOW");
  console.log(`[demo] (1) ALLOW OK counter pre=${preCount} post=${postCount}`);
}

async function runDenyStep(provider, counters) {
  console.log("[demo] (2) DENY step - sidecar denies before provider dispatch");
  const preCount = await readCountingStubHits();
  const preCommit = counters.commit;
  const preOutcomes = counters.outcomes.length;
  let denial;
  let threw = false;
  try {
    await callProvider(provider, "deny", {
      messages: [{ role: "user", content: "D40B_DENY trigger hard cap." }],
    });
  } catch (err) {
    threw = true;
    denial = findDenialEvidence(err);
    console.log(
      `[demo] (2) DENY caught ${err instanceof Error ? err.name : "non-Error"}: ${
        err instanceof Error ? err.message : String(err)
      }`,
    );
  }
  const postCount = await readCountingStubHits();
  if (!threw) {
    fail("DENY step did not raise");
  }
  if (denial === undefined) {
    fail("DENY rejection carries no DecisionDenied evidence");
  }
  if (postCount !== preCount) {
    fail(`DENY counting-stub changed pre=${preCount} post=${postCount}`);
  }
  if (counters.commit !== preCommit) {
    fail("DENY emitted a settlement even though no provider call happened");
  }
  assertNewOutcomes(counters, preOutcomes, [], "DENY");
  console.log(`[demo] (2) DENY OK counter pre=${preCount} post=${postCount}`);
}

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
    const message = node instanceof Error ? node.message : String(node);
    if (/sidecar (DENY|STOP|SKIP|REQUIRE_APPROVAL)|denied|DecisionDenied/i.test(message)) {
      return { kind: "message-match", message };
    }
    node = node instanceof Error ? node.cause : undefined;
  }
  return undefined;
}

async function runStreamStep(provider, counters) {
  console.log("[demo] (3) STREAM step - async iterable settles once after drain");
  const preCount = await readCountingStubHits();
  const preReserve = counters.reserve;
  const preCommit = counters.commit;
  const preOutcomes = counters.outcomes.length;
  const result = await callProvider(provider, "stream", {
    stream: true,
    messages: [{ role: "user", content: "D40B_STREAM short greeting." }],
  });
  const chunks = [];
  for await (const chunk of result) {
    chunks.push(chunk);
  }
  const postCount = await readCountingStubHits();
  if (chunks.length < 3) {
    fail(`STREAM expected at least 3 chunks, got ${chunks.length}`);
  }
  if (!chunks.some((chunk) => chunk?.usage?.total_tokens === 12)) {
    fail(`STREAM chunks missing usage.total_tokens=12: ${JSON.stringify(chunks)}`);
  }
  if (postCount !== preCount + 1) {
    fail(`STREAM counting-stub expected +1, got pre=${preCount} post=${postCount}`);
  }
  if (counters.reserve - preReserve !== 1 || counters.commit - preCommit !== 1) {
    fail(
      `STREAM expected exactly 1 reserve + 1 settlement, got reserves=+${
        counters.reserve - preReserve
      } commits=+${counters.commit - preCommit}`,
    );
  }
  assertNewOutcomes(counters, preOutcomes, ["SUCCESS"], "STREAM");
  console.log(`[demo] (3) STREAM OK chunks=${chunks.length} counter pre=${preCount} post=${postCount}`);
}

async function runProviderErrorStep(provider, counters) {
  console.log("[demo] (4) PROVIDER_ERROR step - upstream error settles then rethrows");
  const preCount = await readCountingStubHits();
  const preReserve = counters.reserve;
  const preCommit = counters.commit;
  const preOutcomes = counters.outcomes.length;
  let threw = false;
  let message = "";
  try {
    await callProvider(provider, "provider-error", {
      spendguardDemoProviderError: true,
      messages: [{ role: "user", content: "D40B_PROVIDER_ERROR return 500." }],
    });
  } catch (err) {
    threw = true;
    message = err instanceof Error ? err.message : String(err);
    console.log(`[demo] (4) PROVIDER_ERROR caught: ${message}`);
  }
  const postCount = await readCountingStubHits();
  if (!threw || !/provider HTTP 500/.test(message)) {
    fail(`PROVIDER_ERROR did not propagate provider HTTP 500, got ${JSON.stringify(message)}`);
  }
  if (postCount !== preCount + 1) {
    fail(`PROVIDER_ERROR counting-stub expected +1, got pre=${preCount} post=${postCount}`);
  }
  if (counters.reserve - preReserve !== 1 || counters.commit - preCommit !== 1) {
    fail(
      `PROVIDER_ERROR expected exactly 1 reserve + 1 failure settlement, got reserves=+${
        counters.reserve - preReserve
      } commits=+${counters.commit - preCommit}`,
    );
  }
  assertNewOutcomes(counters, preOutcomes, ["PROVIDER_ERROR"], "PROVIDER_ERROR");
  console.log(`[demo] (4) PROVIDER_ERROR OK counter pre=${preCount} post=${postCount}`);
}

async function main() {
  requireRuntimeEnv();
  await loadAndValidateOpenClawPluginConfig();
  console.log(
    `[demo] openclaw_provider_plugin driver: socket=${SOCKET_PATH} tenant=${TENANT_ID} ` +
      `stub=${COUNTING_STUB_URL} node=${process.version}`,
  );
  const client = await connectWithRetry();
  const counters = instrumentClient(client);
  const provider = buildProvider(client);
  try {
    await runAllowStep(provider, counters);
    await runDenyStep(provider, counters);
    await runStreamStep(provider, counters);
    await runProviderErrorStep(provider, counters);
    await sleep(2000);
    console.log(
      "[demo] openclaw_provider_plugin ALL 4 steps PASS (ALLOW + DENY + STREAM + PROVIDER_ERROR)",
    );
  } finally {
    await client.close();
  }
}

main().catch((err) => {
  console.error(`[demo] FAIL: ${err instanceof Error ? err.stack ?? err.message : err}`);
  process.exit(7);
});
