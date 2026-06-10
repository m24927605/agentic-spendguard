// COV_D38_02 — real `@mastra/core` Agent integration (tests.md TP-22, gate
// A3.8) + the V1/V2/V3/V5 marker-pin evidence suite.
//
// Marker pins recorded here against the INSTALLED package (design §12):
//   V1 — `implements Processor` typechecks (lockedSurface TP-02 is the type
//        gate); this file proves the runtime hook contract: args carry
//        MastraDBMessage[] `messages` whose flatten feeds §6.3 identity.
//   V2 — throw-from-processInputStep halts pre-provider (failClosed TP-10);
//        this file adds the ALLOW-path complement: returning undefined lets
//        the step proceed to exactly one provider call.
//   V3 — no Mastra run id / per-call correlation id at the hook surface →
//        LOCKED per-runId FIFO fallback (asserted via runId === llmCallId
//        on the real loop's reserve).
//   V5 — Agent constructor mount key `inputProcessors` (both stub-model and
//        model-router-string agents below mount through it).

import { Agent } from "@mastra/core/agent";
import { deriveUuidFromSignature } from "@spendguard/sdk";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { SpendGuardProcessor } from "../src/index.js";
import { MockSpendGuardClient } from "./_support/mockSidecar.js";
import { RecordingStubModel } from "./_support/stubModel.js";

const savedOpenAiKey = process.env.OPENAI_API_KEY;

beforeEach(() => {
  // TP-22's router-string agent must never reach a provider: the DENY plan
  // aborts pre-dispatch. The dummy key only satisfies eager config checks.
  process.env.OPENAI_API_KEY = "sk-test-dummy-never-used";
});

afterEach(() => {
  if (savedOpenAiKey === undefined) {
    delete process.env.OPENAI_API_KEY;
  } else {
    process.env.OPENAI_API_KEY = savedOpenAiKey;
  }
});

describe("COV_D38_02 real @mastra/core integration (TP-22 + V-pins)", () => {
  it("ALLOW path on a real Agent: one reserve per step, provider runs, step text drives identity (V1/V2/V5)", async () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-int" });
    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "integration-agent",
      name: "integration-agent",
      instructions: "test agent",
      model: stub as never,
      // V5 PIN: `inputProcessors` is the @mastra/core 1.x mount key.
      inputProcessors: [guard],
    });

    const result = await agent.generate("ping");

    expect(result.text).toBe("stub-reply");
    expect(stub.totalCalls).toBe(1);
    expect(mock.reserveCalls).toHaveLength(1);

    const req = mock.lastReserveRequest;
    // V1 evidence: the hook received MastraDBMessage[] whose text-part
    // flatten is exactly the user prompt (system instructions travel on
    // `systemMessages`, NOT `messages`) — the §6.3 identity derives from it.
    expect(req?.llmCallId).toBe(
      deriveUuidFromSignature("v1|tenant-int|ping", { scope: "mastra_llm_call_id" }),
    );
    // V3 evidence: no Mastra-context run id exists at the hook → content-
    // derived fallback (runId === llmCallId) on the REAL loop.
    expect(req?.runId).toBe(req?.llmCallId);
  }, 30_000);

  it("TP-22: processor mounts on a model-router-string Agent and processInputStep fires (pins V5)", async () => {
    const mock = new MockSpendGuardClient({ defaultDecision: { kind: "DENY" } });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-router" });
    // The flagship Mastra DX path D06 cannot reach: model-router string.
    // There is NO wrapLanguageModel injection point here — the Processor
    // mount is the only enforcement boundary (design §1/§2).
    const agent = new Agent({
      id: "router-string-agent",
      name: "router-string-agent",
      instructions: "test agent",
      model: "openai/gpt-4o-mini",
      inputProcessors: [guard],
    });

    // DENY plan: the reserve fires (proving the hook runs on the
    // router-string path) and the step aborts before any provider/network
    // activity (fail-closed).
    await expect(agent.generate("router ping")).rejects.toThrow(/mock budget denied/);

    expect(mock.reserveCalls).toHaveLength(1);
    expect(mock.reserveCalls[0]?.rejected?.name).toBe("DecisionDenied");
    const req = mock.lastReserveRequest;
    expect(req?.trigger).toBe("LLM_CALL_PRE");
    expect(req?.llmCallId).toBe(
      deriveUuidFromSignature("v1|tenant-router|router ping", {
        scope: "mastra_llm_call_id",
      }),
    );
  }, 30_000);

  it("retry of the SAME step re-derives the same identity (sidecar idempotency contract, §6.3)", async () => {
    const mock = new MockSpendGuardClient({
      // First attempt: transport failure AFTER the request is recorded;
      // retry: ALLOW.
      decisionQueue: [{ kind: "SIDECAR_UNAVAILABLE" }, { kind: "ALLOW" }],
    });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-retry" });
    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "retry-agent",
      name: "retry-agent",
      instructions: "test agent",
      model: stub as never,
      inputProcessors: [guard],
    });

    await expect(agent.generate("retry me")).rejects.toThrow();
    expect(stub.totalCalls).toBe(0);

    // Same accumulated messages → byte-identical stepText → identical
    // llmCallId / decisionId / idempotencyKey on the retry.
    const result = await agent.generate("retry me");
    expect(result.text).toBe("stub-reply");
    expect(mock.reserveCalls).toHaveLength(2);
    const [first, second] = mock.reserveCalls;
    expect(second?.request.llmCallId).toBe(first?.request.llmCallId);
    expect(second?.request.decisionId).toBe(first?.request.decisionId);
    expect(second?.request.idempotencyKey).toBe(first?.request.idempotencyKey);
  }, 30_000);
});
