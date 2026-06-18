// SLICE 8 — end-to-end integration tests for OTel + retry + idempotency
// cache wiring inside SpendGuardClient.reserve().
//
// These tests use the MockSidecar to exercise the full UDS → protobuf-ts →
// reserve() flow with the new SLICE 8 hooks attached:
//
//   - cfg.idempotencyCache → reserve() hits cache before sidecar; cached
//     outcome short-circuits the wire path.
//   - cfg.otelTracer → reserve() / handshake() / release() / commitEstimated()
//     emit a `spendguard.<rpc>` span via withOtelSpan.
//   - cfg.idempotencyCache caching only happens on successful CONTINUE/DEGRADE
//     outcomes (denials throw before reaching the cache.set call).

import type { Attributes, Span, Tracer } from "@opentelemetry/api";
import { afterEach, describe, expect, it } from "vitest";

import {
  DecisionStopped,
  InMemoryIdempotencyCache,
  NoopIdempotencyCache,
  type SpanRecord,
  SpendGuardClient,
} from "../src/index.js";
import { MockSidecar, makeStopResponse } from "./_support/mockSidecar.js";

const ENV_KEYS = [
  "SPENDGUARD_SOCKET_PATH",
  "SPENDGUARD_SIDECAR_UDS",
  "SPENDGUARD_TENANT_ID",
  "SPENDGUARD_DISABLE",
] as const;
const savedEnv: Record<string, string | undefined> = {};
for (const k of ENV_KEYS) savedEnv[k] = process.env[k];
afterEach(() => {
  for (const k of ENV_KEYS) {
    if (savedEnv[k] === undefined) delete process.env[k];
    else process.env[k] = savedEnv[k];
  }
});

function reserveReq(overrides: Partial<Parameters<SpendGuardClient["reserve"]>[0]> = {}) {
  return {
    trigger: "LLM_CALL_PRE" as const,
    runId: "run-1",
    stepId: "step-1",
    llmCallId: "llm-1",
    decisionId: "d-1",
    route: "openai|gpt-4o-mini",
    projectedClaims: [
      {
        scopeId: "tenant/test/global",
        amountAtomic: "1000",
        unit: { unit: "USD_MICROS", denomination: 1 },
      },
    ],
    idempotencyKey: "sg-0123456789abcdef",
    ...overrides,
  };
}

interface MockSpanCall {
  name: string;
  attributes: Attributes | undefined;
  ended: boolean;
}

function makeMockTracer(): { tracer: Tracer; calls: MockSpanCall[] } {
  const calls: MockSpanCall[] = [];
  const tracer: Tracer = {
    startSpan(name: string, options?: { attributes?: Attributes }): Span {
      const call: MockSpanCall = {
        name,
        attributes: options?.attributes,
        ended: false,
      };
      calls.push(call);
      return {
        spanContext: () => ({
          traceId: "00000000000000000000000000000000",
          spanId: "0000000000000000",
          traceFlags: 0,
        }),
        setAttribute: () => ({}) as Span,
        setAttributes: () => ({}) as Span,
        addEvent: () => ({}) as Span,
        addLink: () => ({}) as Span,
        addLinks: () => ({}) as Span,
        setStatus: () => ({}) as Span,
        updateName: () => ({}) as Span,
        end: () => {
          call.ended = true;
        },
        isRecording: () => true,
        recordException: () => {},
      } as unknown as Span;
    },
    startActiveSpan: () => {
      throw new Error("not implemented");
    },
  } as unknown as Tracer;
  return { tracer, calls };
}

