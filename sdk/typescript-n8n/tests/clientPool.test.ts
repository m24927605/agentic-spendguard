// D37 unit tests — clientPool singleton + FIFO eviction.
// Covers CP-01..CP-09 per tests.md §3.3.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  MAX_POOL_ENTRIES,
  _poolSizeForTests,
  _resetPoolForTests,
  _setClientFactoryForTests,
  acquireClient,
  key,
} from "../src/clientPool";
import { makeMockClient } from "./_support/mockSpendGuardClient";

function creds(over: Record<string, unknown> = {}) {
  return {
    tenantId: "00000000-0000-4000-8000-000000000001",
    socketPath: "/tmp/sg.sock",
    budgetId: "44444444-4444-4444-8444-444444444444",
    windowInstanceId: "55555555-5555-4555-8555-555555555555",
    runtimeKind: "n8n",
    ...over,
  };
}

describe("clientPool", () => {
  beforeEach(() => {
    _resetPoolForTests();
  });
  afterEach(() => {
    _setClientFactoryForTests(null);
    _resetPoolForTests();
  });

  it("CP-01 first acquireClient connects + handshakes", async () => {
    const built: ReturnType<typeof makeMockClient>[] = [];
    _setClientFactoryForTests((opts) => {
      const m = makeMockClient({
        tenantId: opts.tenantId ?? "",
        socketPath: opts.socketPath ?? "",
      });
      built.push(m);
      return m.client;
    });
    const c1 = await acquireClient(creds());
    expect(c1).toBeDefined();
    expect(built[0]?.state.connectCalls).toBe(1);
    expect(built[0]?.state.handshakeCalls).toBe(1);
  });

  it("CP-02 second acquireClient with same creds returns SAME instance", async () => {
    let count = 0;
    _setClientFactoryForTests((opts) => {
      count += 1;
      return makeMockClient({
        tenantId: opts.tenantId ?? "",
        socketPath: opts.socketPath ?? "",
      }).client;
    });
    const c1 = await acquireClient(creds());
    const c2 = await acquireClient(creds());
    expect(c1).toBe(c2);
    expect(count).toBe(1);
  });

  it("CP-03 different tenantId → different instance", async () => {
    _setClientFactoryForTests(
      (opts) =>
        makeMockClient({
          tenantId: opts.tenantId ?? "",
          socketPath: opts.socketPath ?? "",
        }).client,
    );
    const c1 = await acquireClient(creds({ tenantId: "tenant-A" }));
    const c2 = await acquireClient(creds({ tenantId: "tenant-B" }));
    expect(c1).not.toBe(c2);
  });

  it("CP-04 17th unique credential triggers FIFO eviction with close()", async () => {
    const closeSpies: Array<ReturnType<typeof vi.fn>> = [];
    _setClientFactoryForTests((opts) => {
      const close = vi.fn(async () => undefined);
      closeSpies.push(close);
      const m = makeMockClient({
        tenantId: opts.tenantId ?? "",
        socketPath: opts.socketPath ?? "",
      });
      // Replace close with the spy
      (m.client as unknown as { close: () => Promise<void> }).close = close;
      return m.client;
    });
    for (let i = 0; i < MAX_POOL_ENTRIES; i += 1) {
      // Force unique key via socketPath
      await acquireClient(creds({ socketPath: `/tmp/sock-${i}.sock` }));
    }
    expect(_poolSizeForTests()).toBe(MAX_POOL_ENTRIES);
    // 17th unique → evicts oldest (index 0)
    await acquireClient(creds({ socketPath: "/tmp/sock-NEW.sock" }));
    expect(_poolSizeForTests()).toBe(MAX_POOL_ENTRIES);
    // Wait microtask for evicted close to fire.
    await new Promise((resolve) => setImmediate(resolve));
    expect(closeSpies[0]).toHaveBeenCalledTimes(1);
  });

  it("CP-05 failed handshake → entry removed → retry from scratch succeeds", async () => {
    let calls = 0;
    _setClientFactoryForTests((opts) => {
      calls += 1;
      const reject = calls === 1 ? new Error("handshake failed") : undefined;
      const m = makeMockClient({
        tenantId: opts.tenantId ?? "",
        socketPath: opts.socketPath ?? "",
        ...(reject ? { rejectHandshake: reject } : {}),
      });
      return m.client;
    });
    await expect(acquireClient(creds())).rejects.toThrow(/handshake failed/);
    // Pool should have purged the entry, so the next call rebuilds.
    const c2 = await acquireClient(creds());
    expect(c2).toBeDefined();
    expect(calls).toBe(2);
  });

  it("CP-06 concurrent first-calls share a single in-flight promise", async () => {
    let calls = 0;
    _setClientFactoryForTests((opts) => {
      calls += 1;
      return makeMockClient({
        tenantId: opts.tenantId ?? "",
        socketPath: opts.socketPath ?? "",
      }).client;
    });
    const [c1, c2, c3] = await Promise.all([
      acquireClient(creds()),
      acquireClient(creds()),
      acquireClient(creds()),
    ]);
    expect(calls).toBe(1);
    expect(c1).toBe(c2);
    expect(c2).toBe(c3);
  });

  it("CP-07 beforeExit closes every cached client", async () => {
    const closes: Array<ReturnType<typeof vi.fn>> = [];
    _setClientFactoryForTests((opts) => {
      const close = vi.fn(async () => undefined);
      closes.push(close);
      const m = makeMockClient({
        tenantId: opts.tenantId ?? "",
        socketPath: opts.socketPath ?? "",
      });
      (m.client as unknown as { close: () => Promise<void> }).close = close;
      return m.client;
    });
    await acquireClient(creds({ socketPath: "/a.sock" }));
    await acquireClient(creds({ socketPath: "/b.sock" }));
    // Emit beforeExit to drive the registered handler.
    process.emit("beforeExit", 0);
    await new Promise((resolve) => setImmediate(resolve));
    expect(closes[0]).toHaveBeenCalled();
    expect(closes[1]).toHaveBeenCalled();
  });

  it("CP-08 key() deterministic across property iteration order", () => {
    const a = key({ tenantId: "X", socketPath: "/s", runtimeKind: "n8n" });
    const b = key({ runtimeKind: "n8n", socketPath: "/s", tenantId: "X" });
    expect(a).toBe(b);
  });

  it("CP-09 differing only in runtimeKind share a client", async () => {
    let calls = 0;
    _setClientFactoryForTests((opts) => {
      calls += 1;
      return makeMockClient({
        tenantId: opts.tenantId ?? "",
        socketPath: opts.socketPath ?? "",
      }).client;
    });
    await acquireClient(creds({ runtimeKind: "n8n" }));
    await acquireClient(creds({ runtimeKind: "n8n-cloud" }));
    expect(calls).toBe(1);
  });
});
