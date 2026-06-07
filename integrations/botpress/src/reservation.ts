// Reservation / commit / release delegate for the Botpress integration.
//
// Mirrors plugins/dify/spendguard/models/llm/_DifyReservation.py and the
// SpendGuard LiteLLM callback (sdk/python/src/spendguard/integrations/litellm.py
// SpendGuardLiteLLMCallback). Composition over inheritance — the Botpress
// hook signature and the SpendGuard reservation lifecycle are orthogonal
// state machines.
//
// Transport: HTTP+mTLS to the D09 SLICE 1 HTTP companion at
// `/v1/decision` (reserve) and `/v1/trace` (commit / release). The companion
// shape is the Kong-shaped subset of the gRPC RequestDecision RPC; D32
// reuses, does not extend, that contract (review-standards.md §3.14, §3 D09
// contract reuse).
//
// LOCKED behaviour (design.md §5 + review-standards.md §3):
//   1. PRE — build BudgetBinding from configuration + DifyCallContext-style
//      hook context; D05 `deriveIdempotencyKey` + `computePromptHash` from
//      the SDK barrel. Sidecar POST `/v1/decision`; ALLOW returns; DENY →
//      `DecisionDenied`; DEGRADE → `SidecarUnavailable` (fail-closed unless
//      `SPENDGUARD_BOTPRESS_FAIL_OPEN=1`).
//   2. POST SUCCESS — sidecar POST `/v1/trace` with
//      `outcome=ACCEPTED`, `input_tokens` / `output_tokens` from
//      `event.payload.usage`. Estimator-snapshot fallback when `usage` is
//      missing (WARN log).
//   3. POST FAILURE — sidecar POST `/v1/trace` with `outcome=REJECTED`
//      to release the reservation. `releaseFailure` swallows release-RPC
//      errors and logs WARN (TTL sweep backstop).
//
// Anti-scope:
//   - No gRPC over UDS. D32 talks HTTP+mTLS only. The Python Dify plugin
//     uses the gRPC adapter because it lives in-process with the sidecar
//     in the demo; Botpress runs as a sibling pod / container so HTTP+mTLS
//     is the natural transport.
//   - No module-level mutable state (review-standards.md §3 cross-cutting,
//     §2.8 Slice 2). All state lives on the instance.

import { computePromptHash, deriveIdempotencyKey, newUuid7 } from "@spendguard/sdk";
import { type Configuration, SpendGuardConfigError, assertRequiredConfig } from "./config.js";

// --------------------------------------------------------------------
// Public data carriers
// --------------------------------------------------------------------

/**
 * Inputs the reservation sees per Botpress AI hook call. Built by
 * `src/adapter/binding.ts` from the Botpress hook input — `data.conversationId`
 * / `ctx.botId` / `data.model` / `data.input.messages` / `data.input.maxTokens`.
 */
export interface BotpressCallContext {
  readonly botId: string;
  readonly conversationId: string;
  readonly userId: string;
  readonly model: string;
  readonly messages: ReadonlyArray<{ role: string; content: string }>;
  readonly maxTokens: number;
  /** Optional run identifier (Botpress message id); generated if absent. */
  readonly runId?: string;
}

/**
 * State carried from `reserve` → `commitSuccess` / `releaseFailure` for one
 * AI hook call. Stashed on `data._spendguardHandle` between
 * `beforeAiGeneration` and `afterAiGeneration` (review-standards.md §3.11).
 *
 * Readonly + plain-object so it survives the synchronous stash on the
 * Botpress hook payload object without surprising the runtime's
 * JSON-serialisation pass.
 */
export interface ReservationHandle {
  readonly decisionId: string;
  readonly reservationId: string;
  readonly llmCallId: string;
  readonly runId: string;
  readonly stepId: string;
  /** Snapshot of the projected claim used at reserve time. Estimator
   * fallback in afterAiGeneration commits this amount when
   * `event.payload.usage` is missing. */
  readonly estimatorSnapshot: {
    readonly amountAtomic: string;
    readonly inputTokens: number;
    readonly outputTokens: number;
  };
  /** Conversation id for cross-hook correlation in the audit chain. */
  readonly conversationId: string;
}

// --------------------------------------------------------------------
// Errors mirrored from the SDK shape
// --------------------------------------------------------------------

/** Sidecar returned DENY — fail-closed. Translated to Botpress
 *  `RuntimeError("BUDGET_DENIED")` by `src/adapter/errors.ts`. */
