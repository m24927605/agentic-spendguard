// adapter.test.ts — binding + error-translation unit suite.

import { RuntimeError } from "@botpress/sdk";
import { computePromptHash } from "@spendguard/sdk";
import { describe, expect, test } from "vitest";
import {
  DEFAULT_MAX_TOKENS,
  pickTenantId,
  resolveMaxTokens,
  resolveModel,
  toBindingFromActionInput,
} from "../src/adapter/binding.js";
import { codeFor, runtimeErrorCode, toRuntimeError } from "../src/adapter/errors.js";
import { DecisionDenied, SidecarUnavailable, SpendGuardConfigError } from "../src/reservation.js";
import { FIXTURE_TENANT_ID, makeConfig, makeCtx, makeGenerateContentInput } from "./_fixtures.js";

describe("binding adapter (AD01–AD03)", () => {
  test("AD01 binding falls back to bot id when tenantId empty", () => {
    const cfg = makeConfig({ tenantId: "" });
    expect(pickTenantId(cfg, "bot-test-1")).toBe("bot-test-1");
  });

  test("AD02 binding carries explicit tenant id", () => {
    const cfg = makeConfig({ tenantId: FIXTURE_TENANT_ID });
    expect(pickTenantId(cfg, "bot-test-1")).toBe(FIXTURE_TENANT_ID);
  });

  test("AD03 prompt-hash computed via D05 helper over bound messages", () => {
    const cfg = makeConfig({ tenantId: FIXTURE_TENANT_ID });
    const binding = toBindingFromActionInput({
      input: makeGenerateContentInput(),
      configuration: cfg,
      ctx: makeCtx(),
    });
    const promptText = JSON.stringify(
      binding.messages.map((m) => ({ role: m.role, content: m.content })),
    );
    const expected = computePromptHash(promptText, FIXTURE_TENANT_ID);
    expect(expected).toMatch(/^[0-9a-f]{64}$/);
    expect(computePromptHash(promptText, FIXTURE_TENANT_ID)).toBe(expected);
  });

  test("AD03b system prompt is prepended to bound messages", () => {
    const binding = toBindingFromActionInput({
      input: makeGenerateContentInput({ systemPrompt: "you are a budget guard" }),
      configuration: makeConfig(),
      ctx: makeCtx(),
    });
    expect(binding.messages[0]).toEqual({ role: "system", content: "you are a budget guard" });
    expect(binding.messages[1]).toEqual({ role: "user", content: "hello" });
  });

  test("AD03c model + maxTokens resolution honours input then provider default", () => {
    const cfg = makeConfig({ upstreamProvider: "anthropic" });
    // Explicit model id wins.
    expect(resolveModel(makeGenerateContentInput(), cfg)).toBe("gpt-4o-mini");
    // Omitted model -> provider default.
    expect(resolveModel(makeGenerateContentInput({ model: undefined }), cfg)).toBe(
      "claude-3-5-haiku-latest",
    );
    // Omitted maxTokens -> floor.
    expect(resolveMaxTokens(makeGenerateContentInput({ maxTokens: undefined }))).toBe(
      DEFAULT_MAX_TOKENS,
    );
    expect(resolveMaxTokens(makeGenerateContentInput({ maxTokens: 256 }))).toBe(256);
  });
});

describe("error translation (AD04–AD06)", () => {
  test("AD04 DecisionDenied -> RuntimeError carrying BUDGET_DENIED", () => {
    const err = new DecisionDenied("budget cap exceeded", ["BUDGET_EXCEEDED"]);
    const rt = toRuntimeError(err);
    expect(rt).toBeInstanceOf(RuntimeError);
    expect(rt.message).toContain("denied");
    expect(runtimeErrorCode(rt)).toBe("BUDGET_DENIED");
    expect(codeFor(err)).toBe("BUDGET_DENIED");
  });

  test("AD05 SidecarUnavailable -> RuntimeError carrying BUDGET_DEGRADED", () => {
    const err = new SidecarUnavailable("sidecar gone");
    const rt = toRuntimeError(err);
    expect(rt).toBeInstanceOf(RuntimeError);
    expect(rt.message).toContain("degraded");
    expect(runtimeErrorCode(rt)).toBe("BUDGET_DEGRADED");
    expect(codeFor(err)).toBe("BUDGET_DEGRADED");
  });

  test("AD06 SpendGuardConfigError -> RuntimeError carrying BUDGET_CONFIG", () => {
    const err = new SpendGuardConfigError("missing sidecarUrl");
    const rt = toRuntimeError(err);
    expect(rt).toBeInstanceOf(RuntimeError);
    expect(rt.message).toContain("config");
    expect(runtimeErrorCode(rt)).toBe("BUDGET_CONFIG");
    expect(codeFor(err)).toBe("BUDGET_CONFIG");
  });

  test("AD07 unknown error -> BUDGET_CONFIG fallback preserves message", () => {
    const rt = toRuntimeError(new Error("kaboom"));
    expect(rt).toBeInstanceOf(RuntimeError);
    expect(rt.message).toContain("kaboom");
    expect(runtimeErrorCode(rt)).toBe("BUDGET_CONFIG");
  });

  test("AD08 already-a-RuntimeError passes through unchanged", () => {
    const rt = new RuntimeError("already runtime");
    expect(toRuntimeError(rt)).toBe(rt);
  });
});
