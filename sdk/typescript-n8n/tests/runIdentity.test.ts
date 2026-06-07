// D37 unit tests — resolveRunIdentity.
// Covers RI-01..RI-07 per tests.md §3.4.

import { describe, expect, it } from "vitest";
import { resolveRunIdentity } from "../src/runIdentity";
import { makeMockContext } from "./_support/mockN8nContext";

const EXEC = "exec-uuid-abc-123";
const NODE = "AI Agent";

describe("resolveRunIdentity", () => {
  it("RI-01 executionId mode → '{executionId}:{nodeName}'", () => {
    const ctx = makeMockContext({ executionId: EXEC, nodeName: NODE });
    const out = resolveRunIdentity({
      ctx,
      params: { runIdSource: "executionId", customRunId: "" },
      itemIndex: 0,
    });
    expect(out.runId).toBe(`${EXEC}:${NODE}`);
    expect(out.sessionId).toBe(EXEC);
    expect(out.stepId).toBe(NODE);
  });

  it("RI-02 nodeName mode → runId === nodeName", () => {
    const ctx = makeMockContext({ executionId: EXEC, nodeName: NODE });
    const out = resolveRunIdentity({
      ctx,
      params: { runIdSource: "nodeName", customRunId: "" },
      itemIndex: 0,
    });
    expect(out.runId).toBe(NODE);
    expect(out.sessionId).toBe(EXEC);
    expect(out.stepId).toBe(NODE);
  });

  it("RI-03 custom mode with non-empty customRunId → uses custom", () => {
    const ctx = makeMockContext({ executionId: EXEC, nodeName: NODE });
    const out = resolveRunIdentity({
      ctx,
      params: { runIdSource: "custom", customRunId: "my-tenant-run-1" },
      itemIndex: 0,
    });
    expect(out.runId).toBe("my-tenant-run-1");
    expect(out.sessionId).toBe(EXEC);
    expect(out.stepId).toBe(NODE);
  });

  it("RI-04 custom mode with empty customRunId → falls back to executionId mode", () => {
    const ctx = makeMockContext({ executionId: EXEC, nodeName: NODE });
    const out = resolveRunIdentity({
      ctx,
      params: { runIdSource: "custom", customRunId: "" },
      itemIndex: 0,
    });
    expect(out.runId).toBe(`${EXEC}:${NODE}`);
  });

  it("RI-05 sessionId always equals executionId regardless of source", () => {
    const ctx = makeMockContext({ executionId: EXEC, nodeName: NODE });
    for (const source of ["executionId", "nodeName", "custom"] as const) {
      const out = resolveRunIdentity({
        ctx,
        params: { runIdSource: source, customRunId: "abc" },
        itemIndex: 0,
      });
      expect(out.sessionId).toBe(EXEC);
    }
  });

  it("RI-06 stepId always equals nodeName regardless of source", () => {
    const ctx = makeMockContext({ executionId: EXEC, nodeName: NODE });
    for (const source of ["executionId", "nodeName", "custom"] as const) {
      const out = resolveRunIdentity({
        ctx,
        params: { runIdSource: source, customRunId: "abc" },
        itemIndex: 0,
      });
      expect(out.stepId).toBe(NODE);
    }
  });

  it("RI-07 UUID-shaped executionId forwarded byte-identical", () => {
    const UUID_EXEC = "0193fd56-1e2c-72a6-9c48-3a1f0fa9b51b";
    const ctx = makeMockContext({ executionId: UUID_EXEC, nodeName: NODE });
    const out = resolveRunIdentity({
      ctx,
      params: { runIdSource: "executionId", customRunId: "" },
      itemIndex: 0,
    });
    expect(out.sessionId).toBe(UUID_EXEC);
    expect(out.runId).toBe(`${UUID_EXEC}:${NODE}`);
  });
});
