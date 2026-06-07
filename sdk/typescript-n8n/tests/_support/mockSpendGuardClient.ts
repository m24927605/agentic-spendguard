// Fake `SpendGuardClient` for the D37 unit tests.
//
// Records connect / handshake / reserve / commit invocations so the tests
// can assert on call counts and arguments. The shape mirrors
// `@spendguard/sdk`'s `SpendGuardClient` just enough for the adapter +
// `clientPool` to typecheck under structural typing.

import type { SpendGuardClient } from "@spendguard/sdk";
import { vi } from "vitest";

export interface MockClientState {
  connectCalls: number;
  handshakeCalls: number;
  closeCalls: number;
  reserveCalls: unknown[];
  commitCalls: unknown[];
  tenantId: string;
  socketPath: string;
  runtimeKind: string;
}

export function makeMockClient(options: {
  tenantId: string;
  socketPath: string;
  runtimeKind?: string;
  rejectHandshake?: Error;
}): { client: SpendGuardClient; state: MockClientState } {
  const state: MockClientState = {
    connectCalls: 0,
    handshakeCalls: 0,
    closeCalls: 0,
    reserveCalls: [],
    commitCalls: [],
    tenantId: options.tenantId,
    socketPath: options.socketPath,
    runtimeKind: options.runtimeKind ?? "n8n",
  };

  const client = {
    tenantId: options.tenantId,
    connect: vi.fn(async () => {
      state.connectCalls += 1;
    }),
    handshake: vi.fn(async () => {
      state.handshakeCalls += 1;
      if (options.rejectHandshake) {
        throw options.rejectHandshake;
      }
    }),
    close: vi.fn(async () => {
      state.closeCalls += 1;
    }),
    reserve: vi.fn(async (req: unknown) => {
      state.reserveCalls.push(req);
      return {
        decisionId: "00000000-0000-4000-8000-000000000aaa",
        reservationIds: ["00000000-0000-4000-8000-000000000bbb"],
        verdict: "ALLOW",
      };
    }),
    commitEstimated: vi.fn(async (req: unknown) => {
      state.commitCalls.push(req);
    }),
  } as unknown as SpendGuardClient;

  return { client, state };
}
