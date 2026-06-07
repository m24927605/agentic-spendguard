// Flowise canvas-surface lock — covers acceptance.md A2.5 (M-01..M-08)
// + A7.1 / A7.2. Any rename / reorder / type change to a node input is a
// breaking canvas-builder UX change; locking the surface here forces the
// author to ship a 1.x → 1.y bump consciously.

import { describe, expect, it } from "vitest";

import { SpendGuardChatModelWrapper } from "../src/nodes/SpendGuardChatModelWrapper.js";

describe("Flowise INode manifest — public canvas surface", () => {
  const node = new SpendGuardChatModelWrapper();

  it("M-01 label is exactly the public-docs value", () => {
    expect(node.label).toBe("SpendGuard ChatModel Wrapper");
  });

  it("M-02 name is exactly 'spendGuardChatModelWrapper'", () => {
    expect(node.name).toBe("spendGuardChatModelWrapper");
  });

  it("M-03 type and version pin the BaseChatModel anchor + version 1.0", () => {
    expect(node.type).toBe("BaseChatModel");
    expect(node.version).toBe(1.0);
  });

  it("M-04 category is 'Spend Guard' (drives canvas sidebar grouping)", () => {
    expect(node.category).toBe("Spend Guard");
  });

  it("M-05 baseClasses include BOTH BaseChatModel and BaseLanguageModel", () => {
    expect(node.baseClasses).toContain("BaseChatModel");
    expect(node.baseClasses).toContain("BaseLanguageModel");
  });

  it("M-06 icon is the bundled spendguard.svg", () => {
    expect(node.icon).toBe("spendguard.svg");
  });

  it("M-07 input ordering is locked — 8 inputs in known order", () => {
    const names = node.inputs.map((i) => i.name);
    expect(names).toEqual([
      "chatModel",
      "tenantId",
      "budgetId",
      "windowInstanceId",
      "unit",
      "sidecarUds",
      "route",
      "claimEstimatorJson",
    ]);
  });

  it("M-08 required inputs (chatModel, tenantId, budgetId, windowInstanceId) are NOT optional", () => {
    const requiredNames = ["chatModel", "tenantId", "budgetId", "windowInstanceId"];
    for (const name of requiredNames) {
      const input = node.inputs.find((i) => i.name === name);
      expect(input).toBeDefined();
      expect(input?.optional).not.toBe(true);
    }

    const optionalNames = ["sidecarUds", "route", "claimEstimatorJson"];
    for (const name of optionalNames) {
      const input = node.inputs.find((i) => i.name === name);
      expect(input?.optional).toBe(true);
    }

    const unitInput = node.inputs.find((i) => i.name === "unit");
    expect(unitInput?.default).toBe("usd_micros");
  });

  it("snapshot — inputs array (rename or reorder will break this)", () => {
    expect(node.inputs).toMatchSnapshot();
  });
});
