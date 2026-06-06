// COV_S05_07 SLICE 7 R2 — `withRunPlan` + `currentRunPlan` LOCKED-shape matrix.
//
// Spec coverage:
//   - design.md §4.7 lines 290-303 (RunPlan / withRunPlan / currentRunPlan
//     signatures — CURRIED form, RunPlan | null sentinel).
//   - implementation.md §9 lines 1103-1138 (skeleton + outer-wins nesting +
//     TypeError validation messages).
//   - review-standards §1.2 P0 verbatim signature gate.
//   - review-standards §1.5 P0 alias identity (reserve === requestDecision)
//     across the plannedStepsHint auto-fold path.
//   - review-standards §8 run-plan correctness gates §8.1-§8.5.
//
// R2 retires the SLICE 7 R1 IDENTITY-propagation shape (runId / parentRunId /
// traceparent / tracestate / budgetGrantJti) and ships the LOCKED BUDGET-HINT
// shape ({plannedCalls, plannedTools}). See `runPlan.ts` R2 retirement note
// and `docs/slices/COV_S05_07_d05_run_plan.md` R2 amendment footer.
//
// Each test runs inside its own scope so an outer `withRunPlan` cannot bleed
// into a subsequent test through AsyncLocalStorage. We add explicit "no plan"
// guards in the leading tests so a regression in scope cleanup surfaces
// immediately.

import { describe, expect, it } from "vitest";

import type { DecisionRequest as ProtoDecisionRequest } from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import { DecisionResponse_Decision } from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import { type RunPlan, SpendGuardClient, currentRunPlan, withRunPlan } from "../src/index.js";
import { MockSidecar } from "./_support/mockSidecar.js";

// ── §1. Validation — §8.4 review-standards ────────────────────────────────

describe("withRunPlan validation — §8.4 review-standards", () => {
  it("accepts plannedCalls=0, plannedTools=0", () => {
    expect(() => withRunPlan({ plannedCalls: 0, plannedTools: 0 }, async () => {})).not.toThrow();
  });

  it("accepts plannedTools omitted (defaults to 0)", () => {
    expect(() => withRunPlan({ plannedCalls: 5 }, async () => {})).not.toThrow();
  });

  it("rejects plannedCalls=-1 with TypeError synchronously", () => {
    expect(() => withRunPlan({ plannedCalls: -1, plannedTools: 0 }, async () => {})).toThrow(
      TypeError,
    );
  });

  it("rejects plannedCalls=1.5 with TypeError synchronously", () => {
    expect(() => withRunPlan({ plannedCalls: 1.5, plannedTools: 0 }, async () => {})).toThrow(
      TypeError,
    );
  });

  it("rejects plannedTools=-1 with TypeError synchronously", () => {
    expect(() => withRunPlan({ plannedCalls: 0, plannedTools: -1 }, async () => {})).toThrow(
      TypeError,
    );
  });

  it("rejects plannedTools=1.5 with TypeError synchronously", () => {
    expect(() => withRunPlan({ plannedCalls: 0, plannedTools: 1.5 }, async () => {})).toThrow(
      TypeError,
    );
  });

  it("throws TypeError BEFORE returning the wrapped fn (validation at HOF time)", () => {
    let fnInvoked = false;
    const target = async () => {
      fnInvoked = true;
    };
    // The throw must happen on the withRunPlan call itself, not when the
    // returned wrapper is invoked.
    expect(() => withRunPlan({ plannedCalls: -1 }, target)).toThrow(TypeError);
    expect(fnInvoked).toBe(false);
  });
});

// ── §2. Curried form + currentRunPlan scope — §1.2 + §4.7 ─────────────────

describe("withRunPlan curried form — design.md §4.7", () => {
  it("returns a callable wrapper that does NOT invoke fn synchronously", async () => {
    let invocations = 0;
    const target = async () => {
      invocations += 1;
      return "ok";
    };
    const wrapped = withRunPlan({ plannedCalls: 1, plannedTools: 0 }, target);
    expect(typeof wrapped).toBe("function");
    expect(invocations).toBe(0);
    const result = await wrapped();
    expect(invocations).toBe(1);
    expect(result).toBe("ok");
  });

  it("wrapped fn returns a Promise even when target is sync", async () => {
    const wrapped = withRunPlan({ plannedCalls: 0, plannedTools: 0 }, () => 42);
    const got = wrapped();
    expect(got).toBeInstanceOf(Promise);
    await expect(got).resolves.toBe(42);
  });

  it("forwards positional arguments verbatim to fn", async () => {
    const wrapped = withRunPlan(
      { plannedCalls: 0, plannedTools: 0 },
      async (a: number, b: number, c: number) => a + b + c,
    );
    await expect(wrapped(1, 2, 3)).resolves.toBe(6);
  });

  it("inside withRunPlan, currentRunPlan returns the active plan", async () => {
    let observed: RunPlan | null = null;
    const wrapped = withRunPlan({ plannedCalls: 7, plannedTools: 3 }, async () => {
      observed = currentRunPlan();
    });
    await wrapped();
    expect(observed).not.toBeNull();
    expect(observed).toEqual({ plannedCalls: 7, plannedTools: 3 });
  });

  it("outside withRunPlan, currentRunPlan returns null (NOT undefined)", () => {
    const got = currentRunPlan();
    expect(got).toBeNull();
    expect(got).not.toBeUndefined();
  });

  it("after withRunPlan completes, currentRunPlan returns to null", async () => {
    const wrapped = withRunPlan({ plannedCalls: 1, plannedTools: 1 }, async () => {});
    await wrapped();
    expect(currentRunPlan()).toBeNull();
  });
});

