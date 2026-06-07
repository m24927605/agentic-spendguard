import { z, Integration, RuntimeError } from '@botpress/sdk';
import { newUuid7, deriveIdempotencyKey, computePromptHash } from '@spendguard/sdk';

// src/index.ts
var ConfigurationSchema = z.object({
  sidecarUrl: z.string().url().describe("HTTP companion URL (loopback or sidecar-pod port)"),
  spendguardBudgetId: z.string().min(1).describe("UUID of the SpendGuard budget to charge"),
  spendguardWindowInstanceId: z.string().min(1).describe("UUID of the SpendGuard window instance"),
  upstreamProvider: z.enum(["openai", "anthropic", "bedrock"]).describe("Upstream provider Botpress dispatches to"),
  tenantId: z.string().min(1).describe("Operator tenant identifier"),
  tlsCertPath: z.string().optional().describe("Path to SVID cert PEM"),
  tlsKeyPath: z.string().optional().describe("Path to SVID key PEM"),
  tlsRootCaPath: z.string().optional().describe("Path to sidecar CA PEM")
});
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
}
var SpendGuardConfigError = class extends Error {
  code = "BUDGET_CONFIG";
  constructor(message) {
    super(message);
    this.name = "SpendGuardConfigError";
  }
};
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
  constructor(config, opts = {}) {
    assertRequiredConfig(config);
    this.cfg = config;
    this.failOpenDev = opts.failOpenDevOverride ?? (process.env.SPENDGUARD_BOTPRESS_FAIL_OPEN ?? "").trim() === "1";
    this.fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
    this.reserveDeadlineMs = opts.reserveDeadlineMs ?? 5e3;
    this.commitDeadlineMs = opts.commitDeadlineMs ?? 5e3;
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
      const resp = await this.fetchImpl(url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
        signal: controller.signal
      });
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
function toRuntimeError(err) {
  if (err instanceof DecisionDenied) {
    return new RuntimeError(`SpendGuard denied: ${err.message}`);
  }
  if (err instanceof SidecarUnavailable) {
    return new RuntimeError(`SpendGuard degraded: ${err.message}`);
  }
  if (err instanceof SpendGuardConfigError) {
    return new RuntimeError(`SpendGuard config: ${err.message}`);
  }
  if (err instanceof RuntimeError) {
    return err;
  }
  return new RuntimeError(`SpendGuard config: ${err instanceof Error ? err.message : String(err)}`);
}
function codeFor(err) {
  return err.code;
}

// src/adapter/usage.ts
function extractUsageFromBotpressEvent(data) {
  if (data === void 0) return void 0;
  const primary = data.payload?.usage ?? data.usage;
  if (primary !== void 0) {
    if (typeof primary.inputTokens === "number" || typeof primary.outputTokens === "number") {
      return {
        inputTokens: primary.inputTokens ?? 0,
        outputTokens: primary.outputTokens ?? 0
      };
    }
  }
  const raw = data.response?.usage;
  if (raw !== void 0) {
    const inputTokens = raw.input_tokens ?? raw.prompt_tokens;
    const outputTokens = raw.output_tokens ?? raw.completion_tokens;
    if (typeof inputTokens === "number" || typeof outputTokens === "number") {
      return {
        inputTokens: inputTokens ?? 0,
        outputTokens: outputTokens ?? 0
      };
    }
  }
  return void 0;
}
function snapshotToUsage(snapshot) {
  return {
    inputTokens: snapshot.inputTokens,
    outputTokens: snapshot.outputTokens
  };
}
function pickProviderEventId(data) {
  return data?.providerEventId ?? "";
}

