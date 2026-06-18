// SLICE 8 — OTel hook tests (design.md §6.4).
//
// The hook wraps every RPC in a `spendguard.<rpcName>` span. The tracer is
// optional; when undefined, the RPC runs unwrapped with zero overhead. When
// defined, the span carries the design.md §6.4 attribute table.
//
// We use a hand-rolled mock tracer (the OTel test SDK adds 200 KiB of
// transitive deps we don't want in the unit-test surface). The mock records
// `startSpan` calls + attribute / status / exception interactions so each
// test can assert the shape independently.

import type { Attributes, Span, Tracer } from "@opentelemetry/api";
import { describe, expect, it, vi } from "vitest";

import type { SpanRecord } from "../src/config.js";
import { SPENDGUARD_OTEL_ATTR, setOtelSpanAttributes, withOtelSpan } from "../src/otel.js";

// ── Mock tracer ────────────────────────────────────────────────────────────

interface MockSpanCall {
  name: string;
  attributes: Attributes | undefined;
  exception?: unknown;
  status?: { code: number; message?: string };
  ended: boolean;
  setAttributeCalls: Array<{ key: string; value: unknown }>;
}

function makeMockTracer(): { tracer: Tracer; calls: MockSpanCall[] } {
  const calls: MockSpanCall[] = [];
  const tracer: Tracer = {
    startSpan(name: string, options?: { attributes?: Attributes }): Span {
      const call: MockSpanCall = {
        name,
        attributes: options?.attributes,
        ended: false,
        setAttributeCalls: [],
      };
      calls.push(call);
      const span: Span = {
        spanContext() {
          return {
            traceId: "00000000000000000000000000000000",
            spanId: "0000000000000000",
            traceFlags: 0,
          };
        },
        setAttribute(key: string, value: unknown) {
          call.setAttributeCalls.push({ key, value });
          return span;
        },
        setAttributes(_attrs: Attributes) {
          return span;
        },
        addEvent() {
          return span;
        },
        addLink() {
          return span;
        },
        addLinks() {
          return span;
        },
        setStatus(status: { code: number; message?: string }) {
          call.status = status;
          return span;
        },
        updateName(newName: string) {
          call.name = newName;
          return span;
        },
        end() {
          call.ended = true;
        },
        isRecording() {
          return true;
        },
        recordException(exception: unknown) {
          call.exception = exception;
        },
      } as unknown as Span;
      return span;
    },
    startActiveSpan() {
      throw new Error("not implemented in mock");
    },
  } as unknown as Tracer;
  return { tracer, calls };
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe("withOtelSpan — tracer-present path (design §6.4)", () => {
  it("creates a span named 'spendguard.<rpcName>' when tracer present", async () => {
    const { tracer, calls } = makeMockTracer();
    await withOtelSpan(tracer, "reserve", {}, async () => {
      return "result";
    });
    expect(calls).toHaveLength(1);
    expect(calls[0]?.name).toBe("spendguard.reserve");
  });

  it("uses the design §6.4 attribute keys verbatim", async () => {
    const { tracer, calls } = makeMockTracer();
    await withOtelSpan(
      tracer,
      "reserve",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: "tenant-uuid",
        [SPENDGUARD_OTEL_ATTR.DECISION_ID]: "dec-1",
        [SPENDGUARD_OTEL_ATTR.TRIGGER]: "LLM_CALL_PRE",
        [SPENDGUARD_OTEL_ATTR.SDK_VERSION]: "0.1.0",
      },
      async () => "ok",
    );
    expect(calls).toHaveLength(1);
    const attrs = calls[0]?.attributes ?? {};
    // Verbatim key names per design §6.4 attribute table
    expect(attrs["spendguard.tenant_id"]).toBe("tenant-uuid");
    expect(attrs["spendguard.decision_id"]).toBe("dec-1");
    expect(attrs["spendguard.trigger"]).toBe("LLM_CALL_PRE");
    expect(attrs["spendguard.sdk.version"]).toBe("0.1.0");
  });

  it("returns the result of fn() verbatim on success", async () => {
    const { tracer } = makeMockTracer();
    const out = await withOtelSpan(tracer, "reserve", {}, async () => ({
      decisionId: "d1",
      decision: "CONTINUE" as const,
    }));
    expect(out).toEqual({ decisionId: "d1", decision: "CONTINUE" });
  });

  it("ends the span in finally — even on success", async () => {
    const { tracer, calls } = makeMockTracer();
    await withOtelSpan(tracer, "reserve", {}, async () => "ok");
    expect(calls[0]?.ended).toBe(true);
  });

  it("filters undefined attribute values from the span (avoid 'undefined' literal on wire)", async () => {
    const { tracer, calls } = makeMockTracer();
    await withOtelSpan(
      tracer,
      "reserve",
      {
        [SPENDGUARD_OTEL_ATTR.TENANT_ID]: "t",
        [SPENDGUARD_OTEL_ATTR.DECISION_ID]: undefined, // should be dropped
      },
      async () => "ok",
    );
    const attrs = calls[0]?.attributes ?? {};
    expect(attrs["spendguard.tenant_id"]).toBe("t");
    expect("spendguard.decision_id" in attrs).toBe(false);
  });
});

