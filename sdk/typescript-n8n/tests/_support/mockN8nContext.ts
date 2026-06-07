// Fake `ISupplyDataFunctions` for D37 unit tests.
//
// n8n's loader resolves `getCredentials` / `getNodeParameter` /
// `getInputConnectionData` / `getExecutionId` / `getNode` against the
// runtime workflow context. The unit tests do NOT spin up n8n — they
// build a minimal fake context that satisfies the same interface, so the
// SpendGuardChatModel.node's `supplyData` runs unchanged.

import type { ISupplyDataFunctions } from "n8n-workflow";
import { vi } from "vitest";

export interface MockContextOverrides {
  credentials?: Record<string, unknown>;
  parameters?: Record<string, unknown>;
  upstreamModel?: unknown;
  executionId?: string;
  nodeName?: string;
}

export function makeMockContext(overrides: MockContextOverrides = {}): ISupplyDataFunctions {
  const credentials: Record<string, unknown> = {
    tenantId: "00000000-0000-4000-8000-000000000001",
    socketPath: "/tmp/spendguard-mock.sock",
    budgetId: "44444444-4444-4444-8444-444444444444",
    windowInstanceId: "55555555-5555-4555-8555-555555555555",
    runtimeKind: "n8n",
    ...(overrides.credentials ?? {}),
  };
  const parameters: Record<string, unknown> = {
    budgetIdOverride: "",
    route: "llm.call",
    runIdSource: "executionId",
    customRunId: "",
    claimAmountAtomic: "1000000",
    unit: "usd_micros",
    ...(overrides.parameters ?? {}),
  };
  const executionId = overrides.executionId ?? "exec-12345";
  const nodeName = overrides.nodeName ?? "AI Agent";

  const ctx = {
    getCredentials: vi.fn().mockResolvedValue(credentials),
    getNodeParameter: vi.fn((name: string, _itemIndex: number, fallback?: unknown) => {
      return parameters[name] !== undefined ? parameters[name] : fallback;
    }),
    getInputConnectionData: vi.fn().mockResolvedValue(overrides.upstreamModel),
    getExecutionId: vi.fn(() => executionId),
    getNode: vi.fn(() => ({
      id: "n8n-node-1",
      name: nodeName,
      typeVersion: 1,
      type: "n8n-nodes-base.spendGuardChatModel",
      position: [0, 0],
      parameters: {},
    })),
  } as unknown as ISupplyDataFunctions;

  return ctx;
}