// src/hooks/afterAiGeneration.ts
async function runAfterAiGeneration(args) {
  const data = args.input.data;
  const handle = data._spendguardHandle;
  if (handle === void 0) {
    return { data };
  }
  const reservation = args.reservationOverride ?? new SpendGuardReservation(args.configuration);
  const cancelled = data._cancelled === true;
  if (cancelled) {
    await reservation.releaseFailure(
      handle,
      Object.assign(new Error("Botpress conversation cancelled"), {
        name: "AbortError"
      })
    );
    clearHandle(data);
    return { data };
  }
  const realUsage = extractUsageFromBotpressEvent(
    data
  );
  const providerEventId = pickProviderEventId(data);
  try {
    await reservation.commitSuccess(handle, realUsage, providerEventId);
  } catch (commitErr) {
    try {
      await reservation.releaseFailure(handle, commitErr);
    } catch (releaseErr) {
      console.warn(
        `spendguard:botpress: release-after-commit-failure swallowed for handle=${handle.reservationId}: ${releaseErr instanceof Error ? releaseErr.message : String(releaseErr)}`
      );
    }
    clearHandle(data);
    throw toRuntimeError(commitErr);
  }
  clearHandle(data);
  return { data };
}
function clearHandle(data, handle) {
  if ((process.env.SPENDGUARD_BOTPRESS_KEEP_HANDLE ?? "").trim() === "1") {
    return;
  }
  try {
    data._spendguardHandle = void 0;
  } catch {
  }
}

// src/adapter/binding.ts
function toBindingFromHookInput(args) {
  const { input, configuration } = args;
  const data = input.data;
  const messagesRaw = data.input?.messages ?? data.messages ?? [];
  const messages = messagesRaw.map((m) => ({
    role: m.role ?? "user",
    content: m.content ?? ""
  }));
  return {
    botId: input.ctx.botId,
    conversationId: data.conversationId ?? `conv-${input.ctx.botId}`,
    userId: data.userId ?? "anonymous",
    model: data.model ?? "unknown",
    messages,
    maxTokens: data.maxTokens ?? 1024
  };
}
function pickTenantId(configuration, botId) {
  return configuration.tenantId.length > 0 ? configuration.tenantId : botId;
}

// src/hooks/beforeAiGeneration.ts
async function runBeforeAiGeneration(args) {
  let reservation;
  try {
    reservation = args.reservationOverride ?? new SpendGuardReservation(args.configuration);
  } catch (err) {
    throw toRuntimeError(err);
  }
  const ctx = toBindingFromHookInput({
    input: args.input,
    configuration: args.configuration
  });
  let handle;
  try {
    handle = await reservation.reserve(ctx);
  } catch (err) {
    throw toRuntimeError(err);
  }
  const stash = args.input.data;
  Object.defineProperty(stash, "_spendguardHandle", {
    value: handle,
    writable: true,
    enumerable: false,
    configurable: true
  });
  return { data: stash };
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

// src/version.ts
var VERSION = "0.1.0";

// src/index.ts
var src_default = new Integration({
  configuration: { schema: ConfigurationSchema },
  register: async ({ configuration }) => {
    await validateConfiguration({ configuration });
  },
  unregister: async () => {
  },
  channels: {},
  actions: {},
  hooks: {
    beforeAiGeneration: async ({
      ctx,
      data,
      configuration
    }) => {
      const out = await runBeforeAiGeneration({
        input: {
          ctx,
          data
        },
        configuration
      });
      return { data: out.data };
    },
    afterAiGeneration: async ({
      ctx,
      data,
      configuration
    }) => {
      const out = await runAfterAiGeneration({
        input: {
          ctx,
          data
        },
        configuration
      });
      return { data: out.data };
    }
  }
});

export { ConfigurationSchema, DecisionDenied, SidecarUnavailable, SpendGuardConfigError, SpendGuardReservation, VERSION, codeFor, src_default as default, extractUsageFromBotpressEvent, pickProviderEventId, pickTenantId, runAfterAiGeneration, runBeforeAiGeneration, snapshotToUsage, toBindingFromHookInput, toRuntimeError, validateConfiguration };
