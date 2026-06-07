// D37 — Public-surface lock test.
//
// Verifies the locked public-surface invariants from review-standards.md §2:
// node name, version, input/output types, credential name, properties shape.
// A failure here means the n8n loader would reject the node, or the
// audit-chain idempotency key derivation would silently drift.

import { type NodeConnectionType, NodeConnectionTypes } from "n8n-workflow";
import { describe, expect, it } from "vitest";
import { SpendGuardApi } from "../credentials/SpendGuardApi.credentials";
import { SpendGuardChatModel } from "../nodes/SpendGuardChatModel/SpendGuardChatModel.node";
import { VERSION, acquireClient, mapToNodeApiError, resolveRunIdentity } from "../src";

describe("Public surface lock", () => {
  it("Node internal name is spendGuardChatModel", () => {
    const node = new SpendGuardChatModel();
    expect(node.description.name).toBe("spendGuardChatModel");
  });

  it("Node displayName is 'SpendGuard Chat Model'", () => {
    expect(new SpendGuardChatModel().description.displayName).toBe("SpendGuard Chat Model");
  });

  it("Node version is integer 1", () => {
    expect(new SpendGuardChatModel().description.version).toBe(1);
  });

  it("Input[0].type === AiLanguageModel", () => {
    const inputs = new SpendGuardChatModel().description.inputs as Array<{
      type: NodeConnectionType;
    }>;
    expect(inputs[0]?.type).toBe(NodeConnectionTypes.AiLanguageModel);
  });

  it("Output[0].type === AiLanguageModel", () => {
    const outputs = new SpendGuardChatModel().description.outputs as Array<{
      type: NodeConnectionType;
    }>;
    expect(outputs[0]?.type).toBe(NodeConnectionTypes.AiLanguageModel);
  });

  it("Credential binding is { name: 'spendGuardApi', required: true }", () => {
    const node = new SpendGuardChatModel();
    expect(node.description.credentials).toEqual([{ name: "spendGuardApi", required: true }]);
  });

  it("Credential name is spendGuardApi", () => {
    expect(new SpendGuardApi().name).toBe("spendGuardApi");
  });

  it("Helpers are exported from the package barrel", () => {
    expect(typeof resolveRunIdentity).toBe("function");
    expect(typeof acquireClient).toBe("function");
    expect(typeof mapToNodeApiError).toBe("function");
    expect(VERSION).toMatch(/^\d+\.\d+\.\d+/);
  });

  it("Node properties[] is in the LOCKED canonical order", () => {
    const node = new SpendGuardChatModel();
    const names = node.description.properties.map((p) => p.name);
    expect(names).toEqual([
      "budgetIdOverride",
      "route",
      "runIdSource",
      "customRunId",
      "claimAmountAtomic",
      "unit",
    ]);
  });

  it("Codex metadata categories include AI", () => {
    const codex = new SpendGuardChatModel().description.codex;
    expect(codex?.categories).toContain("AI");
  });
});