export class DecisionDenied extends Error {
  readonly code = "BUDGET_DENIED" as const;
  readonly reasonCodes: ReadonlyArray<string>;
  constructor(message: string, reasonCodes: ReadonlyArray<string> = []) {
    super(message);
    this.name = "DecisionDenied";
    this.reasonCodes = reasonCodes;
  }
}

/** Sidecar returned DEGRADE or transport-level failure — fail-closed by
 *  default. Translated to Botpress `RuntimeError("BUDGET_DEGRADED")`. Dev
 *  escape: `SPENDGUARD_BOTPRESS_FAIL_OPEN=1`. */
export class SidecarUnavailable extends Error {
  readonly code = "BUDGET_DEGRADED" as const;
  constructor(message: string) {
    super(message);
    this.name = "SidecarUnavailable";
  }
}

// Re-export the config error so consumers have one import for the trio.
export { SpendGuardConfigError } from "./config.js";

// --------------------------------------------------------------------
// HTTP companion wire shapes — Kong-shaped subset.
// --------------------------------------------------------------------
// See services/sidecar/src/http_companion/handlers.rs for the canonical
// definitions on the sidecar side. We mirror only the fields D32 needs.

interface CompanionDecisionRequest {
  tenant_id: string;
  claim_estimate_atomic: string;
  prompt_class: string;
  model_class: string;
  idempotency_key: string;
  budget_id: string;
  /** Optional metadata for SQL gate filtering — not part of the locked
   *  Kong-shaped contract, but the sidecar passes through unknown JSON
   *  fields verbatim into the decision_context column. */
  decision_context?: Record<string, string>;
}

interface CompanionDecisionResponse {
  verdict: "ALLOW" | "DENY" | "DEGRADE";
  reservation_id: string;
  decision_id: string;
  reason_codes?: string[];
}

interface CompanionTraceRequest {
  reservation_id: string;
  outcome: "ACCEPTED" | "REJECTED";
  provider_event_id?: string;
  input_tokens?: number;
  output_tokens?: number;
  actual_amount_atomic?: string;
}

// --------------------------------------------------------------------
// SpendGuardReservation — the delegate
// --------------------------------------------------------------------

/**
 * Reservation / commit / release delegate for the Botpress integration.
 *
 * Composition-only (review-standards.md §2.5 / §3 cross-cutting). The
 * Botpress hook signature lives in `src/hooks/*`; this class owns the
 * SpendGuard lifecycle and is reusable across hooks if a future Botpress
 * SDK exposes additional pre/post slots (review-standards.md §7 reviewer
 * override note).
 */
export class SpendGuardReservation {
  private readonly cfg: Configuration;
  private readonly failOpenDev: boolean;
  /** Per-instance HTTP client overrides — used by the unit test
   *  `_mockSidecar.ts` to drive the wire path without a real network
   *  socket. Production runtime uses the global `fetch`. */
  private readonly fetchImpl: typeof globalThis.fetch;
  private readonly reserveDeadlineMs: number;
  private readonly commitDeadlineMs: number;

