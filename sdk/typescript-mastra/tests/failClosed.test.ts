// COV_D38_02 — reserve fail-closed subset (tests.md TP-10, TP-13..TP-16,
// gate A3.4; slice doc NOTE 2: COV_D38_04 extends this file to the full §7
// matrix).
//
// Design §7 LOCKED rules 1-3: NO fail-open branch, NO env escape hatch,
// every reserve-path error aborts the step before the provider call.
//
// TP-10 is the V2 pin evidence (see src/processor.ts header): a REAL
// `@mastra/core` Agent with a recording stub model proves DENY ⇒ the agent
// rejects AND zero `doGenerate`/`doStream` invocations. The typed error
// itself is asserted at the hook boundary (TP-13..TP-16) where `instanceof`
// holds; Mastra 1.41.0's workflow engine serializes processor errors, so
// the consumer-facing rejection preserves the MESSAGE but not the class
// (recorded honestly in the V2 pin — a Mastra-runtime property, not an
// adapter degradation).

import { Agent } from "@mastra/core/agent";
import type { ProcessInputStepArgs, ProcessLLMResponseArgs } from "@mastra/core/processors";
import {
  ApprovalRequired,
  DecisionDenied,
  DecisionStopped,
  HandshakeError,
  SidecarUnavailable,
  SpendGuardError,
} from "@spendguard/sdk";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SpendGuardProcessor } from "../src/index.js";
import type { SpendGuardProcessorOptions } from "../src/index.js";
import { type DecisionPlan, MockSpendGuardClient } from "./_support/mockSidecar.js";
import { RecordingStubModel } from "./_support/stubModel.js";

let messageCounter = 0;

function dbMessage(text: string): Record<string, unknown> {
  messageCounter += 1;
  return {
    id: `msg-fc-${messageCounter}`,
    role: "user",
    createdAt: new Date(0),
    content: { format: 2, parts: [{ type: "text", text }] },
  };
}

function makeArgs(text: string): ProcessInputStepArgs {
  return {
    messages: [dbMessage(text)],
    stepNumber: 0,
    steps: [],
    systemMessages: [],
    state: {},
    retryCount: 0,
    abort: (reason?: string) => {
      throw new Error(`unexpected abort: ${reason ?? ""}`);
    },
  } as unknown as ProcessInputStepArgs;
}

function makeAgent(plan: DecisionPlan): {
  agent: Agent;
  mock: MockSpendGuardClient;
  stub: RecordingStubModel;
} {
  const mock = new MockSpendGuardClient({ defaultDecision: plan });
  const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-fc" });
  const stub = new RecordingStubModel();
  const agent = new Agent({
    id: "failclosed-agent",
    name: "failclosed-agent",
    instructions: "test agent",
    model: stub as never,
    inputProcessors: [guard],
  });
  return { agent, mock, stub };
}

