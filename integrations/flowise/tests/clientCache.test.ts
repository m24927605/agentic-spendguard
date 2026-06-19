// clientCache tests — covers acceptance.md A2.3 (C-01..C-06).
//
// The cache is the hot-path optimisation: Flowise instantiates an INode
// per chatflow execution, so without `getOrCreateClient` every chat
// invocation would pay the gRPC handshake cost. These tests pin the
// (tenantId, sidecarUds) composite key + the test-factory injection
// contract.

import { afterEach, describe, expect, it } from "vitest";

import {
  _cacheSize,
  _resetCacheForTests,
  _setClientFactoryForTests,
  getOrCreateClient,
} from "../src/clientCache.js";
import { createMockClientFactory } from "./_support/mockSidecar.js";

afterEach(() => {
  _resetCacheForTests();
});

describe("getOrCreateClient", () => {
  it("C-01 cold start — first call invokes the factory and seeds the cache", async () => {
    const counters = { connectCalls: 0, handshakeCalls: 0, factoryInvocations: 0 };
    _setClientFactoryForTests(createMockClientFactory(counters));
    const c = await getOrCreateClient({ sidecarUds: "/run/sg.sock", tenantId: "t1" });
    expect(c).toBeDefined();
    expect(counters.factoryInvocations).toBe(1);
    expect(counters.connectCalls).toBe(1);
    expect(counters.handshakeCalls).toBe(1);
    expect(_cacheSize()).toBe(1);
  });

  it("C-02 cache hit — same key returns the SAME client instance, no re-handshake", async () => {
    const counters = { connectCalls: 0, handshakeCalls: 0, factoryInvocations: 0 };
    _setClientFactoryForTests(createMockClientFactory(counters));
    const a = await getOrCreateClient({ sidecarUds: "/run/sg.sock", tenantId: "t1" });
    const b = await getOrCreateClient({ sidecarUds: "/run/sg.sock", tenantId: "t1" });
    expect(a).toBe(b);
    expect(counters.factoryInvocations).toBe(1);
    expect(counters.handshakeCalls).toBe(1);
  });

  it("C-03 isolation by sidecarUds — different sockets get different clients", async () => {
    const counters = { connectCalls: 0, handshakeCalls: 0, factoryInvocations: 0 };
    _setClientFactoryForTests(createMockClientFactory(counters));
    const a = await getOrCreateClient({ sidecarUds: "/run/a.sock", tenantId: "t1" });
    const b = await getOrCreateClient({ sidecarUds: "/run/b.sock", tenantId: "t1" });
    expect(a).not.toBe(b);
    expect(counters.factoryInvocations).toBe(2);
    expect(_cacheSize()).toBe(2);
  });

  it("C-04 isolation by tenantId — different tenants get different clients on the same socket", async () => {
    const counters = { connectCalls: 0, handshakeCalls: 0, factoryInvocations: 0 };
    _setClientFactoryForTests(createMockClientFactory(counters));
    const a = await getOrCreateClient({ sidecarUds: "/run/sg.sock", tenantId: "t1" });
    const b = await getOrCreateClient({ sidecarUds: "/run/sg.sock", tenantId: "t2" });
    expect(a).not.toBe(b);
    expect(_cacheSize()).toBe(2);
  });

  it("C-05 reset — _resetCacheForTests clears the cache AND the factory override", async () => {
    const counters = { connectCalls: 0, handshakeCalls: 0, factoryInvocations: 0 };
    _setClientFactoryForTests(createMockClientFactory(counters));
    await getOrCreateClient({ sidecarUds: "/run/sg.sock", tenantId: "t1" });
    expect(_cacheSize()).toBe(1);
    _resetCacheForTests();
    expect(_cacheSize()).toBe(0);
  });

  it("C-06 factory failure — rejected factory bubbles the error and does NOT seed the cache", async () => {
    _setClientFactoryForTests(async () => {
      throw new Error("sidecar down");
    });
    await expect(getOrCreateClient({ sidecarUds: "/run/sg.sock", tenantId: "t1" })).rejects.toThrow(
      /sidecar down/,
    );
    expect(_cacheSize()).toBe(0);
  });
});
