// D37 unit tests — mapToNodeApiError.
// Covers ER-01..ER-10 per tests.md §3.5.

import {
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
  HandshakeError,
  SidecarUnavailable,
} from "@spendguard/sdk";
import { NodeApiError } from "n8n-workflow";
import type { INode } from "n8n-workflow";
import { describe, expect, it } from "vitest";
import { mapToNodeApiError } from "../src/errors";

const NODE: INode = {
  id: "n8n-node-1",
  name: "AI Agent",
  typeVersion: 1,
  type: "n8n-nodes-base.spendGuardChatModel",
  position: [0, 0],
  parameters: {},
};

describe("mapToNodeApiError", () => {
  it("ER-01 DecisionStopped → NodeApiError 403", () => {
    const err = Object.assign(
      new DecisionStopped("stopped", {
        decisionId: "dec-stopped-1",
        reasonCodes: ["budget_exhausted"],
      } as never),
      { reasonCodes: ["budget_exhausted"] },
    );
    const out = mapToNodeApiError(NODE, err);
    expect(out).toBeInstanceOf(NodeApiError);
    expect(out.message).toMatch(/SpendGuard denied/);
  });

  it("ER-02 DecisionDenied → NodeApiError 403", () => {
    const err = Object.assign(
      new DecisionDenied("denied", {
        decisionId: "dec-1",
        reasonCodes: ["budget_exceeded"],
      } as never),
      { reasonCodes: ["budget_exceeded"] },
    );
    const out = mapToNodeApiError(NODE, err);
    expect(out).toBeInstanceOf(NodeApiError);
    expect(out.message).toContain("budget_exceeded");
  });

  it("ER-03 DecisionSkipped → NodeApiError 403", () => {
    const err = Object.assign(
      new DecisionSkipped("skipped", {
        decisionId: "dec-skip-1",
        reasonCodes: ["paused"],
      } as never),
      { reasonCodes: ["paused"] },
    );
    const out = mapToNodeApiError(NODE, err);
    expect(out).toBeInstanceOf(NodeApiError);
    expect(out.message).toMatch(/SpendGuard denied/);
  });

  it("ER-04 ApprovalRequired → NodeApiError 428 with approvalRequestId", () => {
    const err = new ApprovalRequired("approval required", {
      decisionId: "dec-2",
      approvalRequestId: "approval-xyz-1",
    });
    const out = mapToNodeApiError(NODE, err);
    expect(out).toBeInstanceOf(NodeApiError);
    expect(out.description ?? "").toContain("approval-xyz-1");
  });

  it("ER-05 SidecarUnavailable → NodeApiError 503", () => {
    const err = new SidecarUnavailable("uds dial failed");
    const out = mapToNodeApiError(NODE, err);
    expect(out).toBeInstanceOf(NodeApiError);
    expect(out.message).toMatch(/sidecar unavailable/i);
  });

  it("ER-06 HandshakeError → NodeApiError 502", () => {
    const err = new HandshakeError("handshake mismatched");
    const out = mapToNodeApiError(NODE, err);
    expect(out).toBeInstanceOf(NodeApiError);
    expect(out.message).toMatch(/handshake failed/i);
  });

  it("ER-07 Generic Error → NodeApiError passthrough", () => {
    const err = new Error("some other failure");
    const out = mapToNodeApiError(NODE, err);
    expect(out).toBeInstanceOf(NodeApiError);
  });

  it("ER-08 null input does not crash", () => {
    const out = mapToNodeApiError(NODE, null);
    expect(out).toBeInstanceOf(NodeApiError);
    expect(out.message).toMatch(/empty error/);
  });

  it("ER-09 empty reasonCodes → message reads 'SpendGuard denied: decision_denied'", () => {
    const err = Object.assign(
      new DecisionDenied("denied", {
        decisionId: "dec-empty",
        reasonCodes: [],
      } as never),
      { reasonCodes: [] },
    );
    const out = mapToNodeApiError(NODE, err);
    expect(out.message).toContain("decision_denied");
  });

  it("ER-10 missing auditDecisionEventId → description reads '(pending)'", () => {
    const err = Object.assign(
      new DecisionDenied("denied", {
        decisionId: "dec-3",
        reasonCodes: ["x"],
      } as never),
      { reasonCodes: ["x"], decisionId: "dec-3" },
    );
    const out = mapToNodeApiError(NODE, err);
    expect(out.description ?? "").toContain("(pending)");
  });

  it("undefined input does not crash", () => {
    const out = mapToNodeApiError(NODE, undefined);
    expect(out).toBeInstanceOf(NodeApiError);
  });
});
