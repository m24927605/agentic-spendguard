import * as sdk from '@botpress/sdk';
import { z, RuntimeError } from '@botpress/sdk';
import { readFileSync } from 'fs';
import { normalize, isAbsolute } from 'path';
import { newUuid7, deriveIdempotencyKey, computePromptHash } from '@spendguard/sdk';

var __defProp = Object.defineProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var ConfigurationObjectSchema = z.object({
  sidecarUrl: z.string().url().describe("HTTPS companion URL (plaintext http:// allowed only for loopback)"),
  spendguardBudgetId: z.string().min(1).describe("UUID of the SpendGuard budget to charge"),
  spendguardWindowInstanceId: z.string().min(1).describe("UUID of the SpendGuard window instance"),
  upstreamProvider: z.enum(["openai", "anthropic", "bedrock"]).describe("Upstream provider Botpress dispatches to"),
  tenantId: z.string().min(1).describe("Operator tenant identifier"),
  tlsCertPath: z.string().min(1).optional().describe("Path to SVID cert PEM"),
  tlsKeyPath: z.string().min(1).optional().describe("Path to SVID key PEM"),
  tlsRootCaPath: z.string().min(1).optional().describe("Path to sidecar CA PEM")
});
var ConfigurationSchema = z.object({
  sidecarUrl: z.string().url().refine(isSecureSidecarUrl, {
    message: "sidecarUrl must be https:// (plaintext http:// is allowed only for loopback hosts 127.0.0.1/::1/localhost)"
  }).describe("HTTPS companion URL (plaintext http:// allowed only for loopback)"),
  spendguardBudgetId: z.string().min(1).describe("UUID of the SpendGuard budget to charge"),
  spendguardWindowInstanceId: z.string().min(1).describe("UUID of the SpendGuard window instance"),
  upstreamProvider: z.enum(["openai", "anthropic", "bedrock"]).describe("Upstream provider Botpress dispatches to"),
  tenantId: z.string().min(1).describe("Operator tenant identifier"),
  tlsCertPath: z.string().min(1).optional().describe("Path to SVID cert PEM"),
  tlsKeyPath: z.string().min(1).optional().describe("Path to SVID key PEM"),
  tlsRootCaPath: z.string().min(1).optional().describe("Path to sidecar CA PEM")
}).refine(
  (cfg) => {
    const present = [cfg.tlsCertPath, cfg.tlsKeyPath, cfg.tlsRootCaPath].filter(
      (p) => p !== void 0
    ).length;
    return present === 0 || present === 3;
  },
  {
    message: "tlsCertPath, tlsKeyPath and tlsRootCaPath must be supplied together (all three) or not at all",
    path: ["tlsCertPath"]
  }
);
var LOOPBACK_HOSTS = /* @__PURE__ */ new Set(["127.0.0.1", "::1", "localhost"]);
function isSecureSidecarUrl(raw) {
  let u;
  try {
    u = new URL(raw);
  } catch {
    return false;
  }
  if (u.protocol === "https:") return true;
  if (u.protocol !== "http:") return false;
  return LOOPBACK_HOSTS.has(u.hostname.toLowerCase());
}
function assertRequiredConfig(cfg) {
  const missing = [];
  if (!cfg.sidecarUrl) missing.push("sidecarUrl");
  if (!cfg.spendguardBudgetId) missing.push("spendguardBudgetId");
  if (!cfg.spendguardWindowInstanceId) missing.push("spendguardWindowInstanceId");
  if (!cfg.upstreamProvider) missing.push("upstreamProvider");
  if (!cfg.tenantId) missing.push("tenantId");
  if (missing.length > 0) {
    throw new SpendGuardConfigError(
      `spendguard:botpress: missing required configuration field(s): ${missing.join(", ")}`
    );
  }
  if (!isSecureSidecarUrl(cfg.sidecarUrl)) {
    throw new SpendGuardConfigError(
      `spendguard:botpress: sidecarUrl must be https:// (plaintext http:// is allowed only for loopback hosts); got ${redactUrlForError(
        cfg.sidecarUrl
      )}`
    );
  }
  const tlsPresent = [cfg.tlsCertPath, cfg.tlsKeyPath, cfg.tlsRootCaPath].filter(
    (p) => p !== void 0 && p !== ""
  ).length;
  if (tlsPresent !== 0 && tlsPresent !== 3) {
    throw new SpendGuardConfigError(
      "spendguard:botpress: tlsCertPath, tlsKeyPath and tlsRootCaPath must be supplied together (all three) or not at all"
    );
  }
}
function redactUrlForError(raw) {
  try {
    const u = new URL(raw);
    return `${u.protocol}//${u.hostname}`;
  } catch {
    return "(invalid sidecarUrl)";
  }
}
var SpendGuardConfigError = class extends Error {
  code = "BUDGET_CONFIG";
  constructor(message) {
    super(message);
    this.name = "SpendGuardConfigError";
  }
};

