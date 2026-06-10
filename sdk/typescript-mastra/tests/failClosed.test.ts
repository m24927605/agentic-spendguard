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
import type { ProcessInputStepArgs } from "@mastra/core/processors";
import {
  ApprovalRequired,
  DecisionDenied,
  DecisionStopped,
  HandshakeError,
  SidecarUnavailable,
} from "@spendguard/sdk";
import { describe, expect, it } from "vitest";
import { SpendGuardProcessor } from "../src/index.js";
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
