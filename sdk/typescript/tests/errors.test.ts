// Error hierarchy tests (SLICE 3 scope: tests.md §3.2 E-01..E-08, plus the
// SLICE 3 anchor on Connection/Decision discriminated subtypes per slice doc).

import { describe, expect, it, vi } from "vitest";

import {
  ApprovalBundleHotReloadedError,
  ApprovalDeniedError,
  ApprovalLapsedError,
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
  HandshakeError,
  MutationApplyFailed,
  SidecarUnavailable,
  SpendGuardConfigError,
  SpendGuardConnectionError,
  SpendGuardDecisionError,
  SpendGuardError,
} from "../src/index.js";

describe("error hierarchy — E-01 every class extends SpendGuardError + Error", () => {
  const cases: Array<[string, SpendGuardError]> = [
    ["SpendGuardError", new SpendGuardError("x")],
    ["SpendGuardConfigError", new SpendGuardConfigError("x")],
    ["SpendGuardConnectionError", new SpendGuardConnectionError("x")],
    ["SidecarUnavailable", new SidecarUnavailable("x")],
    ["HandshakeError", new HandshakeError("x")],
    ["DecisionDenied", new DecisionDenied("x", { decisionId: "d-1" })],
    ["DecisionStopped", new DecisionStopped("x", { decisionId: "d-1" })],
    ["DecisionSkipped", new DecisionSkipped("x", { decisionId: "d-1" })],
    [
      "ApprovalRequired",
      new ApprovalRequired("x", {
        decisionId: "d-1",
        approvalRequestId: "ap-1",
      }),
    ],
    ["ApprovalDeniedError", new ApprovalDeniedError("x", { decisionId: "d-1" })],
    ["ApprovalLapsedError", new ApprovalLapsedError("x", { decisionId: "d-1", state: "pending" })],
    [
      "ApprovalBundleHotReloadedError",
      new ApprovalBundleHotReloadedError("x", {
        originalBundleHash: "aa",
        currentBundleHash: "bb",
      }),
    ],
    ["MutationApplyFailed", new MutationApplyFailed("x")],
    ["SpendGuardDecisionError", new SpendGuardDecisionError("x")],
  ];

  for (const [name, instance] of cases) {
    it(`${name} is SpendGuardError + Error`, () => {
      expect(instance).toBeInstanceOf(SpendGuardError);
      expect(instance).toBeInstanceOf(Error);
      expect(instance.name).toBe(name);
    });
  }
});

describe("error hierarchy — discriminated subtypes per slice doc", () => {
  it("SpendGuardConfigError is a SpendGuardError but not a SidecarUnavailable", () => {
    const e = new SpendGuardConfigError("nope");
    expect(e).toBeInstanceOf(SpendGuardError);
    expect(e).not.toBeInstanceOf(SidecarUnavailable);
    expect(e).not.toBeInstanceOf(DecisionDenied);
  });

  it("SpendGuardConnectionError is a SpendGuardError but not a DecisionDenied", () => {
    const e = new SpendGuardConnectionError("nope");
    expect(e).toBeInstanceOf(SpendGuardError);
    expect(e).not.toBeInstanceOf(DecisionDenied);
    expect(e).not.toBeInstanceOf(SidecarUnavailable);
  });

  it("Decision subclasses chain: ApprovalRequired ⊂ DecisionDenied ⊂ SpendGuardError ⊂ Error", () => {
    const ap = new ApprovalRequired("x", {
      decisionId: "d",
      approvalRequestId: "a",
    });
    expect(ap).toBeInstanceOf(ApprovalRequired);
    expect(ap).toBeInstanceOf(DecisionDenied);
    expect(ap).toBeInstanceOf(SpendGuardError);
    expect(ap).toBeInstanceOf(Error);
  });

  it("DecisionStopped and DecisionSkipped are siblings under DecisionDenied", () => {
    const stop = new DecisionStopped("x", { decisionId: "d" });
    const skip = new DecisionSkipped("x", { decisionId: "d" });
    expect(stop).toBeInstanceOf(DecisionDenied);
    expect(skip).toBeInstanceOf(DecisionDenied);
    expect(stop).not.toBeInstanceOf(DecisionSkipped);
    expect(skip).not.toBeInstanceOf(DecisionStopped);
  });
});

describe("error hierarchy — statusCode const literals", () => {
  it("E-02: SidecarUnavailable.statusCode === 503", () => {
    const e = new SidecarUnavailable("x");
    expect(e.statusCode).toBe(503);
    // Compile-time: the const literal 503 narrows so e.statusCode is `503`
    // not `number`. We can't assert at runtime; the test above covers value.
  });

  it("E-03: DecisionDenied.statusCode === 403", () => {
    const e = new DecisionDenied("x", { decisionId: "d" });
    expect(e.statusCode).toBe(403);
  });

  it("ApprovalRequired inherits DecisionDenied.statusCode 403", () => {
    const e = new ApprovalRequired("x", {
      decisionId: "d",
      approvalRequestId: "a",
    });
    expect(e.statusCode).toBe(403);
  });
});