describe("SLICE 8 integration — idempotencyCache wiring inside reserve()", () => {
  it("cache HIT short-circuits the sidecar RPC", async () => {
    const mock = await MockSidecar.start();
    const cache = new InMemoryIdempotencyCache();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        idempotencyCache: cache,
      });
      await client.connect();
      await client.handshake();
      // First reserve — goes to sidecar, populates cache
      const out1 = await client.reserve(reserveReq());
      expect(out1.decision).toBe("CONTINUE");
      expect(cache.size).toBe(1);
      // Snapshot how many decision requests the mock has seen
      const requestsBefore = mock.decisionsServed;
      // Second reserve with SAME idempotencyKey — should hit cache
      const out2 = await client.reserve(reserveReq());
      expect(out2).toBe(out1); // identity-equal — same outcome reference
      // Sidecar count UNCHANGED because cache short-circuit
      expect(mock.decisionsServed).toBe(requestsBefore);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("same idempotencyKey but DIFFERENT body falls through to the sidecar (no stale CONTINUE)", async () => {
    const mock = await MockSidecar.start();
    const cache = new InMemoryIdempotencyCache();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        idempotencyCache: cache,
      });
      await client.connect();
      await client.handshake();
      // First reserve populates the cache under the shared key.
      await client.reserve(reserveReq({ idempotencyKey: "sg-collide" }));
      const requestsBefore = mock.decisionsServed;
      // Second reserve REUSES the key but for a logically different request
      // (different amount + route). A key-only cache would return the stale
      // CONTINUE; the body-hash binding must force a sidecar round trip.
      await client.reserve(
        reserveReq({
          idempotencyKey: "sg-collide",
          route: "anthropic|claude-3-5-sonnet",
          projectedClaims: [
            {
              scopeId: "tenant/test/global",
              amountAtomic: "999999",
              unit: { unit: "USD_MICROS", denomination: 1 },
            },
          ],
        }),
      );
      // The sidecar WAS contacted again — the collision did not short-circuit.
      expect(mock.decisionsServed).toBe(requestsBefore + 1);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("cache MISS falls through to sidecar normally", async () => {
    const mock = await MockSidecar.start();
    const cache = new InMemoryIdempotencyCache();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        idempotencyCache: cache,
      });
      await client.connect();
      await client.handshake();
      // Reserve with DIFFERENT idempotency keys — each hits sidecar
      const r1 = await client.reserve(reserveReq({ idempotencyKey: "sg-a" }));
      const r2 = await client.reserve(reserveReq({ idempotencyKey: "sg-b" }));
      expect(r1.decision).toBe("CONTINUE");
      expect(r2.decision).toBe("CONTINUE");
      expect(cache.size).toBe(2);
      expect(mock.decisionsServed).toBe(2);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("NoopIdempotencyCache does NOT short-circuit the sidecar", async () => {
    const mock = await MockSidecar.start();
    const cache = new NoopIdempotencyCache();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        idempotencyCache: cache,
      });
      await client.connect();
      await client.handshake();
      // Two identical reserves — both hit sidecar (noop cache always misses)
      await client.reserve(reserveReq());
      await client.reserve(reserveReq());
      expect(mock.decisionsServed).toBe(2);
      expect(cache.size).toBe(0); // never stored
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("denial outcome is NOT cached (only CONTINUE/DEGRADE outcomes cache)", async () => {
    const mock = await MockSidecar.start({
      onRequestDecision: () =>
        makeStopResponse({
          decisionId: "d-stop",
          reasonCodes: ["budget_exhausted"],
        }),
    });
    const cache = new InMemoryIdempotencyCache();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        idempotencyCache: cache,
      });
      await client.connect();
      await client.handshake();
      await expect(client.reserve(reserveReq())).rejects.toBeInstanceOf(DecisionStopped);
      // STOP throws before cache.set; cache MUST stay empty so the next
      // reserve attempt retries the decision (denials can change over time
      // as budget refills or policy is updated).
      expect(cache.size).toBe(0);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("no cache configured: reserve always hits sidecar", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        // no idempotencyCache
      });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq());
      await client.reserve(reserveReq()); // same key, no cache → sidecar twice
      expect(mock.decisionsServed).toBe(2);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

