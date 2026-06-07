// SpendGuardChatModelWrapper unit tests — covers acceptance.md A2.2
// (W-01..W-16). Drives the wrapper's `init()` with a mock handler
// factory and a mock chat model so the test suite is hermetic.

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { _resetCacheForTests, _setClientFactoryForTests } from "../src/clientCache.js";
import {
  _setHandlerFactoryForTests,
  SpendGuardChatModelWrapper,
} from "../src/nodes/SpendGuardChatModelWrapper.js";
import { createChatModel } from "./_support/mockChatModel.js";
import { createMockClientFactory } from "./_support/mockSidecar.js";

interface InvokedHandlerArgs {
  client: unknown;
  budgetId: string;
  windowInstanceId: string;
  unit: string;
  route: string;
  claimEstimator: () => Array<{ scopeId: string; amountAtomic: string; unit: string }>;
}

let invokedArgs: InvokedHandlerArgs[];
let factoryInvocations: number;
let prevSidecarEnv: string | undefined;

beforeEach(() => {
  invokedArgs = [];
  factoryInvocations = 0;
  prevSidecarEnv = process.env.SPENDGUARD_SIDECAR_UDS;
  delete process.env.SPENDGUARD_SIDECAR_UDS;

  _setClientFactoryForTests(createMockClientFactory());
  _setHandlerFactoryForTests(async (deps) => {
    factoryInvocations += 1;
    invokedArgs.push({
      client: deps.client.raw,
      budgetId: deps.budgetId,
      windowInstanceId: deps.windowInstanceId,
      unit: deps.unit,
      route: deps.route,
      claimEstimator: deps.claimEstimator,
    });
    // Return a sentinel handler the test can identify by reference.
    return { _handlerSentinel: deps.budgetId };
  });
});

afterEach(() => {
  _resetCacheForTests();
  _setHandlerFactoryForTests(undefined);
  if (prevSidecarEnv === undefined) {
    delete process.env.SPENDGUARD_SIDECAR_UDS;
  } else {
    process.env.SPENDGUARD_SIDECAR_UDS = prevSidecarEnv;
  }
});

function inputs(overrides?: Record<string, unknown>): Record<string, unknown> {
  return {
    chatModel: createChatModel(),
    tenantId: "00000000-0000-4000-8000-000000000001",
    budgetId: "44444444-4444-4444-8444-444444444444",
    windowInstanceId: "55555555-5555-4555-8555-555555555555",
    unit: "usd_micros",
    sidecarUds: "/run/spendguard/sg.sock",
    route: "llm.call",
    claimEstimatorJson: "",
    ...(overrides ?? {}),
  };
}

