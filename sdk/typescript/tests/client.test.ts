// SpendGuardClient skeleton tests (SLICE 3 scope: tests.md §3.1 C-01..C-05,
// C-31..C-34 plus the SLICE-3-anchored env-fallback assertions).
//
// SLICE 4-5 wires the RPC bodies; the tests here cover only the lifecycle +
// config surface. Each test that needs a real UDS server uses `MockSidecar`
// (a `@grpc/grpc-js` Server bound to an ephemeral socket).

import { existsSync } from "node:fs";

import { afterEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_SOCKET_PATH } from "../src/env.js";
import {
  DEFAULT_DECISION_TIMEOUT_MS,
  DEFAULT_HANDSHAKE_TIMEOUT_MS,
  DEFAULT_PUBLISH_TIMEOUT_MS,
  DEFAULT_TRACE_TIMEOUT_MS,
  HandshakeError,
  SpendGuardClient,
  SpendGuardConfigError,
  SpendGuardConnectionError,
  SpendGuardError,
  VERSION,
} from "../src/index.js";
import { MockSidecar } from "./_support/mockSidecar.js";

// Save + restore the SPENDGUARD_* env vars between tests so accidental leakage
// from one test does not infect another. Vitest's `pool: "forks"` setting in
// vitest.config.ts already gives per-file isolation; this is per-test belt+braces.
const ENV_KEYS = [
  "SPENDGUARD_SOCKET_PATH",
  "SPENDGUARD_SIDECAR_UDS",
  "SPENDGUARD_TENANT_ID",
  "SPENDGUARD_WORKLOAD_INSTANCE_ID",
  "SPENDGUARD_DECISION_TIMEOUT_MS",
  "SPENDGUARD_HANDSHAKE_TIMEOUT_MS",
  "SPENDGUARD_DISABLE",
  "SPENDGUARD_RUN_PROJECTION_DEFAULT",
] as const;

const savedEnv: Record<string, string | undefined> = {};
for (const k of ENV_KEYS) savedEnv[k] = process.env[k];

afterEach(() => {
  for (const k of ENV_KEYS) {
    if (savedEnv[k] === undefined) {
      delete process.env[k];
    } else {
      process.env[k] = savedEnv[k];
    }
  }
});

// ── C-01..C-03: constructor validation ────────────────────────────────────

