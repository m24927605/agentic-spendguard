// Test-only mock SpendGuard sidecar.
//
// Mirrors D32 Botpress's tests/_mockSidecar.ts in spirit but trimmed to
// the surface the Flowise unit tests need: a minimal `CachedClient`-
// shaped stub plus a per-test counter so the wrapper tests can assert
// that `getOrCreateClient` cache reuse works.

import type { CachedClient } from "../../src/clientCache.js";

export interface MockClientCounters {
  connectCalls: number;
  handshakeCalls: number;
  factoryInvocations: number;
}

export interface MockClient extends CachedClient {
  readonly counters: MockClientCounters;
}

export function createMockClientFactory(counters?: MockClientCounters) {
  const shared = counters ?? { connectCalls: 0, handshakeCalls: 0, factoryInvocations: 0 };
  return async (k: { sidecarUds: string; tenantId: string }): Promise<MockClient> => {
    shared.factoryInvocations += 1;
    const stub: MockClient = {
      raw: {
        sidecarUds: k.sidecarUds,
        tenantId: k.tenantId,
        connect: async () => {
          shared.connectCalls += 1;
        },
        handshake: async () => {
          shared.handshakeCalls += 1;
        },
      },
      counters: shared,
    };
    // Simulate connect+handshake as if the real factory ran them.
    await stub.raw.connect();
    await stub.raw.handshake();
    return stub;
  };
}