  constructor(
    config: Partial<Configuration>,
    opts: {
      readonly fetchImpl?: typeof globalThis.fetch;
      readonly reserveDeadlineMs?: number;
      readonly commitDeadlineMs?: number;
      /** Override the env-var fail-open check (test convenience). */
      readonly failOpenDevOverride?: boolean;
    } = {},
  ) {
    assertRequiredConfig(config);
    this.cfg = config;
    this.failOpenDev =
      opts.failOpenDevOverride ?? (process.env.SPENDGUARD_BOTPRESS_FAIL_OPEN ?? "").trim() === "1";
    this.fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
    this.reserveDeadlineMs = opts.reserveDeadlineMs ?? 5_000;
    this.commitDeadlineMs = opts.commitDeadlineMs ?? 5_000;
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
  async reserve(ctx: BotpressCallContext): Promise<ReservationHandle> {
    const runId = ctx.runId ?? newUuid7();
    const stepId = newUuid7();
    const llmCallId = newUuid7();
    const idempotencyKey = deriveIdempotencyKey({
      tenantId: this.cfg.tenantId,
      sessionId: ctx.conversationId,
      runId,
      stepId,
      llmCallId,
      trigger: "LLM_CALL_PRE",
    });
    // computePromptHash takes (promptText, tenantId) per
    // sdk/typescript/src/promptHash.ts — we serialise the message list
    // into a deterministic JSON string so two structurally-identical
    // message arrays hash byte-for-byte the same. The tenant key salts
    // the HMAC so cross-tenant prompt fingerprints never collide.
    const promptText = JSON.stringify(
      ctx.messages.map((m) => ({ role: m.role, content: m.content })),
    );
    const promptHash = computePromptHash(promptText, this.cfg.tenantId);

    // Estimator: maxTokens is the operator-declared cap; we use it as the
    // projected claim. The atomic unit on the sidecar side is whatever the
    // budget's unit is configured to; for the Kong-shaped subset, we pass
    // the token count as the atomic amount and let the bundle / pricing
    // freeze on the sidecar side translate to dollars at commit time. This
    // mirrors what the Kong plugin (D09) does — see
    // services/sidecar/src/http_companion/handlers.rs DecisionRequest doc.
    const projectedTokens = Math.max(1, ctx.maxTokens);
    const projectedSplit = splitProjectedTokens(projectedTokens);
    const estimatorSnapshot = {
      amountAtomic: String(projectedTokens),
      inputTokens: projectedSplit.input,
      outputTokens: projectedSplit.output,
    };

    const body: CompanionDecisionRequest = {
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
        llm_call_id: llmCallId,
      },
    };

    let resp: CompanionDecisionResponse;
    try {
      resp = await this.postJson<CompanionDecisionResponse>(
        "/v1/decision",
        body,
        this.reserveDeadlineMs,
      );
    } catch (err) {
      if (this.failOpenDev) {
        // Dev escape — return a sentinel handle; commit / release will
        // see the empty reservationId and skip the trace POST.
        console.warn(
          "spendguard:botpress: fail-open dev mode active; sidecar unreachable, ALLOWing call",
        );
        return {
          decisionId: "",
          reservationId: "",
          llmCallId,
          runId,
          stepId,
          estimatorSnapshot,
          conversationId: ctx.conversationId,
        };
      }
      throw new SidecarUnavailable(
        `sidecar unreachable at ${redact(this.cfg.sidecarUrl)}: ${
          err instanceof Error ? err.message : String(err)
        }`,
      );
    }

    if (resp.verdict === "DENY") {
      throw new DecisionDenied(
        `SpendGuard denied: ${resp.reason_codes?.join(",") ?? "BUDGET_EXCEEDED"}`,
        resp.reason_codes ?? [],
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
          conversationId: ctx.conversationId,
        };
      }
      throw new SidecarUnavailable(
        `SpendGuard DEGRADE: ${resp.reason_codes?.join(",") ?? "sidecar_degraded"}`,
      );
    }