// src/reservation.ts
var DecisionDenied = class extends Error {
  code = "BUDGET_DENIED";
  reasonCodes;
  constructor(message, reasonCodes = []) {
    super(message);
    this.name = "DecisionDenied";
    this.reasonCodes = reasonCodes;
  }
};
var SidecarUnavailable = class extends Error {
  code = "BUDGET_DEGRADED";
  constructor(message) {
    super(message);
    this.name = "SidecarUnavailable";
  }
};
var SpendGuardReservation = class {
  cfg;
  failOpenDev;
  /** Per-instance HTTP client overrides — used by the unit test
   *  `_mockSidecar.ts` to drive the wire path without a real network
   *  socket. Production runtime uses the global `fetch`. */
  fetchImpl;
  reserveDeadlineMs;
  commitDeadlineMs;
  /** Whether a caller injected a transport (`fetchImpl`). When true the
   *  caller owns TLS and the mTLS dispatcher is not built — this is the
   *  test/seam path (mirrors the Kong Go plugin's `httpStubClient` bypass). */
  transportInjected;
  /** Resolved + traversal-cleaned mTLS PEM paths, or null when no client
   *  certificate is configured. */
  tlsPaths;
  /** Memoised undici mTLS dispatcher promise. Built lazily on first request
   *  so a missing `undici` fails the call (fail closed) rather than the whole
   *  process at import time. */
  dispatcherPromise = null;
  constructor(config, opts = {}) {
    assertRequiredConfig(config);
    this.cfg = config;
    this.failOpenDev = opts.failOpenDevOverride ?? (process.env.SPENDGUARD_BOTPRESS_FAIL_OPEN ?? "").trim() === "1";
    this.transportInjected = opts.fetchImpl !== void 0;
    this.fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
    this.reserveDeadlineMs = opts.reserveDeadlineMs ?? 5e3;
    this.commitDeadlineMs = opts.commitDeadlineMs ?? 5e3;
    this.tlsPaths = this.transportInjected || this.cfg.tlsCertPath === void 0 ? null : {
      cert: cleanPemPath(this.cfg.tlsCertPath, "tlsCertPath"),
      key: cleanPemPath(this.cfg.tlsKeyPath, "tlsKeyPath"),
      ca: cleanPemPath(this.cfg.tlsRootCaPath, "tlsRootCaPath")
    };
  }
  /**
   * Build (and memoise) the undici mTLS dispatcher from the configured PEM
   * material. FAIL CLOSED: if `undici` cannot be loaded, the returned promise
   * rejects, so the dependent `postJson` throws and the reserve/commit path
   * fails closed rather than dialing without a client certificate.
   */
  mtlsDispatcher() {
    if (this.dispatcherPromise === null) {
      const paths = this.tlsPaths;
      if (paths === null) {
        this.dispatcherPromise = Promise.reject(new Error("no mTLS material configured"));
      } else {
        this.dispatcherPromise = (async () => {
          const undici = await loadUndici();
          const cert = readFileSync(paths.cert, "utf8");
          const key = readFileSync(paths.key, "utf8");
          const ca = readFileSync(paths.ca, "utf8");
          return new undici.Agent({ connect: { cert, key, ca } });
        })();
        this.dispatcherPromise.catch(() => {
        });
      }
    }
    return this.dispatcherPromise;
  }
  /**
   * Reserve projected spend with the sidecar.
   *
   * ALLOW → returns `ReservationHandle`; DENY → throws `DecisionDenied`;
   * DEGRADE → throws `SidecarUnavailable` unless dev fail-open is set, in
   * which case returns a sentinel handle (empty `reservationId`) and the
   * commit / release path no-ops to keep the call moving without leaking
   * a phantom reservation row.
   */
  async reserve(ctx) {
    const runId = ctx.runId ?? newUuid7();
    const stepId = newUuid7();
    const llmCallId = newUuid7();
    const idempotencyKey = deriveIdempotencyKey({
      tenantId: this.cfg.tenantId,
      sessionId: ctx.conversationId,
      runId,
      stepId,
      llmCallId,
      trigger: "LLM_CALL_PRE"
    });
    const promptText = JSON.stringify(
      ctx.messages.map((m) => ({ role: m.role, content: m.content }))
    );
    const promptHash = computePromptHash(promptText, this.cfg.tenantId);
    const projectedTokens = Math.max(1, ctx.maxTokens);
    const projectedSplit = splitProjectedTokens(projectedTokens);
    const estimatorSnapshot = {
      amountAtomic: String(projectedTokens),
      inputTokens: projectedSplit.input,
      outputTokens: projectedSplit.output
    };
    const body = {
      tenant_id: this.cfg.tenantId,
      claim_estimate_atomic: estimatorSnapshot.amountAtomic,
      // The Kong wire surface uses `prompt_class` / `model_class` as
      // string buckets; we forward the upstream provider as the model
      // class and the prompt-hash prefix as the prompt class so the
      // sidecar's prompt-fingerprint cache can hit cross-call. The
      // sidecar will translate these into the full prompt-hash on the
      // decision_context column.
      prompt_class: promptHash.slice(0, 16),
      model_class: this.cfg.upstreamProvider,
      idempotency_key: idempotencyKey,
      budget_id: this.cfg.spendguardBudgetId,
      decision_context: {
        integration: "botpress",
        mode: "integration_sdk",
        upstream_provider: this.cfg.upstreamProvider,
        bot_id: ctx.botId,
        conversation_id: ctx.conversationId,
        user_id: ctx.userId,
        model: ctx.model,
        window_instance_id: this.cfg.spendguardWindowInstanceId,
        prompt_hash: promptHash,
        run_id: runId,
        step_id: stepId,
        llm_call_id: llmCallId
      }
    };
    let resp;
    try {
      resp = await this.postJson(
        "/v1/decision",
        body,
        this.reserveDeadlineMs
      );
    } catch (err) {
      if (this.failOpenDev) {
        console.warn(
          "spendguard:botpress: fail-open dev mode active; sidecar unreachable, ALLOWing call"
        );
        return {
          decisionId: "",
          reservationId: "",
          llmCallId,
          runId,
          stepId,
          estimatorSnapshot,
          conversationId: ctx.conversationId
        };
      }
      throw new SidecarUnavailable(
        `sidecar unreachable at ${redact(this.cfg.sidecarUrl)}: ${err instanceof Error ? err.message : String(err)}`
      );
    }
    if (resp.verdict === "DENY") {
      throw new DecisionDenied(
        `SpendGuard denied: ${resp.reason_codes?.join(",") ?? "BUDGET_EXCEEDED"}`,
        resp.reason_codes ?? []
      );
    }
    if (resp.verdict === "DEGRADE") {
      if (this.failOpenDev) {
        console.warn("spendguard:botpress: fail-open dev mode active; DEGRADE verdict ALLOWed");
        return {
          decisionId: resp.decision_id,
          reservationId: "",
          llmCallId,
          runId,
          stepId,
          estimatorSnapshot,
          conversationId: ctx.conversationId
        };
      }
      throw new SidecarUnavailable(
        `SpendGuard DEGRADE: ${resp.reason_codes?.join(",") ?? "sidecar_degraded"}`
      );
    }
    if (resp.verdict !== "ALLOW" || resp.reservation_id.length === 0) {
      const detail = resp.verdict !== "ALLOW" ? `unexpected verdict ${JSON.stringify(resp.verdict)}` : "ALLOW without reservation_id";
      if (this.failOpenDev) {
        console.warn(`spendguard:botpress: fail-open dev mode active; ${detail} treated as ALLOW`);
        return {
          decisionId: resp.decision_id ?? "",
          reservationId: "",
          llmCallId,
          runId,
          stepId,
          estimatorSnapshot,
          conversationId: ctx.conversationId
        };
      }
      throw new SidecarUnavailable(`SpendGuard decision malformed: ${detail}`);
    }
    return {
      decisionId: resp.decision_id,
      reservationId: resp.reservation_id,
      llmCallId,
      runId,
      stepId,
      estimatorSnapshot,
      conversationId: ctx.conversationId
    };
  }
  /**
   * Commit successful generation with real provider usage. Falls back to
   * the estimator snapshot when `realUsage` is undefined and logs a WARN
   * (INV-5 secondary, design.md §7 question 3).
   */
  async commitSuccess(handle, realUsage, providerEventId) {
    if (handle.reservationId.length === 0) {
      return;
    }
    let usage = realUsage;
    if (usage === void 0) {
      this.warnEstimatorFallback();
      usage = {
        inputTokens: handle.estimatorSnapshot.inputTokens,
        outputTokens: handle.estimatorSnapshot.outputTokens
      };
    }
    const body = {
      reservation_id: handle.reservationId,
      outcome: "ACCEPTED",
      provider_event_id: providerEventId,
      input_tokens: usage.inputTokens,
      output_tokens: usage.outputTokens,
      actual_amount_atomic: String(usage.inputTokens + usage.outputTokens)
    };
    await this.postJson("/v1/trace", body, this.commitDeadlineMs);
  }
  /**
   * Release reservation on failure / cancellation. Swallows release-RPC
   * errors (TTL sweep is the durable backstop) but logs a WARN. Classifies
   * cancellation-shaped errors as `CANCELLED` outcome via the same regex
   * pattern as the LiteLLM callback (`_classify_failure`).
   */
  async releaseFailure(handle, exc) {
    if (handle.reservationId.length === 0) return;
    const classification = classifyFailure(exc);
    const body = {
      reservation_id: handle.reservationId,
      outcome: "REJECTED",
      provider_event_id: "",
      input_tokens: 0,
      output_tokens: 0,
      actual_amount_atomic: "0"
    };
    try {
      await this.postJson("/v1/trace", body, this.commitDeadlineMs);
    } catch (releaseErr) {
      const reason = releaseErr instanceof Error ? releaseErr.message : String(releaseErr);
      console.warn(
        `spendguard:botpress: release RPC failed for reservation=${handle.reservationId} (${reason}); TTL sweep will reconcile`
      );
    }
    if (classification !== "FAILURE") {
      console.warn(
        `spendguard:botpress: release classified as ${classification} for reservation=${handle.reservationId}`
      );
    }
  }
  warnEstimatorFallback() {
    console.warn(
      "spendguard:botpress: falling back to estimator snapshot (no event.payload.usage on afterAiGeneration)"
    );
  }
  async postJson(path, body, deadlineMs) {
    const url = joinUrl(this.cfg.sidecarUrl, path);
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), deadlineMs);
    try {
      const init = {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
        signal: controller.signal
      };
      if (this.tlsPaths !== null) {
        init.dispatcher = await this.mtlsDispatcher();
      }
      const resp = await this.fetchImpl(url, init);
      if (!resp.ok) {
        const text = await safeReadText(resp);
        throw new Error(`sidecar ${path} returned HTTP ${resp.status}: ${text.slice(0, 200)}`);
      }
      return await resp.json();
    } finally {
      clearTimeout(timer);
    }
  }
};
function splitProjectedTokens(total) {
  const input = Math.max(1, Math.floor(total * 0.3));
  const output = Math.max(1, total - input);
  return { input, output };
}
function classifyFailure(exc) {
  if (exc === void 0 || exc === null) return "FAILURE";
  const name = exc instanceof Error ? exc.name : "";
  const msg = exc instanceof Error ? exc.message : String(exc);
  const blob = `${name} ${msg}`;
  if (/abort|cancel/i.test(blob)) return "CANCELLED";
  if (/timeout|deadline/i.test(blob)) return "TIMEOUT";
  return "FAILURE";
}
function cleanPemPath(raw, field) {
  const clean = normalize(raw);
  if (clean.split(/[/\\]/).includes("..")) {
    throw new SpendGuardConfigError(`spendguard:botpress: ${field} path rejected (traversal)`);
  }
  if (!isAbsolute(clean)) {
    throw new SpendGuardConfigError(`spendguard:botpress: ${field} must be an absolute path`);
  }
  return clean;
}
async function loadUndici() {
  const specifier = ["un", "dici"].join("");
  try {
    const mod = await import(specifier);
    if (typeof mod.Agent !== "function") {
      throw new Error("undici module did not export Agent");
    }
    return mod;
  } catch (err) {
    throw new SidecarUnavailable(
      `spendguard:botpress: mTLS dispatcher unavailable (undici not loadable): ${err instanceof Error ? err.message : String(err)}`
    );
  }
}
function redact(url) {
  try {
    const u = new URL(url);
    return `${u.protocol}//${u.hostname}${u.pathname}`;
  } catch {
    return "(invalid sidecarUrl)";
  }
}
function joinUrl(base, path) {
  const stripped = base.endsWith("/") ? base.slice(0, -1) : base;
  return `${stripped}${path}`;
}
async function safeReadText(resp) {
  try {
    return await resp.text();
  } catch {
    return "(failed to read body)";
  }
}