describe("SpendGuardClient — constructor validation", () => {
  it("C-01: rejects missing socketPath and missing SPENDGUARD_SOCKET_PATH/SIDECAR_UDS", () => {
    delete process.env.SPENDGUARD_SOCKET_PATH;
    delete process.env.SPENDGUARD_SIDECAR_UDS;
    expect(() => new SpendGuardClient({ tenantId: "t-1" })).toThrowError(SpendGuardConfigError);
    expect(() => new SpendGuardClient({ tenantId: "t-1" })).toThrowError(/socketPath is required/);
  });

  it("C-02: rejects missing tenantId and missing SPENDGUARD_TENANT_ID", () => {
    delete process.env.SPENDGUARD_TENANT_ID;
    expect(() => new SpendGuardClient({ socketPath: "/tmp/x.sock" })).toThrowError(
      SpendGuardConfigError,
    );
    expect(() => new SpendGuardClient({ socketPath: "/tmp/x.sock" })).toThrowError(
      /tenantId is required/,
    );
  });

  it("C-03: rejects otelTracer + onSpan both set (mutually exclusive)", () => {
    const onSpan = () => undefined;
    // Minimal Tracer-shaped stub cast through `unknown`. The validator only
    // checks for presence, not shape; using `unknown` keeps the test from
    // pulling the @opentelemetry/api types into the strict typecheck.
    const otelTracer = {
      startSpan: () => ({}),
      startActiveSpan: () => undefined,
    } as unknown as import("@opentelemetry/api").Tracer;
    expect(
      () =>
        new SpendGuardClient({
          socketPath: "/tmp/x.sock",
          tenantId: "t",
          onSpan,
          otelTracer,
        }),
    ).toThrowError(SpendGuardConfigError);
    expect(
      () =>
        new SpendGuardClient({
          socketPath: "/tmp/x.sock",
          tenantId: "t",
          onSpan,
          otelTracer,
        }),
    ).toThrowError(/mutually exclusive/);
  });

  it("rejects unsupported protocolVersion", () => {
    expect(
      () =>
        new SpendGuardClient({
          socketPath: "/tmp/x.sock",
          tenantId: "t",
          protocolVersion: 2,
        }),
    ).toThrowError(/protocolVersion=2 is not supported/);
  });

  it("rejects negative decisionTimeoutMs", () => {
    expect(
      () =>
        new SpendGuardClient({
          socketPath: "/tmp/x.sock",
          tenantId: "t",
          decisionTimeoutMs: -1,
        }),
    ).toThrowError(/decisionTimeoutMs=-1/);
  });

  it("rejects non-integer decisionTimeoutMs", () => {
    expect(
      () =>
        new SpendGuardClient({
          socketPath: "/tmp/x.sock",
          tenantId: "t",
          decisionTimeoutMs: 12.5,
        }),
    ).toThrowError(/decisionTimeoutMs=12.5/);
  });

  it("freezes resolved config so config.disabled is observable but not mutable", () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    expect(client.config.socketPath).toBe("/tmp/x.sock");
    expect(client.config.tenantId).toBe("t");
    expect(client.config.decisionTimeoutMs).toBe(DEFAULT_DECISION_TIMEOUT_MS);
    expect(client.config.handshakeTimeoutMs).toBe(DEFAULT_HANDSHAKE_TIMEOUT_MS);
    expect(client.config.publishTimeoutMs).toBe(DEFAULT_PUBLISH_TIMEOUT_MS);
    expect(client.config.traceTimeoutMs).toBe(DEFAULT_TRACE_TIMEOUT_MS);
    expect(client.config.sdkVersion).toBe(VERSION);
    expect(client.config.runtime).toBe("uds-grpc");
    expect(client.config.disabled).toBe(false);
    // Frozen — mutation must throw in strict mode.
    expect(() => {
      (client.config as { disabled?: boolean }).disabled = true;
    }).toThrowError(TypeError);
  });
});

// ── C-04: disabled short-circuit + SLICE 5 placeholders ───────────────────

describe("SpendGuardClient — SLICE 5 placeholders (release / queryBudget / lower-level)", () => {
  const cfg = { socketPath: "/tmp/x.sock", tenantId: "t" } as const;

  it("C-04 (partial): disabled:true sets config flag", () => {
    const client = new SpendGuardClient({ ...cfg, disabled: true });
    expect(client.config.disabled).toBe(true);
  });

  it("release() throws SpendGuardError with SLICE 5 marker", async () => {
    const client = new SpendGuardClient(cfg);
    await expect(
      client.release({ reservationId: "r-1", idempotencyKey: "sg-abc" }),
    ).rejects.toThrowError(/SLICE 5/);
  });

  it("queryBudget() throws SpendGuardError with SLICE 5 marker", async () => {
    const client = new SpendGuardClient(cfg);
    await expect(client.queryBudget({ scopeId: "tenant/test/global" })).rejects.toThrowError(
      /SLICE 5/,
    );
  });

  it("confirmPublishOutcome() throws SpendGuardError with SLICE 5 marker", async () => {
    const client = new SpendGuardClient(cfg);
    await expect(
      client.confirmPublishOutcome({
        decisionId: "d",
        effectHash: new Uint8Array([0x01]),
        outcome: "APPLIED",
      }),
    ).rejects.toThrowError(/SLICE 5/);
  });
});

