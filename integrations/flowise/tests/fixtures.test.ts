// Fixture structural assertions — covers acceptance.md A2.7.
//
// The two committed chatflow fixtures encode the canvas wiring an
// operator would build by hand. The unit tier asserts shape so the
// E2E tier can rely on stable IDs and structure.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const fixturesDir = join(here, "_fixtures");

function loadChatflow(name: string): {
  nodes: Array<{ id: string; data: { name: string; inputs: Record<string, unknown> } }>;
  edges: Array<{ source: string; target: string }>;
} {
  const raw = readFileSync(join(fixturesDir, name), "utf-8");
  return JSON.parse(raw);
}

describe("Flowise chatflow fixtures", () => {
  it("chatflow_minimal.json — three nodes, two edges, wrapper sits between ChatOpenAI and Chain", () => {
    const flow = loadChatflow("chatflow_minimal.json");
    expect(flow.nodes).toHaveLength(3);
    expect(flow.edges).toHaveLength(2);
    const nodeNames = flow.nodes.map((n) => n.data.name).sort();
    expect(nodeNames).toEqual(["chatOpenAI", "conversationChain", "spendGuardChatModelWrapper"]);
    const wrapperInputs = flow.nodes.find((n) => n.data.name === "spendGuardChatModelWrapper")?.data
      .inputs;
    expect(wrapperInputs?.tenantId).toBe("00000000-0000-4000-8000-000000000001");
    expect(wrapperInputs?.budgetId).toBe("44444444-4444-4444-8444-444444444444");
  });

  it("chatflow_deny.json — same shape but claimEstimatorJson forces a DENY", () => {
    const flow = loadChatflow("chatflow_deny.json");
    expect(flow.nodes).toHaveLength(3);
    const wrapperInputs = flow.nodes.find((n) => n.data.name === "spendGuardChatModelWrapper")?.data
      .inputs;
    const claimJson = wrapperInputs?.claimEstimatorJson;
    expect(typeof claimJson).toBe("string");
    const parsed = JSON.parse(claimJson as string);
    expect(parsed.amountAtomic).toBe("999999999999");
    expect(parsed.scopeId).toBe("deny-test");
  });
});
