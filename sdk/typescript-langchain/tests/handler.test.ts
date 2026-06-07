// SLICE 2 — SpendGuardCallbackHandler skeleton tests.
//
// Scope (per docs/slices/COV_D04_S2_handler_skeleton.md):
//   - Confirm the LOCKED public surface (name, options shape, inflight Map).
//   - Confirm LangChain protocol shape (`extends BaseCallbackHandler`).
//   - Confirm SLICE 3 hooks throw the "not implemented" marker so the wiring
//     gate can detect the skeleton state.
//
// SLICE 3 will replace the throws with real `reserve` / `commitEstimated`
// calls; these tests get rewritten then to assert on the substrate
// interaction (review-standards.md §3 — Reserve / commit semantics).
//
// Anti-scope: no mock sidecar, no `reserve` / `commitEstimated` calls,
// no streaming. Those land in SLICE 3 / SLICE 4.

import { BaseCallbackHandler } from "@langchain/core/callbacks/base";
import type { Serialized } from "@langchain/core/load/serializable";
import type { BaseMessage } from "@langchain/core/messages";
import type { LLMResult } from "@langchain/core/outputs";
import { SpendGuardClient } from "@spendguard/sdk";
import { describe, expect, it } from "vitest";
import { SpendGuardCallbackHandler } from "../src/handler.js";
import type { SpendGuardCallbackHandlerOptions } from "../src/options.js";

/**
 * Build a `SpendGuardClient` configured against a non-existent UDS path and a
 * placeholder tenant. SLICE 2 never calls `connect()` / `handshake()` /
 * `reserve()` on this client — the handler stubs throw before any RPC would
 * fire — so the client stays a passive identity. SLICE 4 swaps this for the
 * mock-sidecar helper.
 */
function makeClient(): SpendGuardClient {
  return new SpendGuardClient({
    socketPath: "/tmp/spendguard-slice2-test.sock",
    tenantId: "tenant-slice2-test",
    runtimeKind: "langchain-js",
    workloadInstanceId: "wi-slice2-test",
  });
}

function makeOptions(): SpendGuardCallbackHandlerOptions {
  return { client: makeClient() };
}

/**
 * Minimal fake `Serialized` shape — the SLICE 2 stub never reads it, so a
 * type-cast keeps the test surface terse. SLICE 3 swaps in real serialized
 * model metadata.
 */
const FAKE_SERIALIZED = {
  lc: 1,
  type: "constructor",
  id: ["test"],
  kwargs: {},
} as unknown as Serialized;
const FAKE_MESSAGES: BaseMessage[][] = [[]];
const FAKE_RUN_ID = "11111111-2222-3333-4444-555555555555";
const FAKE_LLM_RESULT: LLMResult = { generations: [[]] };

describe("SpendGuardCallbackHandler — SLICE 2 skeleton", () => {
  it("exposes `name = 'spendguard_callback_handler'` per design.md §4", () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    expect(handler.name).toBe("spendguard_callback_handler");
  });

  it("constructor accepts a `SpendGuardCallbackHandlerOptions` object", () => {
    const opts = makeOptions();
    const handler = new SpendGuardCallbackHandler(opts);
    // The handler is a live instance with the expected base properties.
    expect(handler).toBeDefined();
    // `raiseError` / `awaitHandlers` come from the base class; SLICE 3 will
    // verify the throw-propagation contract end-to-end. SLICE 2 just confirms
    // the base class wired them.
    expect(typeof handler.raiseError).toBe("boolean");
    expect(typeof handler.awaitHandlers).toBe("boolean");
  });

  it("starts with an empty `inflight` Map (no PRE has fired yet)", () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    // Reach through the private boundary via a typed cast — SLICE 3 will
    // promote `inflight.size` to a public assertion via the substrate
    // round-trip.
    const inflight = (handler as unknown as { inflight: Map<string, unknown> }).inflight;
    expect(inflight).toBeInstanceOf(Map);
    expect(inflight.size).toBe(0);
  });

  it("`handleChatModelStart` throws the SLICE 3 not-implemented marker", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    await expect(
      handler.handleChatModelStart(FAKE_SERIALIZED, FAKE_MESSAGES, FAKE_RUN_ID),
    ).rejects.toThrow("SLICE 3 not implemented: handleChatModelStart");
  });

  it("`handleLLMEnd` throws the SLICE 3 not-implemented marker", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    await expect(handler.handleLLMEnd(FAKE_LLM_RESULT, FAKE_RUN_ID)).rejects.toThrow(
      "SLICE 3 not implemented: handleLLMEnd",
    );
  });

  it("`handleLLMError` throws the SLICE 3 not-implemented marker", async () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    await expect(
      handler.handleLLMError(new Error("provider blew up"), FAKE_RUN_ID),
    ).rejects.toThrow("SLICE 3 not implemented: handleLLMError");
  });

  it("`extends BaseCallbackHandler` — instance passes the LangChain identity check", () => {
    const handler = new SpendGuardCallbackHandler(makeOptions());
    expect(handler).toBeInstanceOf(BaseCallbackHandler);
  });

  it("multiple instances each own their own `inflight` Map (no shared state)", () => {
    const a = new SpendGuardCallbackHandler(makeOptions());
    const b = new SpendGuardCallbackHandler(makeOptions());
    const inflightA = (a as unknown as { inflight: Map<string, unknown> }).inflight;
    const inflightB = (b as unknown as { inflight: Map<string, unknown> }).inflight;
    expect(inflightA).not.toBe(inflightB);
    // Mutate one; the other must stay empty — protects against an accidental
    // `private static inflight` regression in SLICE 3.
    inflightA.set("runid-a", { decisionId: "d", reservationId: "r" });
    expect(inflightA.size).toBe(1);
    expect(inflightB.size).toBe(0);
  });
});