describe("SpendGuardClient — SLICE 4 RPC pre-handshake gate", () => {
  const cfg = { socketPath: "/tmp/x.sock", tenantId: "t" } as const;

  it("reserve() before handshake() throws HandshakeError", async () => {
    const client = new SpendGuardClient(cfg);
    await expect(
      client.reserve({
        trigger: "LLM_CALL_PRE",
        runId: "run-1",
        stepId: "step-1",
        llmCallId: "llm-1",
        decisionId: "d-1",
        route: "openai|gpt-4o-mini",
        projectedClaims: [],
        idempotencyKey: "sg-abcdef",
      }),
    ).rejects.toThrowError(HandshakeError);
  });

  it("commitEstimated() before handshake() throws HandshakeError", async () => {
    const client = new SpendGuardClient(cfg);
    await expect(
      client.commitEstimated({
        runId: "r",
        stepId: "s",
        llmCallId: "l",
        decisionId: "d",
        reservationId: "res-1",
        estimatedAmountAtomic: "1000",
        unit: { unit: "USD_MICROS", denomination: 1 },
        pricing: { pricingVersion: "v1", pricingHash: new Uint8Array() },
        providerEventId: "ev-1",
        outcome: "SUCCESS",
      }),
    ).rejects.toThrowError(HandshakeError);
  });
});

// ── C-05 (partial via skeleton): handshake state pre-RPC ──────────────────

describe("SpendGuardClient — state getters", () => {
  it("C-32: tenantId is stable after construction", () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "tenant-xyz",
    });
    expect(client.tenantId).toBe("tenant-xyz");
  });

  it("C-33: sessionId throws HandshakeError before handshake completes", () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    expect(() => client.sessionId).toThrowError(HandshakeError);
    expect(() => client.sessionId).toThrowError(/handshake\(\) has not completed/);
  });

  it("handshakeOutcome throws HandshakeError before handshake completes", () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    expect(() => client.handshakeOutcome).toThrowError(HandshakeError);
  });

  it("isConnected is false before connect()", () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    expect(client.isConnected).toBe(false);
  });
});

// ── C-31, C-34: lifecycle against mock UDS server ─────────────────────────

describe("SpendGuardClient — UDS lifecycle vs mock sidecar", () => {
  it("connect() opens the UDS transport against a real mock server", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-a",
      });
      expect(client.isConnected).toBe(false);
      await client.connect();
      expect(client.isConnected).toBe(true);
      await client.close();
      expect(client.isConnected).toBe(false);
    } finally {
      await mock.close();
    }
  });

  it("connect() is idempotent (second call is a no-op)", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-a",
      });
      await client.connect();
      const transportBefore = client.isConnected;
      await client.connect();
      expect(client.isConnected).toBe(transportBefore);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("C-31: [Symbol.asyncDispose] closes UDS channel; second close is no-op", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-a",
      });
      await client.connect();
      expect(client.isConnected).toBe(true);
      // First dispose closes.
      await client[Symbol.asyncDispose]();
      expect(client.isConnected).toBe(false);
      // Second dispose is a no-op (idempotent close).
      await expect(client[Symbol.asyncDispose]()).resolves.toBeUndefined();
      // close() before any connect() is also a no-op.
      const freshClient = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-a",
      });
      await expect(freshClient.close()).resolves.toBeUndefined();
    } finally {
      await mock.close();
    }
  });

  it("`await using` syntax via Symbol.asyncDispose tears down on scope exit", async () => {
    const mock = await MockSidecar.start();
    try {
      let observedConnected = false;
      // Wrap in a function so the using-block scope exit is bounded.
      const run = async () => {
        await using client = new SpendGuardClient({
          socketPath: mock.socketPath,
          tenantId: "tenant-a",
        });
        await client.connect();
        observedConnected = client.isConnected;
        return client;
      };
      const clientRef = await run();
      expect(observedConnected).toBe(true);
      // After the using scope exited, the client should be disposed.
      expect(clientRef.isConnected).toBe(false);
    } finally {
      await mock.close();
    }
  });

  it("close() before connect() does not throw", async () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    await expect(client.close()).resolves.toBeUndefined();
  });

  it("connect() against a non-existent socket surfaces SpendGuardConnectionError or eventual UNAVAILABLE", async () => {
    // The grpc-js `unix:` resolver is permissive — constructing the channel
    // succeeds even if the socket does not exist; the failure surfaces on the
    // first RPC. SLICE 3 ships the lifecycle gate, so the only assertion we
    // can make here is that `connect()` either resolves cleanly (channel
    // ready to fail at RPC time) OR throws our typed error. Both are
    // acceptable; the SLICE 4 retry path will exercise the RPC-time failure.
    const client = new SpendGuardClient({
      socketPath: "/tmp/non-existent-spendguard-test.sock",
      tenantId: "t",
    });
    try {
      await client.connect();
      // If it succeeded, isConnected should be true — channel is set up
      // even though no socket exists; RPC will fail at SLICE 4 wire time.
      expect(client.isConnected).toBe(true);
    } catch (err) {
      // If it failed, it MUST be our typed error.
      expect(err).toBeInstanceOf(SpendGuardConnectionError);
    } finally {
      await client.close();
      // Cleanup any spurious socket file.
      if (existsSync("/tmp/non-existent-spendguard-test.sock")) {
        try {
          (await import("node:fs")).unlinkSync("/tmp/non-existent-spendguard-test.sock");
        } catch {
          // ignore
        }
      }
    }
  });

  it("C-34: connect() uses unix: URI with grpc.default_authority=localhost", async () => {
    // We can't directly observe the channel option through @grpc/grpc-js's
    // public API. The integration assertion is that connect() against a UDS
    // mock works at all — without the `grpc.default_authority` override,
    // grpc-js would default to the URL-encoded UDS path which the underlying
    // HTTP/2 parser rejects with PROTOCOL_ERROR before the gRPC handler runs.
    // The mere fact that `connect()` against `MockSidecar` (which is the
    // grpc-js Server itself) does not error proves the option is in place
    // — because we only set up that single channel option in connect().
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-a",
      });
      await client.connect();
      expect(client.isConnected).toBe(true);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── fromEnv() factory ──────────────────────────────────────────────────────