// src/adapter/errors.ts
function runtimeError(spendguardCode, message, cause) {
  return new RuntimeError(message, cause, void 0, { spendguardCode });
}
function toRuntimeError(err) {
  if (err instanceof DecisionDenied) {
    return runtimeError("BUDGET_DENIED", `SpendGuard denied: ${err.message}`, err);
  }
  if (err instanceof SidecarUnavailable) {
    return runtimeError("BUDGET_DEGRADED", `SpendGuard degraded: ${err.message}`, err);
  }
  if (err instanceof SpendGuardConfigError) {
    return runtimeError("BUDGET_CONFIG", `SpendGuard config: ${err.message}`, err);
  }
  if (err instanceof RuntimeError) {
    return err;
  }
  const cause = err instanceof Error ? err : void 0;
  return runtimeError(
    "BUDGET_CONFIG",
    `SpendGuard config: ${err instanceof Error ? err.message : String(err)}`,
    cause
  );
}
function runtimeErrorCode(rt) {
  const meta = rt.metadata;
  const code = meta?.spendguardCode;
  return code === "BUDGET_DENIED" || code === "BUDGET_DEGRADED" || code === "BUDGET_CONFIG" ? code : void 0;
}
function codeFor(err) {
  return err.code;
}

// src/lifecycle/validateConfiguration.ts
async function validateConfiguration(args) {
  const reservation = args.reservationOverride ?? new SpendGuardReservation(args.configuration);
  const ctx = {
    botId: "validateConfiguration",
    conversationId: "validateConfiguration-probe",
    userId: "validateConfiguration-probe",
    model: args.configuration.upstreamProvider,
    messages: [{ role: "user", content: "probe" }],
    maxTokens: 1,
    runId: "validateConfiguration"
  };
  try {
    const handle = await reservation.reserve(ctx);
    await reservation.releaseFailure(handle, new Error("validateConfiguration probe complete"));
  } catch (err) {
    throw toRuntimeError(err);
  }
}