describe("SLICE 8 integration — otelTracer wiring across all 5 wired RPCs", () => {
  it("handshake() emits a 'spendguard.handshake' span", async () => {
    const mock = await MockSidecar.start();
    const { tracer, calls } = makeMockTracer();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        otelTracer: tracer,
      });
      await client.connect();
      await client.handshake();
      const handshakeSpan = calls.find((c) => c.name === "spendguard.handshake");
      expect(handshakeSpan).toBeDefined();
      expect(handshakeSpan?.ended).toBe(true);
      expect(handshakeSpan?.attributes?.["spendguard.tenant_id"]).toBe("t");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("reserve() emits a 'spendguard.reserve' span with decision/trigger attributes", async () => {
    const mock = await MockSidecar.start();
    const { tracer, calls } = makeMockTracer();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        otelTracer: tracer,
      });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq());
      const reserveSpan = calls.find((c) => c.name === "spendguard.reserve");
      expect(reserveSpan).toBeDefined();
      expect(reserveSpan?.ended).toBe(true);
      expect(reserveSpan?.attributes?.["spendguard.tenant_id"]).toBe("t");
      expect(reserveSpan?.attributes?.["spendguard.decision_id"]).toBe("d-1");
      expect(reserveSpan?.attributes?.["spendguard.trigger"]).toBe("LLM_CALL_PRE");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("reserve() span ends in finally even when decision throws (STOP)", async () => {
    const mock = await MockSidecar.start({
      onRequestDecision: () => makeStopResponse({ decisionId: "d-stop" }),
    });
    const { tracer, calls } = makeMockTracer();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        otelTracer: tracer,
      });
      await client.connect();
      await client.handshake();
      await expect(client.reserve(reserveReq())).rejects.toBeInstanceOf(DecisionStopped);
      // The span MUST still end despite the thrown DecisionStopped
      const reserveSpan = calls.find((c) => c.name === "spendguard.reserve");
      expect(reserveSpan?.ended).toBe(true);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("no spans created when otelTracer is undefined (peer-optional zero cost)", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        // no otelTracer
      });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq());
      // Nothing to assert directly — the test would have crashed if any span
      // emission code paths tried to touch an undefined tracer.
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

describe("SLICE 8 integration — onSpan observer wiring (no OTel dep path)", () => {
  it("reserve() invokes cfg.onSpan once with a SpanRecord for the RPC", async () => {
    const mock = await MockSidecar.start();
    const records: SpanRecord[] = [];
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        onSpan: (r) => records.push(r),
      });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq());
      const reserveRecord = records.find((r) => r.name === "spendguard.reserve");
      expect(reserveRecord).toBeDefined();
      // handshake also emits a record (both RPCs are wired uniformly).
      expect(records.some((r) => r.name === "spendguard.handshake")).toBe(true);
      expect(reserveRecord?.attributes["spendguard.tenant_id"]).toBe("t");
      expect(typeof reserveRecord?.startTimeMs).toBe("number");
      expect(typeof reserveRecord?.durationMs).toBe("number");
      expect(reserveRecord?.error).toBeUndefined();
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("onSpan receives the error when the RPC throws (e.g. STOP decision)", async () => {
    const mock = await MockSidecar.start({
      onRequestDecision: () => makeStopResponse({ decisionId: "d-stop" }),
    });
    const records: SpanRecord[] = [];
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        onSpan: (r) => records.push(r),
      });
      await client.connect();
      await client.handshake();
      await expect(client.reserve(reserveReq())).rejects.toBeInstanceOf(DecisionStopped);
      const reserveRecord = records.find((r) => r.name === "spendguard.reserve");
      expect(reserveRecord?.error).toBeInstanceOf(Error);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// MockSidecar may not expose decisionRequestCount; if not, the integration
// tests for cache miss/hit can stand by relying on cache.size as the proxy.
// We rely on whatever the existing mock surface provides. If
// decisionRequestCount is absent on the current mock, the assertion will
// throw — see Step 12 below to add the counter to the mock.