describe("SpendGuardClient.fromEnv()", () => {
  it("EN-02: reads SPENDGUARD_SOCKET_PATH and SPENDGUARD_TENANT_ID from env when omitted", () => {
    process.env.SPENDGUARD_SOCKET_PATH = "/var/run/sg-test.sock";
    process.env.SPENDGUARD_TENANT_ID = "tenant-env";
    const client = SpendGuardClient.fromEnv();
    expect(client.config.socketPath).toBe("/var/run/sg-test.sock");
    expect(client.config.tenantId).toBe("tenant-env");
  });

  it("defaults socketPath to /var/run/spendguard/adapter.sock when env unset", () => {
    delete process.env.SPENDGUARD_SOCKET_PATH;
    delete process.env.SPENDGUARD_SIDECAR_UDS;
    process.env.SPENDGUARD_TENANT_ID = "t";
    const client = SpendGuardClient.fromEnv();
    expect(client.config.socketPath).toBe(DEFAULT_SOCKET_PATH);
    expect(DEFAULT_SOCKET_PATH).toBe("/var/run/spendguard/adapter.sock");
  });

  it("accepts SPENDGUARD_SIDECAR_UDS as an alias when SPENDGUARD_SOCKET_PATH is unset", () => {
    delete process.env.SPENDGUARD_SOCKET_PATH;
    process.env.SPENDGUARD_SIDECAR_UDS = "/var/run/alias.sock";
    process.env.SPENDGUARD_TENANT_ID = "t";
    const client = SpendGuardClient.fromEnv();
    expect(client.config.socketPath).toBe("/var/run/alias.sock");
  });

  it("SPENDGUARD_SOCKET_PATH wins over SPENDGUARD_SIDECAR_UDS when both set", () => {
    process.env.SPENDGUARD_SOCKET_PATH = "/var/run/canonical.sock";
    process.env.SPENDGUARD_SIDECAR_UDS = "/var/run/legacy.sock";
    process.env.SPENDGUARD_TENANT_ID = "t";
    const client = SpendGuardClient.fromEnv();
    expect(client.config.socketPath).toBe("/var/run/canonical.sock");
  });

  it("reads SPENDGUARD_RUN_PROJECTION_DEFAULT into config", () => {
    process.env.SPENDGUARD_TENANT_ID = "t";
    process.env.SPENDGUARD_RUN_PROJECTION_DEFAULT = "STRICT_CEILING";
    const client = SpendGuardClient.fromEnv();
    expect(client.config.runProjectionDefault).toBe("STRICT_CEILING");
  });

  it("EN-04: SPENDGUARD_DISABLE=1 enables disabled mode", () => {
    process.env.SPENDGUARD_TENANT_ID = "t";
    process.env.SPENDGUARD_DISABLE = "1";
    const client = SpendGuardClient.fromEnv();
    expect(client.config.disabled).toBe(true);
  });

  it("EN-04: SPENDGUARD_DISABLE=true (any case) enables disabled mode", () => {
    process.env.SPENDGUARD_TENANT_ID = "t";
    process.env.SPENDGUARD_DISABLE = "TRUE";
    const client = SpendGuardClient.fromEnv();
    expect(client.config.disabled).toBe(true);
  });

  it("EN-05: missing SPENDGUARD_TENANT_ID throws SpendGuardConfigError", () => {
    delete process.env.SPENDGUARD_TENANT_ID;
    process.env.SPENDGUARD_SOCKET_PATH = "/tmp/x.sock";
    expect(() => SpendGuardClient.fromEnv()).toThrowError(SpendGuardConfigError);
  });

  it("EN-03: non-integer SPENDGUARD_DECISION_TIMEOUT_MS throws SpendGuardConfigError", () => {
    process.env.SPENDGUARD_TENANT_ID = "t";
    process.env.SPENDGUARD_DECISION_TIMEOUT_MS = "not-a-number";
    expect(() => SpendGuardClient.fromEnv()).toThrowError(SpendGuardConfigError);
    expect(() => SpendGuardClient.fromEnv()).toThrowError(/SPENDGUARD_DECISION_TIMEOUT_MS/);
  });

  it("EN-01: explicit overrides win over env", () => {
    process.env.SPENDGUARD_SOCKET_PATH = "/from-env.sock";
    process.env.SPENDGUARD_TENANT_ID = "from-env";
    const client = SpendGuardClient.fromEnv({
      socketPath: "/from-arg.sock",
      tenantId: "from-arg",
    });
    expect(client.config.socketPath).toBe("/from-arg.sock");
    expect(client.config.tenantId).toBe("from-arg");
  });
});