// src/adapter/binding.ts
var DEFAULT_MODEL = {
  openai: "gpt-4o-mini",
  anthropic: "claude-3-5-haiku-latest",
  bedrock: "anthropic.claude-3-5-haiku"
};
var DEFAULT_MAX_TOKENS = 1024;
function resolveModel(input, configuration) {
  const explicit = input.model;
  if (explicit !== void 0 && explicit.id.length > 0) {
    return explicit.id;
  }
  return DEFAULT_MODEL[configuration.upstreamProvider];
}
function resolveMaxTokens(input) {
  return input.maxTokens !== void 0 && input.maxTokens > 0 ? input.maxTokens : DEFAULT_MAX_TOKENS;
}
function toBindingFromActionInput(args) {
  const { input, configuration, ctx } = args;
  const systemMessages = input.systemPrompt !== void 0 && input.systemPrompt.length > 0 ? [{ role: "system", content: input.systemPrompt }] : [];
  const messages = [
    ...systemMessages,
    ...input.messages.map((m) => ({ role: m.role, content: m.content }))
  ];
  return {
    botId: ctx.botId,
    conversationId: `bot-${ctx.botId}`,
    userId: input.userId ?? "anonymous",
    model: resolveModel(input, configuration),
    messages,
    maxTokens: resolveMaxTokens(input)
  };
}
function pickTenantId(configuration, botId) {
  return configuration.tenantId.length > 0 ? configuration.tenantId : botId;
}

