// COV_D40A_01 - OpenClaw base-URL recipe smoke runner.
//
// This runner validates the committed OpenClaw provider config fixture
// and then emits the OpenAI-compatible calls that the configured
// provider path sends to its baseUrl. It proves SpendGuard's egress
// proxy behavior locally with a counting upstream stub.

import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";

const CONFIG_PATH =
  process.env.OPENCLAW_CONFIG_PATH ?? "/opt/spendguard/deploy/demo/openclaw_base_url/openclaw.config.json";
const PROVIDER_ID = process.env.OPENCLAW_PROVIDER_ID ?? "spendguard";
const EXPECTED_BASE_URL =
  process.env.OPENCLAW_EXPECTED_BASE_URL ?? "http://egress-proxy:9000/v1";
const COUNTING_STUB_URL =
  process.env.SPENDGUARD_COUNTING_STUB_URL ?? "http://counting-stub:8765";
const REQUEST_TIMEOUT_MS = Number.parseInt(
  process.env.SPENDGUARD_OPENCLAW_REQUEST_TIMEOUT_MS ?? "30000",
  10,
);

function fail(message) {
  throw new Error(`[demo] FATAL: ${message}`);
}

function requireObject(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  return value;
}

function requireString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${label} must be a non-empty string`);
  }
  return value;
}

function normalizeBaseUrl(value) {
  return requireString(value, "provider.baseUrl").replace(/\/+$/, "");
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

function buildRequestHeaders(provider, extra = {}) {
  return {
    Authorization: `Bearer ${provider.apiKey}`,
    "Content-Type": "application/json",
    "X-SpendGuard-Agent-Id": "openclaw-base-url-demo",
    "X-SpendGuard-Run-Id": `openclaw-${randomUUID()}`,
    "X-SpendGuard-Idempotency-Key": `openclaw-${randomUUID()}`,
    ...extra,
  };
}

async function loadAndValidateOpenClawConfig() {
  const raw = await readFile(CONFIG_PATH, "utf8");
  const config = JSON.parse(raw);

  const agents = requireObject(config.agents, "agents");
  const defaults = requireObject(agents.defaults, "agents.defaults");
  const model = requireObject(defaults.model, "agents.defaults.model");
  const primary = requireString(model.primary, "agents.defaults.model.primary");
  const [providerFromRef, modelId] = primary.split("/");
  if (providerFromRef !== PROVIDER_ID || !modelId) {
    fail(
      `primary model must be ${PROVIDER_ID}/<model>, got ${JSON.stringify(primary)}`,
    );
  }

  const models = requireObject(config.models, "models");
  const providers = requireObject(models.providers, "models.providers");
  const provider = requireObject(providers[PROVIDER_ID], `models.providers.${PROVIDER_ID}`);
  const baseUrl = normalizeBaseUrl(provider.baseUrl);
  if (baseUrl !== EXPECTED_BASE_URL) {
    fail(`provider.baseUrl expected ${EXPECTED_BASE_URL}, got ${baseUrl}`);
  }
  if (provider.api !== "openai-completions") {
    fail(`provider.api expected "openai-completions", got ${JSON.stringify(provider.api)}`);
  }
  const apiKey = requireString(provider.apiKey, "provider.apiKey");
  const configuredModels = Array.isArray(provider.models) ? provider.models : [];
  if (!configuredModels.some((m) => m && m.id === modelId)) {
    fail(`provider.models must include id=${modelId}`);
  }

  console.log(
    `[demo] OpenClaw config fixture OK provider=${PROVIDER_ID} model=${modelId} baseUrl=${baseUrl}`,
  );
  return { baseUrl, apiKey, modelId, modelRef: primary };
}

async function runAllowStep(provider) {
  console.log("[demo] (1) ALLOW step - OpenAI-compatible chat through OpenClaw baseUrl");
  const pre = await readCountingStubHits();
  const r = await fetchWithTimeout(`${provider.baseUrl}/chat/completions`, {
    method: "POST",
    headers: buildRequestHeaders(provider),
    body: JSON.stringify({
      model: provider.modelId,
      messages: [{ role: "user", content: "Say hi in two words." }],
      max_tokens: 16,
    }),
  });
  const body = await r.json();
  const post = await readCountingStubHits();
  if (!r.ok) {
    fail(`ALLOW returned HTTP ${r.status}: ${JSON.stringify(body)}`);
  }
  const content = body?.choices?.[0]?.message?.content;
  if (typeof content !== "string" || content.length === 0) {
    fail(`ALLOW response did not include assistant content: ${JSON.stringify(body)}`);
  }
  if (post !== pre + 1) {
    fail(`ALLOW counting-stub expected +1, got pre=${pre} post=${post}`);
  }
  console.log(`[demo] (1) ALLOW OK counter pre=${pre} post=${post}`);
}

async function runDenyStep(provider) {
  console.log("[demo] (2) DENY step - SpendGuard blocks before provider dispatch");
  const pre = await readCountingStubHits();
  const r = await fetchWithTimeout(`${provider.baseUrl}/chat/completions`, {
    method: "POST",
    headers: buildRequestHeaders(provider),
    body: JSON.stringify({
      model: provider.modelId,
      messages: [{ role: "user", content: "Trigger a hard-cap denial." }],
      max_tokens: 256,
    }),
  });
  const text = await r.text();
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    body = { raw: text };
  }
  const post = await readCountingStubHits();
  if (r.status !== 429) {
    fail(`DENY expected HTTP 429, got ${r.status}: ${JSON.stringify(body)}`);
  }
  if (body?.error?.code !== "spendguard_blocked") {
    fail(`DENY expected spendguard_blocked, got ${JSON.stringify(body)}`);
  }
  if (post !== pre) {
    fail(`DENY counting-stub changed pre=${pre} post=${post}`);
  }
  console.log(`[demo] (2) DENY OK counter pre=${pre} post=${post}`);
}

async function runStreamStep(provider) {
  console.log("[demo] (3) STREAM step - SSE through OpenClaw baseUrl");
  const pre = await readCountingStubHits();
  const r = await fetchWithTimeout(`${provider.baseUrl}/chat/completions`, {
    method: "POST",
    headers: buildRequestHeaders(provider),
    body: JSON.stringify({
      model: provider.modelId,
      stream: true,
      messages: [{ role: "user", content: "Stream a short greeting." }],
      max_tokens: 20,
    }),
  });
  if (!r.ok) {
    fail(`STREAM returned HTTP ${r.status}: ${await r.text()}`);
  }
  const contentType = r.headers.get("content-type") ?? "";
  if (!contentType.startsWith("text/event-stream")) {
    fail(`STREAM expected text/event-stream, got ${contentType}`);
  }
  if (!r.body) {
    fail("STREAM response body is missing");
  }

  const reader = r.body.getReader();
  const decoder = new TextDecoder();
  let raw = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) {
      break;
    }
    raw += decoder.decode(value, { stream: true });
  }
  raw += decoder.decode();

  const dataLines = raw
    .split(/\r?\n/)
    .filter((line) => line.startsWith("data: "))
    .map((line) => line.slice("data: ".length));
  if (dataLines.length < 3) {
    fail(`STREAM expected at least 3 data frames, got ${dataLines.length}: ${raw}`);
  }
  if (!dataLines.includes("[DONE]")) {
    fail(`STREAM missing [DONE] sentinel: ${raw}`);
  }
  if (!dataLines.some((line) => line.includes('"total_tokens"'))) {
    fail(`STREAM missing usage.total_tokens frame: ${raw}`);
  }

  const post = await readCountingStubHits();
  if (post !== pre + 1) {
    fail(`STREAM counting-stub expected +1, got pre=${pre} post=${post}`);
  }
  console.log(`[demo] (3) STREAM OK frames=${dataLines.length} counter pre=${pre} post=${post}`);
}

function requireDemoRuntimeEnv() {
  for (const name of [
    "SPENDGUARD_TENANT_ID",
    "SPENDGUARD_BUDGET_ID",
    "SPENDGUARD_WINDOW_INSTANCE_ID",
    "SPENDGUARD_UNIT_ID",
    "SPENDGUARD_PRICING_VERSION",
    "SPENDGUARD_FX_RATE_VERSION",
    "SPENDGUARD_UNIT_CONVERSION_VERSION",
  ]) {
    requireString(process.env[name], name);
  }
}

async function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function main() {
  requireDemoRuntimeEnv();
  const provider = await loadAndValidateOpenClawConfig();
  await runAllowStep(provider);
  await runDenyStep(provider);
  await runStreamStep(provider);
  await sleep(2000);
  console.log("[demo] openclaw_base_url ALL 3 steps PASS (ALLOW + DENY + STREAM)");
}

main().catch((err) => {
  console.error(err instanceof Error ? err.stack ?? err.message : String(err));
  process.exit(7);
});