describe("withOtelSpan — onSpan observer path (no OTel dep)", () => {
  it("invokes onSpan once with a SpanRecord on success", async () => {
    const records: SpanRecord[] = [];
    const out = await withOtelSpan(
      undefined,
      "reserve",
      { [SPENDGUARD_OTEL_ATTR.TENANT_ID]: "t", [SPENDGUARD_OTEL_ATTR.DECISION_ID]: undefined },
      async () => "ok",
      (r) => records.push(r),
    );
    expect(out).toBe("ok");
    expect(records).toHaveLength(1);
    expect(records[0]?.name).toBe("spendguard.reserve");
    expect(records[0]?.attributes["spendguard.tenant_id"]).toBe("t");
    // undefined attribute dropped
    expect("spendguard.decision_id" in (records[0]?.attributes ?? {})).toBe(false);
    expect(typeof records[0]?.startTimeMs).toBe("number");
    expect(typeof records[0]?.durationMs).toBe("number");
    expect(records[0]?.error).toBeUndefined();
  });

  it("invokes onSpan with the error set when fn() throws, and rethrows", async () => {
    const records: SpanRecord[] = [];
    const err = new Error("boom");
    await expect(
      withOtelSpan(
        undefined,
        "reserve",
        {},
        async () => {
          throw err;
        },
        (r) => records.push(r),
      ),
    ).rejects.toBe(err);
    expect(records).toHaveLength(1);
    expect(records[0]?.error).toBe(err);
  });

  it("an onSpan callback that throws does not mask the RPC result", async () => {
    const out = await withOtelSpan(
      undefined,
      "reserve",
      {},
      async () => "ok",
      () => {
        throw new Error("observer blew up");
      },
    );
    expect(out).toBe("ok");
  });
});

describe("withOtelSpan — tracer-absent path (peer-optional dep)", () => {
  it("runs fn() without creating any span when tracer is undefined", async () => {
    const fn = vi.fn(async () => "ok");
    const result = await withOtelSpan(undefined, "reserve", {}, fn);
    expect(result).toBe("ok");
    expect(fn).toHaveBeenCalledOnce();
  });

  it("passes through return value when tracer is undefined", async () => {
    const out = await withOtelSpan(undefined, "anything", {}, async () => 42);
    expect(out).toBe(42);
  });

  it("rethrows fn() error when tracer is undefined (no recording side effect)", async () => {
    await expect(
      withOtelSpan(undefined, "reserve", {}, async () => {
        throw new Error("rpc failed");
      }),
    ).rejects.toThrow("rpc failed");
  });
});

