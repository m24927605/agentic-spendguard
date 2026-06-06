// Env-resolution tests (SLICE 3 scope: tests.md §3.11 EN-01..EN-05 plus the
// SLICE-3-specific aliasing/defaults behavior the slice doc names).

import { afterEach, describe, expect, it } from "vitest";

import { EnvParseError, resolveEnvConfig } from "../src/env.js";

const KEYS = [
  "SPENDGUARD_SOCKET_PATH",
  "SPENDGUARD_SIDECAR_UDS",
  "SPENDGUARD_TENANT_ID",
  "SPENDGUARD_WORKLOAD_INSTANCE_ID",
  "SPENDGUARD_DECISION_TIMEOUT_MS",
  "SPENDGUARD_HANDSHAKE_TIMEOUT_MS",
  "SPENDGUARD_RUN_PROJECTION_DEFAULT",
  "SPENDGUARD_DISABLE",
] as const;

afterEach(() => {
  for (const k of KEYS) delete process.env[k];
});

describe("resolveEnvConfig — empty env returns empty snapshot", () => {
  it("returns empty object when no env vars are set", () => {
    for (const k of KEYS) delete process.env[k];
    expect(resolveEnvConfig({})).toEqual({});
  });
});

describe("resolveEnvConfig — socket path aliasing", () => {
  it("reads SPENDGUARD_SOCKET_PATH", () => {
    const env = { SPENDGUARD_SOCKET_PATH: "/var/run/canonical.sock" };
    expect(resolveEnvConfig(env).socketPath).toBe("/var/run/canonical.sock");
  });

  it("falls back to SPENDGUARD_SIDECAR_UDS", () => {
    const env = { SPENDGUARD_SIDECAR_UDS: "/var/run/legacy.sock" };
    expect(resolveEnvConfig(env).socketPath).toBe("/var/run/legacy.sock");
  });

  it("SPENDGUARD_SOCKET_PATH wins over SPENDGUARD_SIDECAR_UDS", () => {
    const env = {
      SPENDGUARD_SOCKET_PATH: "/canonical.sock",
      SPENDGUARD_SIDECAR_UDS: "/legacy.sock",
    };
    expect(resolveEnvConfig(env).socketPath).toBe("/canonical.sock");
  });

  it("empty string is treated as unset", () => {
    const env = { SPENDGUARD_SOCKET_PATH: "" };
    expect(resolveEnvConfig(env).socketPath).toBeUndefined();
  });
});

describe("resolveEnvConfig — tenantId", () => {
  it("reads SPENDGUARD_TENANT_ID", () => {
    expect(resolveEnvConfig({ SPENDGUARD_TENANT_ID: "tenant-A" }).tenantId).toBe("tenant-A");
  });

  it("empty string is treated as unset", () => {
    expect(resolveEnvConfig({ SPENDGUARD_TENANT_ID: "" }).tenantId).toBeUndefined();
  });
});

describe("resolveEnvConfig — workloadInstanceId", () => {
  it("reads SPENDGUARD_WORKLOAD_INSTANCE_ID", () => {
    expect(resolveEnvConfig({ SPENDGUARD_WORKLOAD_INSTANCE_ID: "host-1" }).workloadInstanceId).toBe(
      "host-1",
    );
  });
});

describe("resolveEnvConfig — timeouts", () => {
  it("parses positive integer SPENDGUARD_DECISION_TIMEOUT_MS", () => {
    expect(resolveEnvConfig({ SPENDGUARD_DECISION_TIMEOUT_MS: "500" }).decisionTimeoutMs).toBe(500);
  });

  it("parses positive integer SPENDGUARD_HANDSHAKE_TIMEOUT_MS", () => {
    expect(resolveEnvConfig({ SPENDGUARD_HANDSHAKE_TIMEOUT_MS: "3000" }).handshakeTimeoutMs).toBe(
      3000,
    );
  });

  it("zero is valid (caller may want infinite deadlines)", () => {
    expect(resolveEnvConfig({ SPENDGUARD_DECISION_TIMEOUT_MS: "0" }).decisionTimeoutMs).toBe(0);
  });

  it("throws EnvParseError on non-integer", () => {
    expect(() => resolveEnvConfig({ SPENDGUARD_DECISION_TIMEOUT_MS: "abc" })).toThrowError(
      EnvParseError,
    );
  });

  it("throws EnvParseError on negative", () => {
    expect(() => resolveEnvConfig({ SPENDGUARD_DECISION_TIMEOUT_MS: "-1" })).toThrowError(
      EnvParseError,
    );
  });

  it("throws EnvParseError on float", () => {
    expect(() => resolveEnvConfig({ SPENDGUARD_DECISION_TIMEOUT_MS: "12.5" })).toThrowError(
      EnvParseError,
    );
  });

  it("error carries the varName", () => {
    try {
      resolveEnvConfig({ SPENDGUARD_HANDSHAKE_TIMEOUT_MS: "nope" });
      throw new Error("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(EnvParseError);
      expect((err as EnvParseError).varName).toBe("SPENDGUARD_HANDSHAKE_TIMEOUT_MS");
    }
  });
});

describe("resolveEnvConfig — SPENDGUARD_DISABLE", () => {
  it.each(["1", "true", "TRUE", "True", "yes", "on"])("treats %s as enabled", (value) => {
    expect(resolveEnvConfig({ SPENDGUARD_DISABLE: value }).disabled).toBe(true);
  });

  it.each(["0", "false", "no", "off", "", "garbage"])("treats %s as not enabled", (value) => {
    expect(resolveEnvConfig({ SPENDGUARD_DISABLE: value }).disabled).toBeUndefined();
  });
});

describe("resolveEnvConfig — SPENDGUARD_RUN_PROJECTION_DEFAULT", () => {
  it("reads value into snapshot", () => {
    expect(
      resolveEnvConfig({
        SPENDGUARD_RUN_PROJECTION_DEFAULT: "EMPIRICAL_RUN_CEILING",
      }).runProjectionDefault,
    ).toBe("EMPIRICAL_RUN_CEILING");
  });
});
