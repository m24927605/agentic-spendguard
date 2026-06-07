// `SpendGuardCallbackHandler` — the public LangChain.js callback handler.
//
// SLICE 2 wires the skeleton ONLY:
//   - `extends BaseCallbackHandler` (LangChain's canonical base — review-
//     standards.md §1.2).
//   - `name = "spendguard_callback_handler"` per design.md §4; the parent
//     declares `name` as `abstract`, so the subclass assigns it without an
//     `override` keyword (the rest of the LangChain handlers we mirror
//     follow the same pattern).
//   - PRE / POST / ERROR hook stubs whose signatures match
//     `@langchain/core@0.3`'s `BaseCallbackHandler` exactly — see
//     review-standards.md §2.1 / §2.2 / §2.3 / §2.4.
//   - Per-call `inflight: Map<runId, { decisionId, reservationId }>` so the
//     SLICE 3 wiring has a correlation surface to hand off PRE → POST
//     state without re-deriving the decision identity in the commit path.
//
// SLICE 3 replaces the `Error` throws with real `client.reserve` /
// `client.commitEstimated` calls and lifts the `decisionId` / `reservationId`
// out of the substrate response into the inflight Map.
//
// Throw policy: the stubs intentionally throw a plain `Error` (NOT a
// `SpendGuardError`) so reviewers + tests can distinguish the "skeleton not
// wired" state from real adapter errors. The marker string
// `"SLICE 3 not implemented"` is grep-friendly for the SLICE 3 review gate.

import { BaseCallbackHandler } from "@langchain/core/callbacks/base";
import type { Serialized } from "@langchain/core/load/serializable";
import type { BaseMessage } from "@langchain/core/messages";
import type { LLMResult } from "@langchain/core/outputs";
import type { SpendGuardClient } from "@spendguard/sdk";
import type { SpendGuardCallbackHandlerOptions } from "./options.js";

/**
 * In-flight correlation record. Written by `handleChatModelStart` /
 * `handleLLMStart` (SLICE 3), consumed + deleted by `handleLLMEnd` /
 * `handleLLMError`. Keyed by LangChain's `runId` (the RunManager UUID,
 * which design.md §6.3 fixes as our deterministic `llmCallId`).
 *
 * SLICE 2 exports the type implicitly via the `inflight` field shape; the
 * struct is intentionally minimal so SLICE 3 can extend it (stepId,
 * llmCallId, full `DecisionOutcome`) without breaking the SLICE 2 surface.
 */
interface InflightReservation {
  decisionId: string;
  reservationId: string;
}

/**
 * SpendGuard adapter for LangChain.js.
 *
 * Drop-in via `callbacks: [handler]` on any `BaseChatModel` / `BaseLLM`.
 * SLICE 2 wires the LangChain protocol shape; SLICE 3 wires the substrate
 * `reserve` / `commitEstimated` calls.
 *
 * @example
 * ```ts
 * import { SpendGuardClient } from "@spendguard/sdk";
 * import { SpendGuardCallbackHandler } from "@spendguard/langchain";
 *
 * const client = new SpendGuardClient({ ... });
 * await client.connect();
 * await client.handshake();
 * const handler = new SpendGuardCallbackHandler({ client });
 * // SLICE 3+:  new ChatOpenAI({ callbacks: [handler] }).invoke(...);
 * ```
 *
 * @throws {Error} SLICE 2: every PRE/POST hook throws
 *   `"SLICE 3 not implemented: <hookName>"` until SLICE 3 wires the substrate.
 */
export class SpendGuardCallbackHandler extends BaseCallbackHandler {
  /**
   * Stable serialization name. Mirrors the Python adapter's
   * `class SpendGuardChatModel` and matches the LangChain.js convention of
   * snake_case handler identifiers (`tracer_langchain`,
   * `langfuse_handler`, …).
   */
  name = "spendguard_callback_handler";

  /**
   * Substrate client handed in by the consumer. Stored read-only; the
   * adapter never mutates client config. SLICE 3 dispatches `reserve` /
   * `commitEstimated` against this reference.
   */
  private readonly client: SpendGuardClient;

  /**
   * Consumer-supplied options snapshot. Treated as immutable for the
   * lifetime of the handler. SLICE 3 reads tenant / budget overrides from
   * here on every `reserve` call.
   */
  private readonly opts: SpendGuardCallbackHandlerOptions;

  /**
   * PRE → POST correlation Map keyed by LangChain's `runId`. Empty on
   * construction; SLICE 3 writes on `handleChatModelStart` and `take()`s on
   * `handleLLMEnd` / `handleLLMError`. SLICE 2 keeps it as a plain `Map`;
   * SLICE 3 may swap to a bounded FIFO (review-standards.md §5.2 — 10 k
   * entries with FIFO eviction) without changing the public surface.
   */
  private readonly inflight = new Map<string, InflightReservation>();

  constructor(options: SpendGuardCallbackHandlerOptions) {
    super();
    this.client = options.client;
    this.opts = options;
  }

  /**
   * SLICE 3 wires:
   *   1. Derive `(signature, stepId, decisionId, idempotencyKey)` from
   *      `(messages, runId, parentRunId, tags, metadata, extraParams)`.
   *   2. `await this.client.reserve({ trigger: "LLM_CALL_PRE", … })`.
   *   3. On success, `this.inflight.set(runId, { decisionId, reservationId })`.
   *   4. On `DecisionDenied` / `DecisionStopped` / `SidecarUnavailable`,
   *      rethrow — `raiseError = true` propagates through the
   *      CallbackManager and halts `await model.invoke()`.
   *
   * @throws {Error} SLICE 2 stub — throws unconditionally with the marker
   *   `"SLICE 3 not implemented: handleChatModelStart"`.
   */
  override async handleChatModelStart(
    _llm: Serialized,
    _messages: BaseMessage[][],
    _runId: string,
    _parentRunId?: string,
    _extraParams?: Record<string, unknown>,
    _tags?: string[],
    _metadata?: Record<string, unknown>,
    _name?: string,
  ): Promise<void> {
    throw new Error("SLICE 3 not implemented: handleChatModelStart");
  }

  /**
   * SLICE 3 wires:
   *   1. `const pending = this.inflight.get(runId); this.inflight.delete(runId);`
   *      — unknown `runId` is a no-op (review-standards.md §3.11).
   *   2. Extract `totalTokens` + `providerEventId` from `output`.
   *   3. `await this.client.commitEstimated({ outcome: "SUCCESS", … })`.
   *
   * @throws {Error} SLICE 2 stub — throws unconditionally with the marker
   *   `"SLICE 3 not implemented: handleLLMEnd"`.
   */
  override async handleLLMEnd(
    _output: LLMResult,
    _runId: string,
    _parentRunId?: string,
    _tags?: string[],
  ): Promise<void> {
    throw new Error("SLICE 3 not implemented: handleLLMEnd");
  }

  /**
   * SLICE 3 wires the `PROVIDER_ERROR` commit path. Same correlation
   * lookup as {@link handleLLMEnd}; commits with
   * `estimatedAmountAtomic="0"` and `outcome="PROVIDER_ERROR"`.
   *
   * @throws {Error} SLICE 2 stub — throws unconditionally with the marker
   *   `"SLICE 3 not implemented: handleLLMError"`.
   */
  override async handleLLMError(
    _err: Error,
    _runId: string,
    _parentRunId?: string,
    _tags?: string[],
  ): Promise<void> {
    throw new Error("SLICE 3 not implemented: handleLLMError");
  }
}