describe("withOtelSpan — error recording path", () => {
  it("records exception + sets ERROR status when fn() throws an Error", async () => {
    const { tracer, calls } = makeMockTracer();
    const err = new Error("UNAVAILABLE");
    await expect(
      withOtelSpan(tracer, "reserve", {}, async () => {
        throw err;
      }),
    ).rejects.toBe(err);
    expect(calls[0]?.exception).toBe(err);
    // SpanStatusCode.ERROR === 2 (OTel API contract)
    expect(calls[0]?.status?.code).toBe(2);
    expect(calls[0]?.status?.message).toBe("UNAVAILABLE");
  });

  it("ends the span in finally — even when fn() throws", async () => {
    const { tracer, calls } = makeMockTracer();
    await expect(
      withOtelSpan(tracer, "reserve", {}, async () => {
        throw new Error("boom");
      }),
    ).rejects.toThrow("boom");
    expect(calls[0]?.ended).toBe(true);
  });

  it("rethrows the original error unchanged (no wrap)", async () => {
    const { tracer } = makeMockTracer();
    class CustomError extends Error {
      constructor(public readonly code: string) {
        super(code);
      }
    }
    const custom = new CustomError("DENIED");
    let caught: unknown;
    try {
      await withOtelSpan(tracer, "reserve", {}, async () => {
        throw custom;
      });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBe(custom); // identity, not wrapped
    expect((caught as CustomError).code).toBe("DENIED");
  });

  it("records non-Error throws via a synthesised Exception shape", async () => {
    const { tracer, calls } = makeMockTracer();
    await expect(
      withOtelSpan(tracer, "reserve", {}, async () => {
        throw "string-throw"; // intentional non-Error
      }),
    ).rejects.toBe("string-throw");
    expect(calls[0]?.exception).toEqual({
      name: "SpendGuardError",
      message: "string-throw",
    });
    expect(calls[0]?.status?.code).toBe(2);
  });
});

describe("withOtelSpan — span naming convention", () => {
  it("prepends 'spendguard.' to the rpcName (caller passes bare name)", async () => {
    const { tracer, calls } = makeMockTracer();
    await withOtelSpan(tracer, "handshake", {}, async () => "ok");
    await withOtelSpan(tracer, "release", {}, async () => "ok");
    await withOtelSpan(tracer, "commitEstimated", {}, async () => "ok");
    await withOtelSpan(tracer, "queryBudget", {}, async () => "ok");
    expect(calls.map((c) => c.name)).toEqual([
      "spendguard.handshake",
      "spendguard.release",
      "spendguard.commitEstimated",
      "spendguard.queryBudget",
    ]);
  });
});

describe("setOtelSpanAttributes — outcome-attribute helper", () => {
  it("sets attributes on the active span when provided", async () => {
    const { tracer, calls } = makeMockTracer();
    await withOtelSpan(tracer, "reserve", {}, async () => {
      // Look up the span via the recorded mock (would normally come from
      // closure — this exercises the helper's contract).
      const span = tracer.startSpan("inner");
      setOtelSpanAttributes(span, {
        [SPENDGUARD_OTEL_ATTR.OUTCOME_DECISION]: "CONTINUE",
        [SPENDGUARD_OTEL_ATTR.OUTCOME_REASON_CODES]: ["budget_ok", "policy_ok"],
      });
      span.end();
    });
    const innerCall = calls.find((c) => c.name === "inner");
    expect(innerCall).toBeDefined();
    const setCalls = innerCall?.setAttributeCalls ?? [];
    expect(setCalls).toContainEqual({
      key: "spendguard.outcome.decision",
      value: "CONTINUE",
    });
    expect(setCalls).toContainEqual({
      key: "spendguard.outcome.reason_codes",
      value: ["budget_ok", "policy_ok"],
    });
  });

  it("is a no-op when span is undefined", () => {
    // Should not throw; should silently no-op
    expect(() => setOtelSpanAttributes(undefined, { foo: "bar" })).not.toThrow();
  });

  it("skips undefined values rather than setting them", async () => {
    const { tracer, calls } = makeMockTracer();
    const span = tracer.startSpan("test");
    setOtelSpanAttributes(span, {
      [SPENDGUARD_OTEL_ATTR.TENANT_ID]: "t",
      [SPENDGUARD_OTEL_ATTR.DECISION_ID]: undefined,
    });
    span.end();
    const c = calls.find((x) => x.name === "test");
    expect(c?.setAttributeCalls).toEqual([{ key: "spendguard.tenant_id", value: "t" }]);
  });
});

describe("SPENDGUARD_OTEL_ATTR — design §6.4 attribute table contract", () => {
  it("locks the design §6.4 key names verbatim", () => {
    // §1.2 P0 verbatim-signature gate: these names are the public contract
    // that observability dashboards consume. A rename breaks every adapter
    // dashboard at once.
    expect(SPENDGUARD_OTEL_ATTR.TENANT_ID).toBe("spendguard.tenant_id");
    expect(SPENDGUARD_OTEL_ATTR.DECISION_ID).toBe("spendguard.decision_id");
    expect(SPENDGUARD_OTEL_ATTR.TRIGGER).toBe("spendguard.trigger");
    expect(SPENDGUARD_OTEL_ATTR.OUTCOME_DECISION).toBe("spendguard.outcome.decision");
    expect(SPENDGUARD_OTEL_ATTR.OUTCOME_REASON_CODES).toBe("spendguard.outcome.reason_codes");
    expect(SPENDGUARD_OTEL_ATTR.SDK_VERSION).toBe("spendguard.sdk.version");
  });
});