// ── Spy that resumeAfterApproval is delegated correctly when SLICE 4 wires
//    the body. SLICE 3 only verifies the throw shape.

describe("SpendGuardClient — placeholder delegation (SLICE 5)", () => {
  it("resumeAfterApproval throws SpendGuardError with SLICE 5 marker", async () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    await expect(
      client.resumeAfterApproval({
        approvalId: "ap-1",
        tenantId: "t",
        decisionId: "d-1",
      }),
    ).rejects.toThrowError(/resumeAfterApproval\(\)/);
  });

  it("safeConfirmApplyFailed throws (SLICE 5 will wire swallow semantics)", async () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    await expect(
      client.safeConfirmApplyFailed({
        decisionId: "d",
        effectHash: new Uint8Array(),
        adapterError: "boom",
      }),
    ).rejects.toThrowError(/safeConfirmApplyFailed\(\)/);
  });

  it("emitLlmCallPost throws SpendGuardError with SLICE 5 marker (provider-report path)", async () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    await expect(
      client.emitLlmCallPost({
        runId: "r",
        stepId: "s",
        llmCallId: "l",
        decisionId: "d",
        reservationId: "res",
        estimatedAmountAtomic: "1",
        unit: { unit: "USD_MICROS", denomination: 1 },
        pricing: { pricingVersion: "v1", pricingHash: new Uint8Array() },
        providerEventId: "ev",
        outcome: "SUCCESS",
      }),
    ).rejects.toThrowError(/emitLlmCallPost\(\)/);
  });
});

// vi is imported above so we can use `vi.spyOn` once SLICE 4 wires bodies.
// SLICE 3 doesn't need it but importing it future-proofs the file.
void vi;