// ── §3. Nesting — §8.2 OUTER WINS ─────────────────────────────────────────

describe("withRunPlan nesting — §8.2 outer wins", () => {
  it("nested withRunPlan: OUTER plan wins; inner is a no-op for storage", async () => {
    const outerPlan = { plannedCalls: 10, plannedTools: 5 };
    const innerPlan = { plannedCalls: 99, plannedTools: 99 };
    let observedInside: RunPlan | null = null;

    const innerWrapped = withRunPlan(innerPlan, async () => {
      observedInside = currentRunPlan();
    });
    const outerWrapped = withRunPlan(outerPlan, async () => {
      await innerWrapped();
    });

    await outerWrapped();
    expect(observedInside).toEqual(outerPlan);
  });

  it("after nested inner returns, OUTER plan is still visible at outer scope", async () => {
    const outerPlan = { plannedCalls: 2, plannedTools: 1 };
    const innerPlan = { plannedCalls: 50, plannedTools: 50 };
    let outerObservedAfterInner: RunPlan | null = null;

    const innerWrapped = withRunPlan(innerPlan, async () => {
      // no-op
    });
    const outerWrapped = withRunPlan(outerPlan, async () => {
      await innerWrapped();
      outerObservedAfterInner = currentRunPlan();
    });

    await outerWrapped();
    expect(outerObservedAfterInner).toEqual(outerPlan);
  });
});

// ── §4. Async parity + concurrency ────────────────────────────────────────

describe("withRunPlan async parity + concurrency", () => {
  it("await inside fn preserves the plan across the suspension", async () => {
    const observed: (RunPlan | null)[] = [];
    const wrapped = withRunPlan({ plannedCalls: 4, plannedTools: 2 }, async () => {
      observed.push(currentRunPlan());
      await new Promise((resolve) => setTimeout(resolve, 1));
      observed.push(currentRunPlan());
      await Promise.resolve();
      observed.push(currentRunPlan());
    });
    await wrapped();
    expect(observed).toEqual([
      { plannedCalls: 4, plannedTools: 2 },
      { plannedCalls: 4, plannedTools: 2 },
      { plannedCalls: 4, plannedTools: 2 },
    ]);
  });

  it("concurrent withRunPlan via Promise.all does not bleed plans", async () => {
    const observed: Record<string, RunPlan | null> = {};
    const wrappedA = withRunPlan({ plannedCalls: 1, plannedTools: 1 }, async () => {
      await new Promise((resolve) => setTimeout(resolve, 2));
      observed.a = currentRunPlan();
    });
    const wrappedB = withRunPlan({ plannedCalls: 9, plannedTools: 9 }, async () => {
      await new Promise((resolve) => setTimeout(resolve, 1));
      observed.b = currentRunPlan();
    });
    await Promise.all([wrappedA(), wrappedB()]);
    expect(observed.a).toEqual({ plannedCalls: 1, plannedTools: 1 });
    expect(observed.b).toEqual({ plannedCalls: 9, plannedTools: 9 });
  });

  it("does NOT mutate the caller-supplied plan object", async () => {
    const callerPlan = { plannedCalls: 3, plannedTools: 4 } as {
      plannedCalls: number;
      plannedTools: number;
    };
    const wrapped = withRunPlan(callerPlan, async () => {});
    await wrapped();
    expect(callerPlan).toEqual({ plannedCalls: 3, plannedTools: 4 });
  });
});

// ── §5. Wire integration — §8.5 plannedStepsHint auto-fold ────────────────

function reserveReq(
  overrides: Partial<Parameters<SpendGuardClient["reserve"]>[0]> = {},
): Parameters<SpendGuardClient["reserve"]>[0] {
  return {
    trigger: "LLM_CALL_PRE" as const,
    runId: "run-runplan",
    stepId: "step-runplan",
    llmCallId: "llm-runplan",
    decisionId: "d-runplan",
    route: "openai|gpt-4o-mini",
    projectedClaims: [
      {
        scopeId: "tenant/test/global",
        amountAtomic: "1000",
        unit: { unit: "USD_MICROS", denomination: 1 },
      },
    ],
    idempotencyKey: "sg-runplan-deadbeef",
    ...overrides,
  };
}

describe("SpendGuardClient.reserve plannedStepsHint — §8.5", () => {
  it("plannedStepsHint = plannedCalls + plannedTools when plan is active", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const wrapped = withRunPlan({ plannedCalls: 7, plannedTools: 3 }, async () => {
        await client.reserve(reserveReq());
      });
      await wrapped();
      expect(captured).not.toBeNull();
      expect((captured as unknown as ProtoDecisionRequest).plannedStepsHint).toBe(10);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("plannedStepsHint = plannedCalls only when plannedTools omitted (defaults to 0)", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      const wrapped = withRunPlan({ plannedCalls: 4 }, async () => {
        await client.reserve(reserveReq());
      });
      await wrapped();
      expect((captured as unknown as ProtoDecisionRequest).plannedStepsHint).toBe(4);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("plannedStepsHint = 0 when no plan is active", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq());
      expect((captured as unknown as ProtoDecisionRequest).plannedStepsHint).toBe(0);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("alias identity holds: reserve === requestDecision (§1.5 P0)", async () => {
    // §1.5 P0 alias identity gate — both paths share the same method body, so
    // the auto-fold runs identically through either entry point.
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({ socketPath: mock.socketPath, tenantId: "t" });
      await client.connect();
      expect(client.reserve).toBe(client.requestDecision);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});