describe("COV_D38_02 fail-closed reserve subset (TP-10, TP-13..TP-16)", () => {
  it("TP-10: DENY-before-inner-call — real Agent rejects, stub model records ZERO calls (pins V2)", async () => {
    const { agent, mock, stub } = makeAgent({ kind: "DENY" });

    await expect(agent.generate("please spend money")).rejects.toThrow(/mock budget denied/);

    // The observable contract (design §7.3): DENY ⇒ zero provider calls.
    expect(stub.doGenerateCalls).toBe(0);
    expect(stub.doStreamCalls).toBe(0);
    expect(stub.totalCalls).toBe(0);
    // Exactly one reserve was attempted and it rejected DecisionDenied.
    expect(mock.reserveCalls).toHaveLength(1);
    expect(mock.reserveCalls[0]?.rejected?.name).toBe("DecisionDenied");
    // No commit ever fires for a denied step.
    expect(mock.commitCalls).toHaveLength(0);
  }, 30_000);

  it("TP-13: SidecarUnavailable → step aborts; 0 model calls; error reachable via instanceof", async () => {
    // Hook boundary: the typed error propagates with class identity intact
    // (NO catch around client.reserve() — design §7 rule 1).
    const mock = new MockSpendGuardClient({
      defaultDecision: { kind: "SIDECAR_UNAVAILABLE" },
    });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp13" });
    await expect(guard.processInputStep(makeArgs("unavailable"))).rejects.toSatisfy(
      (err: unknown) => err instanceof SidecarUnavailable,
    );

    // Real-agent boundary: the step aborts and the provider is never called
    // (FAIL-CLOSED — the D04/D06 log-and-proceed degradation branch is
    // deliberately absent, design §7 rule 1).
    const { agent, stub } = makeAgent({ kind: "SIDECAR_UNAVAILABLE" });
    await expect(agent.generate("hello")).rejects.toThrow(/mock sidecar UDS gone/);
    expect(stub.totalCalls).toBe(0);
  }, 30_000);

  it("TP-14: DecisionStopped / ApprovalRequired propagate identically (both instanceof DecisionDenied)", async () => {
    const mockStop = new MockSpendGuardClient({ defaultDecision: { kind: "STOP" } });
    const guardStop = new SpendGuardProcessor({
      client: mockStop.client,
      tenantId: "tenant-tp14",
    });
    await expect(guardStop.processInputStep(makeArgs("stop me"))).rejects.toSatisfy(
      (err: unknown) => err instanceof DecisionStopped && err instanceof DecisionDenied,
    );

    const mockApproval = new MockSpendGuardClient({
      defaultDecision: { kind: "APPROVAL_REQUIRED" },
    });
    const guardApproval = new SpendGuardProcessor({
      client: mockApproval.client,
      tenantId: "tenant-tp14",
    });
    await expect(guardApproval.processInputStep(makeArgs("approve me"))).rejects.toSatisfy(
      (err: unknown) => err instanceof ApprovalRequired && err instanceof DecisionDenied,
    );
  });

  it("TP-15: HandshakeError propagates; 0 model calls", async () => {
    const mock = new MockSpendGuardClient({ defaultDecision: { kind: "HANDSHAKE_ERROR" } });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp15" });
    await expect(guard.processInputStep(makeArgs("handshake"))).rejects.toSatisfy(
      (err: unknown) => err instanceof HandshakeError,
    );

    const { agent, stub } = makeAgent({ kind: "HANDSHAKE_ERROR" });
    await expect(agent.generate("hello")).rejects.toThrow(/mock handshake missing/);
    expect(stub.totalCalls).toBe(0);
  }, 30_000);

  it("TP-16: no catch-and-continue in the reserve section — a thrown sentinel always rejects the step", async () => {
    // A plain (non-SpendGuard) sentinel error: if ANY catch existed around
    // client.reserve(), this would be swallowed and the step would proceed.
    const sentinel = new Error("sentinel-reserve-failure-7f3a");
    const mock = new MockSpendGuardClient({
      defaultDecision: { kind: "SENTINEL_ERROR", error: sentinel },
    });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp16" });

    // The exact sentinel instance propagates — nothing wrapped, nothing
    // swallowed, no inflight entry recorded after the failure.
    await expect(guard.processInputStep(makeArgs("sentinel"))).rejects.toBe(sentinel);

    // And through the real loop: the step rejects, the provider never runs.
    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "sentinel-agent",
      name: "sentinel-agent",
      instructions: "test agent",
      model: stub as never,
      inputProcessors: [new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp16" })],
    });
    await expect(agent.generate("hello")).rejects.toThrow(/sentinel-reserve-failure-7f3a/);
    expect(stub.totalCalls).toBe(0);
  }, 30_000);
});

// ── COV_D38_04 — FULL §7 fail-closed matrix ───────────────────────────────
//
// Every design §7 matrix row gets a test pinning BOTH halves of the LOCKED
// contract: (a) the typed error propagates (hook boundary: `instanceof`;
// agent boundary: message-match — the V2 residual split, gh #181), and
// (b) ZERO provider calls + ZERO commit RPCs. Rows already pinned by the
// COV_D38_02 subset above (DENY TP-10, SidecarUnavailable TP-13,
// HandshakeError TP-15, sentinel TP-16) are not duplicated; this block
// completes the matrix: constructor row, STOP / STOP_RUN_PROJECTION /
// REQUIRE_APPROVAL through the REAL agent loop, the sidecar-timeout flavour
// of SidecarUnavailable, and the "any other substrate error" →
// SpendGuardError row. The commit-path rows of §7 (rows 6-8) are pinned by
// TP-27/TP-28/TP-29 in processor.test.ts — they are POST-dispatch by design
// (§7.4 LOCKED asymmetry) and deliberately not re-tested as reserve rows.