describe("SpendGuardChatModelWrapper", () => {
  it("W-01 happy path — init() returns the SAME chatModel reference", async () => {
    const w = new SpendGuardChatModelWrapper();
    const chatModel = createChatModel("ref-eq-test");
    const result = await w.init({ inputs: inputs({ chatModel }) }, "", {});
    expect(result).toBe(chatModel);
  });

  it("W-02 happy path — init() appends ONE handler to chatModel.callbacks", async () => {
    const w = new SpendGuardChatModelWrapper();
    const chatModel = createChatModel();
    await w.init({ inputs: inputs({ chatModel }) }, "", {});
    expect(chatModel.callbacks).toHaveLength(1);
    expect((chatModel.callbacks?.[0] as { _handlerSentinel?: string })._handlerSentinel).toBe(
      "44444444-4444-4444-8444-444444444444",
    );
  });

  it("W-03 idempotency — pre-existing callbacks are preserved", async () => {
    const w = new SpendGuardChatModelWrapper();
    const chatModel = createChatModel();
    const preexisting = { _name: "langsmith" };
    chatModel.callbacks = [preexisting];
    await w.init({ inputs: inputs({ chatModel }) }, "", {});
    expect(chatModel.callbacks).toHaveLength(2);
    expect(chatModel.callbacks?.[0]).toBe(preexisting);
  });

  it("W-04 happy path — handler factory receives the budget metadata as-typed", async () => {
    const w = new SpendGuardChatModelWrapper();
    await w.init({ inputs: inputs() }, "", {});
    expect(invokedArgs).toHaveLength(1);
    expect(invokedArgs[0]?.budgetId).toBe("44444444-4444-4444-8444-444444444444");
    expect(invokedArgs[0]?.windowInstanceId).toBe("55555555-5555-4555-8555-555555555555");
    expect(invokedArgs[0]?.unit).toBe("usd_micros");
    expect(invokedArgs[0]?.route).toBe("llm.call");
  });

  it("W-05 missing chatModel — throws explicit error", async () => {
    const w = new SpendGuardChatModelWrapper();
    await expect(
      w.init({ inputs: inputs({ chatModel: undefined }) }, "", {}),
    ).rejects.toThrow(/chatModel input is required/);
  });

  it("W-06 missing tenantId — throws aggregate error including 'tenantId'", async () => {
    const w = new SpendGuardChatModelWrapper();
    await expect(
      w.init({ inputs: inputs({ tenantId: "" }) }, "", {}),
    ).rejects.toThrow(/tenantId, budgetId, windowInstanceId/);
  });

  it("W-07 missing budgetId — throws aggregate error", async () => {
    const w = new SpendGuardChatModelWrapper();
    await expect(
      w.init({ inputs: inputs({ budgetId: "" }) }, "", {}),
    ).rejects.toThrow(/tenantId, budgetId, windowInstanceId/);
  });

  it("W-08 missing windowInstanceId — throws aggregate error", async () => {
    const w = new SpendGuardChatModelWrapper();
    await expect(
      w.init({ inputs: inputs({ windowInstanceId: "" }) }, "", {}),
    ).rejects.toThrow(/windowInstanceId/);
  });

  it("W-09 missing sidecarUds and no env — throws aggregate error", async () => {
    const w = new SpendGuardChatModelWrapper();
    await expect(
      w.init({ inputs: inputs({ sidecarUds: "" }) }, "", {}),
    ).rejects.toThrow(/sidecarUds \(or env SPENDGUARD_SIDECAR_UDS\)/);
  });

  it("W-10 env fallback — SPENDGUARD_SIDECAR_UDS picked up when sidecarUds blank", async () => {
    process.env.SPENDGUARD_SIDECAR_UDS = "/run/from-env/sg.sock";
    const w = new SpendGuardChatModelWrapper();
    const chatModel = createChatModel();
    await w.init({ inputs: inputs({ chatModel, sidecarUds: "" }) }, "", {});
    expect(chatModel.callbacks).toHaveLength(1);
  });

  it("W-11 default unit — empty unit input falls back to 'usd_micros'", async () => {
    const w = new SpendGuardChatModelWrapper();
    await w.init({ inputs: inputs({ unit: "" }) }, "", {});
    expect(invokedArgs[0]?.unit).toBe("usd_micros");
  });

  it("W-12 default route — empty route input falls back to 'llm.call'", async () => {
    const w = new SpendGuardChatModelWrapper();
    await w.init({ inputs: inputs({ route: "" }) }, "", {});
    expect(invokedArgs[0]?.route).toBe("llm.call");
  });

  it("W-13 claimEstimator JSON override flows into the handler factory", async () => {
    const w = new SpendGuardChatModelWrapper();
    await w.init(
      {
        inputs: inputs({
          claimEstimatorJson: JSON.stringify({
            amountAtomic: "250000",
            scopeId: "tier-premium",
          }),
        }),
      },
      "",
      {},
    );
    const claims = invokedArgs[0]?.claimEstimator();
    expect(claims).toEqual([
      { scopeId: "tier-premium", amountAtomic: "250000", unit: "usd_micros" },
    ]);
  });

  it("W-14 claimEstimator default — no JSON yields the conservative $1 default", async () => {
    const w = new SpendGuardChatModelWrapper();
    await w.init({ inputs: inputs() }, "", {});
    const claims = invokedArgs[0]?.claimEstimator();
    expect(claims).toEqual([
      { scopeId: "default", amountAtomic: "1000000", unit: "usd_micros" },
    ]);
  });

  it("W-15 client cache reuse — two init() calls share the same SpendGuardClient", async () => {
    const w = new SpendGuardChatModelWrapper();
    const chatModelA = createChatModel("A");
    const chatModelB = createChatModel("B");
    await w.init({ inputs: inputs({ chatModel: chatModelA }) }, "", {});
    await w.init({ inputs: inputs({ chatModel: chatModelB }) }, "", {});
    expect(factoryInvocations).toBe(2);
    expect(invokedArgs[0]?.client).toBe(invokedArgs[1]?.client);
  });

  it("W-16 trimming — leading / trailing whitespace on string inputs is normalised", async () => {
    const w = new SpendGuardChatModelWrapper();
    await w.init(
      {
        inputs: inputs({
          tenantId: "  00000000-0000-4000-8000-000000000001  ",
          budgetId: "  44444444-4444-4444-8444-444444444444 ",
          windowInstanceId: " 55555555-5555-4555-8555-555555555555 ",
          sidecarUds: " /run/spendguard/sg.sock ",
          route: "  llm.call  ",
        }),
      },
      "",
      {},
    );
    expect(invokedArgs[0]?.budgetId).toBe("44444444-4444-4444-8444-444444444444");
    expect(invokedArgs[0]?.route).toBe("llm.call");
  });
});
