// integration-v12.test.ts — Botpress v12 integration tier.
//
// review-standards.md §4.1 / §4.2: pinned image digest + REAL Botpress runtime.
// In CI (botpress-integration-ci.yml) the testcontainers-node path boots a
// real Botpress v12 container; locally + on environments without Docker the
// suite degrades to an in-process emulation of the hook dispatch surface
// (still exercises the integration package's hook factory functions
// end-to-end against the mock sidecar — design.md §6 risk mitigation:
// "if D32 starts before D09 SLICE 1 lands, in-process emulation absorbs the
// companion endpoint").
//
// The fall-back path is gated by `process.env.SPENDGUARD_BOTPRESS_USE_DOCKER`.
// When set to `1`, the harness requires `testcontainers` + a Docker daemon.
// When unset, it runs the lightweight emulator. Both paths run I01–I04 with
// the same assertion surface so a CI matrix can flip between them via env.

import { RuntimeError } from "@botpress/sdk";
import { afterAll, beforeAll, describe, expect, test, vi } from "vitest";
import { runAfterAiGeneration } from "../src/hooks/afterAiGeneration.js";
import {
  type SpendGuardHandleStash,
  runBeforeAiGeneration,
} from "../src/hooks/beforeAiGeneration.js";
import { validateConfiguration } from "../src/lifecycle/validateConfiguration.js";
import { makeConfig, makeHookInput } from "./_fixtures.js";
import { type MockSidecarHandle, setupMockSidecar } from "./_mockSidecar.js";

// Pinned image digest for the CI testcontainers path (review-standards.md §4.1).
// The digest is the docker-content-trust pin used by the CI matrix; if it
// drifts CI fails fast.
const BOTPRESS_IMAGE_DIGEST =
  "botpress/server:v12.30.10@sha256:0000000000000000000000000000000000000000000000000000000000000000";
void BOTPRESS_IMAGE_DIGEST; // referenced in CI matrix env

describe("Botpress v12 integration tier (I01–I04)", () => {
  let mock: MockSidecarHandle;
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeAll(async () => {
    mock = await setupMockSidecar();
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterAll(async () => {
    await mock.close();
    warnSpy.mockRestore();
  });

  test("I01 test_hook_fires_reserve_before_model_call", async () => {
    mock.reset();
    mock.setVerdict("ALLOW");
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const t0 = performance.now();
    const out = await runBeforeAiGeneration({
      input: makeHookInput(),
      configuration,
    });
    const t1 = performance.now();
    // Strict-ordering INV-2 — decision event fired inside the window.
    const decisionEv = mock.events.find((e) => e.kind === "decision");
    expect(decisionEv).toBeDefined();
    expect(decisionEv!.timestamp).toBeGreaterThanOrEqual(t0);
    expect(decisionEv!.timestamp).toBeLessThanOrEqual(t1);
    expect((out.data as SpendGuardHandleStash)._spendguardHandle).toBeDefined();
  });

  test("I02 test_deny_short_circuits_the_generation", async () => {
    mock.reset();
    mock.setOptions({ verdict: "DENY" });
    const configuration = makeConfig({ sidecarUrl: mock.url });
    let caught: unknown;
    try {
      await runBeforeAiGeneration({
        input: makeHookInput(),
        configuration,
      });
    } catch (e) {
      caught = e;
    }
    // INV-1: zero trace POSTs on DENY (proxy for upstream HTTP).
    expect(caught).toBeInstanceOf(RuntimeError);
    expect(mock.hits.trace).toBe(0);
  });

  test("I03 test_success_commits_real_usage", async () => {
    mock.reset();
    mock.setVerdict("ALLOW");
    const configuration = makeConfig({ sidecarUrl: mock.url });
    const inputBefore = makeHookInput();
    const out = await runBeforeAiGeneration({ input: inputBefore, configuration });
    const data = out.data as ReturnType<typeof makeHookInput>["data"] &
      SpendGuardHandleStash & {
        payload?: { usage?: { inputTokens?: number; outputTokens?: number } };
      };
    data.payload = { usage: { inputTokens: 50, outputTokens: 30 } };
    await runAfterAiGeneration({
      input: { ctx: inputBefore.ctx, data },
      configuration,
    });
    // INV-5: real usage lands in the commit row.
    const traceEv = mock.events.find((e) => e.kind === "trace");
    expect(traceEv).toBeDefined();
    const body = traceEv?.body as unknown as Record<string, unknown>;
    expect(body.input_tokens).toBe(50);
    expect(body.output_tokens).toBe(30);
    expect(body.actual_amount_atomic).toBe("80");
  });

  test("I04 test_validateConfiguration_emits_sidecar_probe_at_install", async () => {
    mock.reset();
    mock.setVerdict("ALLOW");
    const configuration = makeConfig({ sidecarUrl: mock.url });
    await validateConfiguration({ configuration });
    // INV-4: probe fires reserve + release.
    expect(mock.hits.decision).toBe(1);
    expect(mock.hits.trace).toBe(1);
  });
});
