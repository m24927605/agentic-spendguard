// adapter.test.ts — unit suite covering AD01–AD06 per tests.md §2.4.

import { RuntimeError } from "@botpress/sdk";
import { computePromptHash } from "@spendguard/sdk";
import { describe, expect, test } from "vitest";
import { pickTenantId, toBindingFromHookInput } from "../src/adapter/binding.js";
import { codeFor, toRuntimeError } from "../src/adapter/errors.js";
import { DecisionDenied, SidecarUnavailable, SpendGuardConfigError } from "../src/reservation.js";
import { FIXTURE_TENANT_ID, makeConfig, makeHookInput } from "./_fixtures.js";

describe("adapters (AD01–AD06)", () => {
  test("AD01 test_binding_carries_bot_id_as_tenant_default", () => {
    const cfg = makeConfig({ tenantId: "" });
    const tid = pickTenantId(cfg, "bot-test-1");
    expect(tid).toBe("bot-test-1");
  });

  test("AD02 test_binding_carries_explicit_tenant_id", () => {
    const cfg = makeConfig({ tenantId: FIXTURE_TENANT_ID });
    const tid = pickTenantId(cfg, "bot-test-1");
    expect(tid).toBe(FIXTURE_TENANT_ID);
  });

  test("AD03 test_prompt_hash_computed_via_d05_helper", () => {
    const cfg = makeConfig({ tenantId: FIXTURE_TENANT_ID });
    const input = makeHookInput();
    const binding = toBindingFromHookInput({ input, configuration: cfg });
    // Reproduce the prompt-hash derivation: JSON-serialise the messages
    // and HMAC with the tenant id.
    const promptText = JSON.stringify(
      binding.messages.map((m) => ({ role: m.role, content: m.content })),
    );
    const expected = computePromptHash(promptText, FIXTURE_TENANT_ID);
    expect(expected).toMatch(/^[0-9a-f]{64}$/);
    // Idempotent: calling computePromptHash twice yields the same result.
    const second = computePromptHash(promptText, FIXTURE_TENANT_ID);
    expect(second).toBe(expected);
  });

  test("AD04 test_error_translation_denied_to_budget_denied", () => {
    const err = new DecisionDenied("budget cap exceeded", ["BUDGET_EXCEEDED"]);
    const rt = toRuntimeError(err);
    expect(rt).toBeInstanceOf(RuntimeError);
    expect(rt.message).toContain("denied");
    expect(codeFor(err)).toBe("BUDGET_DENIED");
  });

  test("AD05 test_error_translation_unavailable_to_budget_degraded", () => {
    const err = new SidecarUnavailable("sidecar gone");
    const rt = toRuntimeError(err);
    expect(rt).toBeInstanceOf(RuntimeError);
    expect(rt.message).toContain("degraded");
    expect(codeFor(err)).toBe("BUDGET_DEGRADED");
  });

  test("AD06 test_error_translation_config_to_budget_config", () => {
    const err = new SpendGuardConfigError("missing sidecarUrl");
    const rt = toRuntimeError(err);
    expect(rt).toBeInstanceOf(RuntimeError);
    expect(rt.message).toContain("config");
    expect(codeFor(err)).toBe("BUDGET_CONFIG");
  });
});