describe("COV_D38_04 full §7 fail-closed matrix", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("§7 row 1: invalid options → TypeError at construction (matrix-level re-verify of TP-03)", () => {
    const mock = new MockSpendGuardClient();
    expect(() => new SpendGuardProcessor({} as never)).toThrow(TypeError);
    expect(() => new SpendGuardProcessor({ tenantId: "t" } as never)).toThrow(TypeError);
    expect(() => new SpendGuardProcessor({ client: mock.client, tenantId: "" })).toThrow(TypeError);
    expect(() => new SpendGuardProcessor(null as never)).toThrow(TypeError);
    // Construction failure means NO RPC ever fires.
    expect(mock.reserveCalls).toHaveLength(0);
    expect(mock.commitCalls).toHaveLength(0);
  });

  it("§7 row 2 (STOP): DecisionStopped through the real Agent — step aborts, zero provider calls, zero commits", async () => {
    const { agent, mock, stub } = makeAgent({ kind: "STOP" });
    await expect(agent.generate("stop me")).rejects.toThrow(/mock STOP terminal/);
    expect(stub.totalCalls).toBe(0);
    expect(mock.reserveCalls).toHaveLength(1);
    expect(mock.reserveCalls[0]?.rejected?.name).toBe("DecisionStopped");
    expect(mock.commitCalls).toHaveLength(0);
  }, 30_000);

  it("§7 row 2 (STOP_RUN_PROJECTION): DecisionStopped with the projection reason — same abort contract", async () => {
    // The sidecar surfaces STOP_RUN_PROJECTION decisions as DecisionStopped
    // carrying the projection reason code (substrate taxonomy) — the
    // adapter's contract is identical to plain STOP: propagate, zero calls.
    const mock = new MockSpendGuardClient({
      defaultDecision: { kind: "STOP", reasonCodes: ["STOP_RUN_PROJECTION"] },
    });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-srp" });
    await expect(guard.processInputStep(makeArgs("projected over"))).rejects.toSatisfy(
      (err: unknown) =>
        err instanceof DecisionStopped &&
        err instanceof DecisionDenied &&
        err.reasonCodes.includes("STOP_RUN_PROJECTION"),
    );

    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "srp-agent",
      name: "srp-agent",
      instructions: "test agent",
      model: stub as never,
      inputProcessors: [new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-srp" })],
    });
    await expect(agent.generate("projected over")).rejects.toThrow(/mock STOP terminal/);
    expect(stub.totalCalls).toBe(0);
    expect(mock.commitCalls).toHaveLength(0);
  }, 30_000);

  it("§7 row 3 (REQUIRE_APPROVAL): ApprovalRequired through the real Agent — aborts, zero provider calls (no resume helper in v1)", async () => {
    const { agent, mock, stub } = makeAgent({ kind: "APPROVAL_REQUIRED" });
    await expect(agent.generate("needs approval")).rejects.toThrow(/mock approval required/);
    expect(stub.totalCalls).toBe(0);
    expect(mock.reserveCalls).toHaveLength(1);
    expect(mock.reserveCalls[0]?.rejected?.name).toBe("ApprovalRequired");
    expect(mock.commitCalls).toHaveLength(0);
  }, 30_000);

  it("§7 row 4 (timeout flavour): SidecarUnavailable from a reserve TIMEOUT propagates identically", async () => {
    const mock = new MockSpendGuardClient({
      defaultDecision: { kind: "SIDECAR_UNAVAILABLE", message: "reserve deadline exceeded (2s)" },
    });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-timeout" });
    await expect(guard.processInputStep(makeArgs("slow sidecar"))).rejects.toSatisfy(
      (err: unknown) =>
        err instanceof SidecarUnavailable && /deadline exceeded/.test((err as Error).message),
    );

    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "timeout-agent",
      name: "timeout-agent",
      instructions: "test agent",
      model: stub as never,
      inputProcessors: [
        new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-timeout" }),
      ],
    });
    await expect(agent.generate("slow sidecar")).rejects.toThrow(/deadline exceeded/);
    expect(stub.totalCalls).toBe(0);
    expect(mock.commitCalls).toHaveLength(0);
  }, 30_000);

  it("§7 row 5: any other substrate error (SpendGuardError) → step aborts FAIL-CLOSED, zero provider calls", async () => {
    const substrateError = new SpendGuardError("substrate internal: unexpected wire frame");
    const mock = new MockSpendGuardClient({
      defaultDecision: { kind: "SENTINEL_ERROR", error: substrateError },
    });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-sge" });
    await expect(guard.processInputStep(makeArgs("substrate boom"))).rejects.toBe(substrateError);

    const stub = new RecordingStubModel();
    const agent = new Agent({
      id: "sge-agent",
      name: "sge-agent",
      instructions: "test agent",
      model: stub as never,
      inputProcessors: [new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-sge" })],
    });
    await expect(agent.generate("substrate boom")).rejects.toThrow(/unexpected wire frame/);
    expect(stub.totalCalls).toBe(0);
    expect(mock.commitCalls).toHaveLength(0);
  }, 30_000);

  it("matrix invariant: a failed reserve leaves NO inflight entry — a later commit hook warns + no-ops", async () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    const mock = new MockSpendGuardClient({ defaultDecision: { kind: "DENY" } });
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-noentry" });
    const state: Record<string, unknown> = {};

    const argsBag = {
      ...makeArgs("deny then settle"),
      state,
    } as unknown as ProcessInputStepArgs;
    await expect(guard.processInputStep(argsBag)).rejects.toSatisfy(
      (err: unknown) => err instanceof DecisionDenied,
    );
    // The throw happened BEFORE the inflight push and the state stash, so a
    // (hypothetical) commit-hook delivery finds nothing to settle.
    expect(state).toEqual({});
    await guard.processLLMResponse({
      chunks: [],
      state,
      stepNumber: 0,
      steps: [],
      retryCount: 0,
      fromCache: false,
      abort: () => {
        throw new Error("unexpected abort");
      },
    } as unknown as ProcessLLMResponseArgs);
    expect(mock.commitCalls).toHaveLength(0);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining("no inflight entry"));
  });

  it("TP-04 matrix-level re-verify: no fail-open knob exists on the options surface (runtime probe)", async () => {
    // The authoritative type-level gate lives in lockedSurface.test.ts; the
    // §7 matrix re-verifies at the matrix level that the FULLY-POPULATED
    // options literal exposes no enforcement-weakening key.
    const mock = new MockSpendGuardClient();
    const fullOptions: Required<SpendGuardProcessorOptions> = {
      client: mock.client,
      tenantId: "tenant-matrix-tp04",
      budgetId: "budget-matrix",
      unitId: "unit-matrix",
      route: "mastra-llm",
      defaultBudgetMicrosCap: 1n,
      claimEstimator: () => [],
      runIdProvider: () => "run-matrix",
      // §6.7 amendment #3 (2026-06-11): `pricing` is part of the §5 surface.
      pricing: { pricingVersion: "v-matrix", pricingHash: new Uint8Array([1]) },
    };
    for (const forbidden of ["failOpen", "degradeOnUnavailable", "enforcementMode"]) {
      expect(Object.keys(fullOptions)).not.toContain(forbidden);
    }
    // And no env escape hatch (§7 LOCKED rule 2): construction with
    // SPENDGUARD_DISABLE set still reserves (and still fail-closes).
    const saved = process.env.SPENDGUARD_DISABLE;
    process.env.SPENDGUARD_DISABLE = "1";
    try {
      const denyMock = new MockSpendGuardClient({ defaultDecision: { kind: "DENY" } });
      const guard = new SpendGuardProcessor({ client: denyMock.client, tenantId: "tenant-env" });
      await expect(guard.processInputStep(makeArgs("env escape probe"))).rejects.toSatisfy(
        (err: unknown) => err instanceof DecisionDenied,
      );
      expect(denyMock.reserveCalls).toHaveLength(1);
    } finally {
      if (saved === undefined) {
        delete process.env.SPENDGUARD_DISABLE;
      } else {
        process.env.SPENDGUARD_DISABLE = saved;
      }
    }
  });
});