describe("error hierarchy — E-04 ApprovalRequired.resume delegates", () => {
  it("delegates to client.resumeAfterApproval with the right args", async () => {
    const resumeSpy = vi.fn(async () => "delegated");
    const fakeClient = { resumeAfterApproval: resumeSpy };
    const ap = new ApprovalRequired("x", {
      decisionId: "decision-A",
      approvalRequestId: "approval-B",
      tenantId: "tenant-C",
    });
    const result = await ap.resume(fakeClient);
    expect(result).toBe("delegated");
    expect(resumeSpy).toHaveBeenCalledTimes(1);
    expect(resumeSpy).toHaveBeenCalledWith({
      approvalId: "approval-B",
      tenantId: "tenant-C",
      decisionId: "decision-A",
    });
  });

  it("uses empty tenantId when not set on the error", async () => {
    const resumeSpy = vi.fn(async () => undefined);
    const fakeClient = { resumeAfterApproval: resumeSpy };
    const ap = new ApprovalRequired("x", {
      decisionId: "d",
      approvalRequestId: "a",
    });
    await ap.resume(fakeClient);
    expect(resumeSpy).toHaveBeenCalledWith(expect.objectContaining({ tenantId: "" }));
  });
});

describe("error hierarchy — E-05 ApprovalLapsedError reason_codes", () => {
  it.each(["pending", "expired", "cancelled", "unknown"] as const)(
    "state=%s prepends approval_lapsed_<state> to reasonCodes",
    (state) => {
      const e = new ApprovalLapsedError("x", {
        decisionId: "d",
        state,
        reasonCodes: ["budget"],
      });
      expect(e.reasonCodes[0]).toBe(`approval_lapsed_${state}`);
      expect(e.reasonCodes).toContain("budget");
    },
  );
});

describe("error hierarchy — ApprovalDeniedError reason_codes", () => {
  it("prepends approval_denied", () => {
    const e = new ApprovalDeniedError("x", {
      decisionId: "d",
      reasonCodes: ["other"],
    });
    expect(e.reasonCodes[0]).toBe("approval_denied");
    expect(e.reasonCodes).toContain("other");
  });

  it("carries approverSubject and approverReason when provided", () => {
    const e = new ApprovalDeniedError("x", {
      decisionId: "d",
      approverSubject: "alice@example.com",
      approverReason: "budget too high",
    });
    expect(e.approverSubject).toBe("alice@example.com");
    expect(e.approverReason).toBe("budget too high");
  });
});

describe("error hierarchy — E-06 name preserved across JSON.stringify", () => {
  it("name field is enumerable for SidecarUnavailable", () => {
    const e = new SidecarUnavailable("test message");
    const json = JSON.parse(JSON.stringify(e)) as Record<string, unknown>;
    expect(json.name).toBe("SidecarUnavailable");
  });

  it("name field is enumerable for DecisionStopped", () => {
    const e = new DecisionStopped("x", { decisionId: "d" });
    const json = JSON.parse(JSON.stringify(e)) as Record<string, unknown>;
    expect(json.name).toBe("DecisionStopped");
  });
});

describe("error hierarchy — E-07 cause is forwarded", () => {
  it("SpendGuardError carries cause when provided", () => {
    const root = new Error("root cause");
    const e = new SpendGuardError("wrapper", { cause: root });
    expect((e as Error & { cause?: unknown }).cause).toBe(root);
  });

  it("SidecarUnavailable carries cause when provided", () => {
    const root = new Error("ECONNREFUSED");
    const e = new SidecarUnavailable("dial failed", { cause: root });
    expect((e as Error & { cause?: unknown }).cause).toBe(root);
  });

  it("DecisionDenied carries cause when provided", () => {
    const root = new Error("audit fetch failed");
    const e = new DecisionDenied("wrapper", { decisionId: "d" }, { cause: root });
    expect((e as Error & { cause?: unknown }).cause).toBe(root);
  });
});

describe("error hierarchy — E-08 routing", () => {
  it("SpendGuardConfigError and MutationApplyFailed both route via SpendGuardError catch", () => {
    const errs: SpendGuardError[] = [new SpendGuardConfigError("x"), new MutationApplyFailed("x")];
    for (const err of errs) {
      try {
        throw err;
      } catch (caught) {
        expect(caught).toBeInstanceOf(SpendGuardError);
      }
    }
  });

  it("ApprovalBundleHotReloadedError carries the two bundle hashes", () => {
    const e = new ApprovalBundleHotReloadedError("rotated", {
      originalBundleHash: "00abc",
      currentBundleHash: "11def",
    });
    expect(e.originalBundleHash).toBe("00abc");
    expect(e.currentBundleHash).toBe("11def");
    expect(e).toBeInstanceOf(SpendGuardError);
  });

  it("DecisionDenied carries decisionId / reasonCodes / auditDecisionEventId / matchedRuleIds", () => {
    const e = new DecisionDenied("x", {
      decisionId: "d-9",
      reasonCodes: ["budget.over_threshold"],
      auditDecisionEventId: "ev-1",
      matchedRuleIds: ["rule-A"],
    });
    expect(e.decisionId).toBe("d-9");
    expect(e.reasonCodes).toEqual(["budget.over_threshold"]);
    expect(e.auditDecisionEventId).toBe("ev-1");
    expect(e.matchedRuleIds).toEqual(["rule-A"]);
  });
});