// src/provider/forward.ts
var ProviderForwardError = class extends Error {
  constructor(message) {
    super(message);
    this.name = "ProviderForwardError";
  }
};
function toForwardRequest(input, config, resolvedModel, resolvedMaxTokens) {
  return {
    provider: config.upstreamProvider,
    model: resolvedModel,
    messages: input.messages,
    systemPrompt: input.systemPrompt,
    maxTokens: resolvedMaxTokens,
    temperature: input.temperature,
    topP: input.topP,
    stopSequences: input.stopSequences,
    userId: input.userId
  };
}
function toGenerateContentOutput(result, provider, cost) {
  return {
    id: result.id,
    provider,
    model: result.model,
    choices: [
      {
        role: "assistant",
        type: "text",
        content: result.content,
        index: 0,
        stopReason: result.stopReason
      }
    ],
    usage: {
      inputTokens: result.inputTokens,
      outputTokens: result.outputTokens
    },
    botpress: { cost }
  };
}
var PROVIDER_ENDPOINTS = {
  openai: { url: "https://api.openai.com/v1/chat/completions", apiKeyEnv: "OPENAI_API_KEY" },
  anthropic: { url: "https://api.anthropic.com/v1/messages", apiKeyEnv: "ANTHROPIC_API_KEY" },
  bedrock: {
    url: process.env.BEDROCK_OPENAI_GATEWAY_URL ?? "",
    apiKeyEnv: "BEDROCK_API_KEY"
  }
};
var defaultForward = async (req) => {
  const endpoint = PROVIDER_ENDPOINTS[req.provider];
  const apiKey = (process.env[endpoint.apiKeyEnv] ?? "").trim();
  if (apiKey.length === 0) {
    throw new ProviderForwardError(
      `spendguard:botpress: ${endpoint.apiKeyEnv} is not set; cannot forward to ${req.provider}`
    );
  }
  if (endpoint.url.length === 0) {
    throw new ProviderForwardError(
      `spendguard:botpress: no endpoint configured for provider ${req.provider}`
    );
  }
  if (req.provider === "anthropic") {
    return forwardAnthropic(req, endpoint.url, apiKey);
  }
  return forwardOpenAiCompatible(req, endpoint.url, apiKey);
};
async function forwardOpenAiCompatible(req, url, apiKey) {
  const messages = req.systemPrompt ? [{ role: "system", content: req.systemPrompt }, ...req.messages] : [...req.messages];
  const body = {
    model: req.model,
    messages,
    max_tokens: req.maxTokens,
    ...req.temperature !== void 0 ? { temperature: req.temperature } : {},
    ...req.topP !== void 0 ? { top_p: req.topP } : {},
    ...req.stopSequences !== void 0 ? { stop: req.stopSequences } : {},
    ...req.userId !== void 0 ? { user: req.userId } : {}
  };
  const resp = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${apiKey}` },
    body: JSON.stringify(body)
  });
  if (!resp.ok) {
    throw new ProviderForwardError(`upstream ${req.provider} returned HTTP ${resp.status}`);
  }
  const json = await resp.json();
  const choice = json.choices?.[0];
  return {
    id: json.id ?? "",
    model: json.model ?? req.model,
    content: choice?.message?.content ?? "",
    stopReason: mapStopReason(choice?.finish_reason),
    inputTokens: json.usage?.prompt_tokens ?? 0,
    outputTokens: json.usage?.completion_tokens ?? 0
  };
}
async function forwardAnthropic(req, url, apiKey) {
  const body = {
    model: req.model,
    max_tokens: req.maxTokens,
    ...req.systemPrompt !== void 0 ? { system: req.systemPrompt } : {},
    ...req.temperature !== void 0 ? { temperature: req.temperature } : {},
    ...req.topP !== void 0 ? { top_p: req.topP } : {},
    ...req.stopSequences !== void 0 ? { stop_sequences: req.stopSequences } : {},
    messages: req.messages.map((m) => ({ role: m.role, content: m.content }))
  };
  const resp = await fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-api-key": apiKey,
      "anthropic-version": "2023-06-01"
    },
    body: JSON.stringify(body)
  });
  if (!resp.ok) {
    throw new ProviderForwardError(`upstream anthropic returned HTTP ${resp.status}`);
  }
  const json = await resp.json();
  const text = (json.content ?? []).filter((b) => b.type === "text").map((b) => b.text ?? "").join("");
  return {
    id: json.id ?? "",
    model: json.model ?? req.model,
    content: text,
    stopReason: mapStopReason(json.stop_reason),
    inputTokens: json.usage?.input_tokens ?? 0,
    outputTokens: json.usage?.output_tokens ?? 0
  };
}
function mapStopReason(raw) {
  switch (raw) {
    case "stop":
    case "end_turn":
    case "stop_sequence":
      return "stop";
    case "length":
    case "max_tokens":
      return "max_tokens";
    case "content_filter":
      return "content_filter";
    default:
      return "other";
  }
}

// src/llm/generateContent.ts
async function runGenerateContent(args) {
  const { input, configuration, ctx } = args;
  const forward = args.forward ?? defaultForward;
  const cost = args.costResolver ?? (() => 0);
  let reservation;
  try {
    reservation = args.reservationOverride ?? new SpendGuardReservation(configuration);
  } catch (err) {
    throw toRuntimeError(err);
  }
  const callCtx = toBindingFromActionInput({ input, configuration, ctx });
  let handle;
  try {
    handle = await reservation.reserve(callCtx);
  } catch (err) {
    throw toRuntimeError(err);
  }
  const resolvedModel = resolveModel(input, configuration);
  const resolvedMaxTokens = resolveMaxTokens(input);
  const forwardReq = toForwardRequest(input, configuration, resolvedModel, resolvedMaxTokens);
  let result;
  try {
    result = await forward(forwardReq);
  } catch (err) {
    await reservation.releaseFailure(handle, err);
    const providerErr = err instanceof ProviderForwardError ? err : new ProviderForwardError(
      `spendguard:botpress: upstream forward failed: ${err instanceof Error ? err.message : String(err)}`
    );
    throw toRuntimeError(providerErr);
  }
  const realUsage = { inputTokens: result.inputTokens, outputTokens: result.outputTokens };
  try {
    await reservation.commitSuccess(handle, realUsage, result.id);
  } catch (commitErr) {
    await reservation.releaseFailure(handle, commitErr);
    throw toRuntimeError(commitErr);
  }
  return toGenerateContentOutput(result, configuration.upstreamProvider, cost(realUsage));
}

// src/llm/interfaceAdapter.ts
function flattenContent(content) {
  if (content === null) {
    return "";
  }
  if (typeof content === "string") {
    return content;
  }
  return content.filter((part) => part.type === "text").map((part) => part.text ?? "").join("");
}
function toInternalMessage(message) {
  return { role: message.role, content: flattenContent(message.content) };
}
function toInternalInput(input) {
  return {
    model: input.model !== void 0 ? { id: input.model.id } : void 0,
    messages: input.messages.map(toInternalMessage),
    systemPrompt: input.systemPrompt,
    maxTokens: input.maxTokens,
    temperature: input.temperature,
    topP: input.topP,
    stopSequences: input.stopSequences,
    userId: input.userId
  };
}
function toInterfaceOutput(output) {
  return {
    id: output.id,
    provider: output.provider,
    model: output.model,
    choices: output.choices.map((choice) => ({
      role: choice.role,
      type: choice.type,
      content: choice.content,
      index: choice.index,
      stopReason: choice.stopReason
    })),
    usage: {
      inputTokens: output.usage.inputTokens,
      inputCost: 0,
      outputTokens: output.usage.outputTokens,
      outputCost: 0
    },
    botpress: { cost: output.botpress.cost }
  };
}

// src/llm/listLanguageModels.ts
var MODELS_BY_PROVIDER = {
  openai: [
    {
      id: "gpt-4o",
      name: "OpenAI GPT-4o",
      description: "OpenAI flagship multimodal model.",
      tags: ["recommended", "general-purpose", "vision", "function-calling", "agents"],
      input: { maxTokens: 128e3, costPer1MTokens: 2.5 },
      output: { maxTokens: 16384, costPer1MTokens: 10 }
    },
    {
      id: "gpt-4o-mini",
      name: "OpenAI GPT-4o mini",
      description: "Smaller, low-cost OpenAI multimodal model.",
      tags: ["low-cost", "general-purpose", "vision", "function-calling"],
      input: { maxTokens: 128e3, costPer1MTokens: 0.15 },
      output: { maxTokens: 16384, costPer1MTokens: 0.6 }
    }
  ],
  anthropic: [
    {
      id: "claude-3-5-sonnet-latest",
      name: "Anthropic Claude 3.5 Sonnet",
      description: "Anthropic balanced model for general-purpose agent work.",
      tags: ["recommended", "general-purpose", "vision", "coding", "agents"],
      input: { maxTokens: 2e5, costPer1MTokens: 3 },
      output: { maxTokens: 8192, costPer1MTokens: 15 }
    },
    {
      id: "claude-3-5-haiku-latest",
      name: "Anthropic Claude 3.5 Haiku",
      description: "Fast, low-cost Anthropic model.",
      tags: ["low-cost", "general-purpose", "function-calling"],
      input: { maxTokens: 2e5, costPer1MTokens: 0.8 },
      output: { maxTokens: 8192, costPer1MTokens: 4 }
    }
  ],
  bedrock: [
    {
      id: "anthropic.claude-3-5-sonnet",
      name: "Bedrock Claude 3.5 Sonnet",
      description: "Anthropic Claude 3.5 Sonnet served via Amazon Bedrock.",
      tags: ["recommended", "general-purpose", "vision", "coding", "agents"],
      input: { maxTokens: 2e5, costPer1MTokens: 3 },
      output: { maxTokens: 8192, costPer1MTokens: 15 }
    },
    {
      id: "anthropic.claude-3-5-haiku",
      name: "Bedrock Claude 3.5 Haiku",
      description: "Fast, low-cost Anthropic model served via Amazon Bedrock.",
      tags: ["low-cost", "general-purpose", "function-calling"],
      input: { maxTokens: 2e5, costPer1MTokens: 0.8 },
      output: { maxTokens: 8192, costPer1MTokens: 4 }
    }
  ]
};
function runListLanguageModels(configuration) {
  return { models: [...MODELS_BY_PROVIDER[configuration.upstreamProvider]] };
}
var Integration2 = class extends sdk.Integration {
};

// src/version.ts
var VERSION = "0.1.0";

// src/llm/schemas.ts
var schemas_exports = {};
__export(schemas_exports, {
  ChoiceSchema: () => ChoiceSchema,
  GenerateContentInputSchema: () => GenerateContentInputSchema,
  GenerateContentOutputSchema: () => GenerateContentOutputSchema,
  LanguageModelIdSchema: () => LanguageModelIdSchema,
  LanguageModelSchema: () => LanguageModelSchema,
  ListLanguageModelsInputSchema: () => ListLanguageModelsInputSchema,
  ListLanguageModelsOutputSchema: () => ListLanguageModelsOutputSchema,
  MessageSchema: () => MessageSchema,
  ModelRefSchema: () => ModelRefSchema,
  UsageSchema: () => UsageSchema
});
var LanguageModelIdSchema = z.string().title("LLM Model ID").describe("Provider-qualified model id, e.g. gpt-4o-mini");
var ModelRefSchema = z.object({
  id: LanguageModelIdSchema
});
var MessageSchema = z.object({
  role: z.enum(["system", "user", "assistant", "tool"]).describe("Message role"),
  content: z.string().describe("Flattened message text content")
});
var UsageSchema = z.object({
  inputTokens: z.number().describe("Prompt / input token count"),
  outputTokens: z.number().describe("Completion / output token count")
});
var ChoiceSchema = z.object({
  role: z.literal("assistant").describe("Always assistant for generated content"),
  type: z.literal("text").describe("Content type \u2014 text only in v1"),
  content: z.string().describe("Generated text"),
  index: z.number().describe("Choice index"),
  stopReason: z.enum(["stop", "max_tokens", "content_filter", "other"]).describe("Why generation stopped")
});
var GenerateContentInputSchema = z.object({
  model: ModelRefSchema.optional().describe("Model to use; defaults to the provider default"),
  messages: z.array(MessageSchema).describe("Prompt messages (content flattened to text)"),
  systemPrompt: z.string().optional().describe("Optional system prompt"),
  maxTokens: z.number().optional().describe("Operator-declared output cap; drives the SpendGuard reserve estimate"),
  temperature: z.number().optional().describe("Sampling temperature"),
  topP: z.number().optional().describe("Nucleus sampling cutoff"),
  stopSequences: z.array(z.string()).optional().describe("Stop sequences"),
  userId: z.string().optional().describe("Opaque end-user id forwarded upstream")
});
var GenerateContentOutputSchema = z.object({
  id: z.string().describe("Provider response id"),
  provider: z.string().describe("Upstream provider that served the call"),
  model: z.string().describe("Model id that served the call"),
  choices: z.array(ChoiceSchema).describe("Generated choices"),
  usage: UsageSchema.describe("Real token usage committed to SpendGuard"),
  botpress: z.object({ cost: z.number().describe("Cost in USD as reported to Botpress billing") }).describe("Botpress billing envelope")
});
var LanguageModelSchema = z.object({
  id: LanguageModelIdSchema,
  name: z.string().describe("Human-facing model name"),
  description: z.string().describe("Short model description"),
  tags: z.array(
    z.enum([
      "recommended",
      "deprecated",
      "general-purpose",
      "low-cost",
      "vision",
      "coding",
      "agents",
      "function-calling",
      "roleplay",
      "storytelling",
      "reasoning",
      "preview",
      "speech-to-text",
      "image-generation",
      "text-to-speech"
    ])
  ).describe("Capability / lifecycle tags rendered in the model picker"),
  input: z.object({
    maxTokens: z.number().describe("Max input tokens"),
    costPer1MTokens: z.number().describe("Input cost per 1M tokens, USD")
  }).describe("Input limits + pricing"),
  output: z.object({
    maxTokens: z.number().describe("Max output tokens"),
    costPer1MTokens: z.number().describe("Output cost per 1M tokens, USD")
  }).describe("Output limits + pricing")
});
var ListLanguageModelsInputSchema = z.object({});
var ListLanguageModelsOutputSchema = z.object({
  models: z.array(LanguageModelSchema).describe("Models this integration can route to")
});

// src/index.ts
var src_default = new Integration2({
  // INV-4: prove the SpendGuard sidecar wiring at install time with a
  // 1-token reserve + release roundtrip. A bad sidecar URL / mTLS material /
  // budget id fails the install rather than the first conversation.
  register: async ({ ctx }) => {
    await validateConfiguration({ configuration: ctx.configuration });
  },
  unregister: async () => {
  },
  channels: {},
  // No webhook surface: SpendGuard is invoked synchronously via actions.
  handler: async () => {
  },
  actions: {
    // The SpendGuard gate point. Reserve -> forward -> commit (fail-closed).
    //
    // `input` arrives in the `llm` interface's rich `generateContent` shape
    // (multipart/tool content, reasoning controls, etc.). The boundary adapter
    // narrows it to the SpendGuard pipeline's internal input, runs the gate,
    // then widens the internal result back to the interface output shape.
    generateContent: async ({ input, ctx, logger }) => {
      const result = await runGenerateContent({
        input: toInternalInput(input),
        configuration: ctx.configuration,
        ctx: { botId: ctx.botId, integrationId: ctx.integrationId },
        logger: { warn: (m) => logger.forBot().warn(m) }
      });
      return toInterfaceOutput(result);
    },
    listLanguageModels: async ({ ctx }) => {
      return runListLanguageModels(ctx.configuration);
    }
  }
});

export { ConfigurationObjectSchema, ConfigurationSchema, DecisionDenied, ProviderForwardError, SidecarUnavailable, SpendGuardConfigError, SpendGuardReservation, VERSION, codeFor, src_default as default, defaultForward, flattenContent, schemas_exports as llmSchemas, pickTenantId, resolveMaxTokens, resolveModel, runGenerateContent, runListLanguageModels, runtimeErrorCode, toBindingFromActionInput, toForwardRequest, toGenerateContentOutput, toInterfaceOutput, toInternalInput, toRuntimeError, validateConfiguration };