    return {
      decisionId: resp.decision_id,
      reservationId: resp.reservation_id,
      llmCallId,
      runId,
      stepId,
      estimatorSnapshot,
      conversationId: ctx.conversationId,
    };
  }

  /**
   * Commit successful generation with real provider usage. Falls back to
   * the estimator snapshot when `realUsage` is undefined and logs a WARN
   * (INV-5 secondary, design.md §7 question 3).
   */
  async commitSuccess(
    handle: ReservationHandle,
    realUsage: { inputTokens: number; outputTokens: number } | undefined,
    providerEventId: string,
  ): Promise<void> {
    if (handle.reservationId.length === 0) {
      // Fail-open sentinel — nothing to commit. Already logged at reserve.
      return;
    }
    let usage = realUsage;
    if (usage === undefined) {
      this.warnEstimatorFallback();
      usage = {
        inputTokens: handle.estimatorSnapshot.inputTokens,
        outputTokens: handle.estimatorSnapshot.outputTokens,
      };
    }
    const body: CompanionTraceRequest = {
      reservation_id: handle.reservationId,
      outcome: "ACCEPTED",
      provider_event_id: providerEventId,
      input_tokens: usage.inputTokens,
      output_tokens: usage.outputTokens,
      actual_amount_atomic: String(usage.inputTokens + usage.outputTokens),
    };
    await this.postJson("/v1/trace", body, this.commitDeadlineMs);
  }

  /**
   * Release reservation on failure / cancellation. Swallows release-RPC
   * errors (TTL sweep is the durable backstop) but logs a WARN. Classifies
   * cancellation-shaped errors as `CANCELLED` outcome via the same regex
   * pattern as the LiteLLM callback (`_classify_failure`).
   */
  async releaseFailure(handle: ReservationHandle, exc: unknown): Promise<void> {
    if (handle.reservationId.length === 0) return;
    const classification = classifyFailure(exc);
    const body: CompanionTraceRequest = {
      reservation_id: handle.reservationId,
      outcome: "REJECTED",
      provider_event_id: "",
      input_tokens: 0,
      output_tokens: 0,
      actual_amount_atomic: "0",
    };
    try {
      await this.postJson("/v1/trace", body, this.commitDeadlineMs);
    } catch (releaseErr) {
      const reason = releaseErr instanceof Error ? releaseErr.message : String(releaseErr);
      console.warn(
        `spendguard:botpress: release RPC failed for reservation=${handle.reservationId} (${reason}); TTL sweep will reconcile`,
      );
      // Do NOT re-throw — TTL sweep is the durable backstop
      // (review-standards.md §3.5).
    }
    // Re-emit the classification on the log so the audit operator can
    // see the cancellation signal (no PII; only the regex bucket).
    if (classification !== "FAILURE") {
      console.warn(
        `spendguard:botpress: release classified as ${classification} for reservation=${handle.reservationId}`,
      );
    }
  }

  private warnEstimatorFallback(): void {
    console.warn(
      "spendguard:botpress: falling back to estimator snapshot (no event.payload.usage on afterAiGeneration)",
    );
  }

  private async postJson<T>(
    path: "/v1/decision" | "/v1/trace",
    body: unknown,
    deadlineMs: number,
  ): Promise<T> {
    const url = joinUrl(this.cfg.sidecarUrl, path);
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), deadlineMs);
    try {
      const resp = await this.fetchImpl(url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
        signal: controller.signal,
      });
      if (!resp.ok) {
        const text = await safeReadText(resp);
        throw new Error(`sidecar ${path} returned HTTP ${resp.status}: ${text.slice(0, 200)}`);
      }
      return (await resp.json()) as T;
    } finally {
      clearTimeout(timer);
    }
  }
}

// --------------------------------------------------------------------
// Helpers — pure
// --------------------------------------------------------------------

/** Split projected `maxTokens` 30/70 input/output as a default budget
 *  estimator. Replaced by the SpendGuard tokenizer service in a future
 *  slice; the 30/70 default matches the heuristic in the egress-proxy
 *  decision path (decision.rs:277-295 dead-code reference). */
function splitProjectedTokens(total: number): { input: number; output: number } {
  const input = Math.max(1, Math.floor(total * 0.3));
  const output = Math.max(1, total - input);
  return { input, output };
}

/**
 * Classify a failure-path exception into CANCELLED / TIMEOUT / FAILURE.
 *
 * Mirrors `sdk/python/src/spendguard/integrations/litellm.py::_classify_failure`
 * lines 735-760. The matching regex stays alpha-only to dodge locale-specific
 * casing and to keep the substring scan cheap.
 */
function classifyFailure(exc: unknown): "CANCELLED" | "TIMEOUT" | "FAILURE" {
  if (exc === undefined || exc === null) return "FAILURE";
  const name = exc instanceof Error ? exc.name : "";
  const msg = exc instanceof Error ? exc.message : String(exc);
  const blob = `${name} ${msg}`;
  if (/abort|cancel/i.test(blob)) return "CANCELLED";
  if (/timeout|deadline/i.test(blob)) return "TIMEOUT";
  return "FAILURE";
}

/** Redact a URL down to the scheme + host + path so logs do not leak the
 *  port the loopback companion lives on or any embedded auth material.
 *  Operator-visible breadcrumb only; INV-6. */
function redact(url: string): string {
  try {
    const u = new URL(url);
    return `${u.protocol}//${u.hostname}${u.pathname}`;
  } catch {
    return "(invalid sidecarUrl)";
  }
}

/** Join a base URL and an absolute path without double-slash hazards. */
function joinUrl(base: string, path: `/${string}`): string {
  const stripped = base.endsWith("/") ? base.slice(0, -1) : base;
  return `${stripped}${path}`;
}

async function safeReadText(resp: Response): Promise<string> {
  try {
    return await resp.text();
  } catch {
    return "(failed to read body)";
  }
}

// Internal helper re-exports for unit tests
/** @internal */
export const __internal = {
  splitProjectedTokens,
  classifyFailure,
  redact,
  joinUrl,
};
