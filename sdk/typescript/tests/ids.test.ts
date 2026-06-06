// SpendGuard SDK — ID helper tests (SLICE 6 / COV_S05_06).
//
// Coverage:
//   - newUuid7 RFC 9562 §5.7 byte layout (version=7, variant=10)
//   - newUuid7 timestamp monotonicity (100-sample loop)
//   - newUuid7 randomness (no collisions across N samples)
//   - deriveIdempotencyKey determinism + format
//   - deriveIdempotencyKey domain separation (tenant / session / run / step /
//     llm_call / trigger each independently produce a different key)
//   - deriveUuidFromSignature determinism + version/variant nibbles
//   - deriveUuidFromSignature scope separation
//   - workloadInstanceId env-var read
//
// Spec refs:
//   - design.md §4.6 LOCKED ID helper surface
//   - implementation.md §6
//   - review-standards.md §1.5 cross-language byte-equivalence
//   - tests.md §5.3 cross-language fixture matrix
//
// Cross-language gate (review-standards §2.2, P0 blocker): the TS
// deriveIdempotencyKey + deriveUuidFromSignature outputs MUST match Python's
// `derive_idempotency_key(**kwargs)` / `derive_uuid_from_signature(sig,
// scope=scope)` byte-for-byte for every fixture below. Both runtimes use
// BLAKE2b-128 (`hashlib.blake2b(..., digest_size=16)` on Python; the
// `@noble/hashes` BLAKE2b primitive with `dkLen: 16` on TS).
//
// Fixture generation command (Python, reference implementation):
//   cd sdk/python && uv run python -c "
//     from spendguard.ids import derive_idempotency_key, derive_uuid_from_signature
//     print(derive_idempotency_key(tenant_id='t-1', session_id='s-1',
//         run_id='r-1', step_id='step-1', llm_call_id='llm-1',
//         trigger='LLM_CALL_PRE'))
//     print(derive_uuid_from_signature('sig-abc', scope='decision_id'))"

import { afterEach, describe, expect, it } from "vitest";

import {
  deriveIdempotencyKey,
  deriveUuidFromSignature,
  newUuid7,
  workloadInstanceId,
} from "../src/ids.js";

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

describe("newUuid7() — RFC 9562 §5.7", () => {
  it("returns 36-char canonical hex form", () => {
    const u = newUuid7();
    expect(u).toMatch(UUID_RE);
  });

  it("has version nibble 7 in the 13th hex char", () => {
    const u = newUuid7();
    // Layout: xxxxxxxx-xxxx-7xxx-yxxx-xxxxxxxxxxxx
    // The "7" is the 13th hex char from the start (counting hyphens).
    // Split and check directly: third group's first char.
    const groups = u.split("-");
    expect(groups).toHaveLength(5);
    expect(groups[2]!.charAt(0)).toBe("7");
  });

  it("has variant bits 10xx in the 17th hex char (first char of group 4)", () => {
    // The fourth group starts with a hex digit whose top 2 bits are 10 →
    // first hex char is in {8, 9, a, b}.
    const variants = new Set<string>();
    for (let i = 0; i < 100; i++) {
      const u = newUuid7();
      const groups = u.split("-");
      const v = groups[3]!.charAt(0).toLowerCase();
      variants.add(v);
      expect(["8", "9", "a", "b"]).toContain(v);
    }
    // We expect at least 2 distinct variant nibbles across 100 samples (the
    // random low 62 bits should cycle through them).
    expect(variants.size).toBeGreaterThanOrEqual(2);
  });

  it("is time-monotonic to ms precision across 100 samples", () => {
    // RFC 9562 §5.7 guarantees timestamp prefix is non-decreasing. Two UUIDs
    // generated in the same ms slot are randomized below the timestamp, so
    // we check the leading 48-bit timestamp prefix instead.
    const tsPrefixes: number[] = [];
    for (let i = 0; i < 100; i++) {
      const u = newUuid7();
      const groups = u.split("-");
      // First two groups (xxxxxxxx-xxxx) = 48 bits of timestamp in hex.
      const tsHex = `${groups[0]}${groups[1]}`;
      tsPrefixes.push(Number.parseInt(tsHex, 16));
    }
    for (let i = 1; i < tsPrefixes.length; i++) {
      expect(tsPrefixes[i]).toBeGreaterThanOrEqual(tsPrefixes[i - 1]!);
    }
  });

  it("produces no collisions across 1024 samples (randomness)", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 1024; i++) {
      const u = newUuid7();
      expect(seen.has(u)).toBe(false);
      seen.add(u);
    }
  });
});

describe("deriveIdempotencyKey — cross-language Python parity (P0 gate)", () => {
  // FX1: simple ASCII numeric (the live drift case from R1).
  it("FX1: t-1/s-1/r-1/step-1/llm-1/LLM_CALL_PRE matches Python output", () => {
    const got = deriveIdempotencyKey({
      tenantId: "t-1",
      sessionId: "s-1",
      runId: "r-1",
      stepId: "step-1",
      llmCallId: "llm-1",
      trigger: "LLM_CALL_PRE",
    });
    expect(got).toBe("sg-df6a372619ee74530c2d9e6e4cbbc4b9");
  });

  // FX2: ASCII numeric, alternate index.
  it("FX2: t-2/s-2/r-2/step-2/llm-2/LLM_CALL_PRE matches Python output", () => {
    const got = deriveIdempotencyKey({
      tenantId: "t-2",
      sessionId: "s-2",
      runId: "r-2",
      stepId: "step-2",
      llmCallId: "llm-2",
      trigger: "LLM_CALL_PRE",
    });
    expect(got).toBe("sg-faefca72f21e98f85ca1428a07ff74cf");
  });

  // FX3: UUID tenant — the production-shaped case.
  it("FX3: UUID tenant matches Python output", () => {
    const got = deriveIdempotencyKey({
      tenantId: "00000000-0000-0000-0000-000000000001",
      sessionId: "sess-1",
      runId: "run-1",
      stepId: "step-1",
      llmCallId: "llm-1",
      trigger: "LLM_CALL_PRE",
    });
    expect(got).toBe("sg-8f58c05cb80e39934ac161f5a4c7db40");
  });

  // FX4: empty trigger but other fields populated (degraded path).
  it("FX4: tenant-abc with empty trigger matches Python output", () => {
    const got = deriveIdempotencyKey({
      tenantId: "tenant-abc",
      sessionId: "sess-1",
      runId: "run-1",
      stepId: "step-1",
      llmCallId: "llm-1",
      trigger: "",
    });
    expect(got).toBe("sg-3a2090b41777421828f4362daca7aaa5");
  });

  // FX5: all-empty (degraded but deterministic).
  it("FX5: all empty strings matches Python output", () => {
    const got = deriveIdempotencyKey({
      tenantId: "",
      sessionId: "",
      runId: "",
      stepId: "",
      llmCallId: "",
      trigger: "",
    });
    expect(got).toBe("sg-ff302dc3560c000bc0cf9b9f72359b10");
  });

  // FX6: different trigger (AGENT_STEP_PRE boundary).
  it("FX6: AGENT_STEP_PRE trigger matches Python output", () => {
    const got = deriveIdempotencyKey({
      tenantId: "tenant-xyz",
      sessionId: "sess-42",
      runId: "run-42",
      stepId: "step-7",
      llmCallId: "llm-7",
      trigger: "AGENT_STEP_PRE",
    });
    expect(got).toBe("sg-a0c3ef27b9ef67c2649e3f54763f28cd");
  });

  // FX7: multi-byte UTF-8 tenant id (CJK) — exercises encoding path.
  it("FX7: UTF-8 multi-byte tenant id matches Python output", () => {
    const got = deriveIdempotencyKey({
      tenantId: "租户-甲",
      sessionId: "sess-1",
      runId: "run-1",
      stepId: "step-1",
      llmCallId: "llm-1",
      trigger: "LLM_CALL_PRE",
    });
    expect(got).toBe("sg-95f41144d9bcd120386eeba22b83b74c");
  });
});

describe("deriveUuidFromSignature — cross-language Python parity (P0 gate)", () => {
  // FXU1: live drift case from R1.
  it("FXU1: (sig-abc, decision_id) matches Python output", () => {
    expect(deriveUuidFromSignature("sig-abc", { scope: "decision_id" })).toBe(
      "5f870046-6d3e-4e1d-87bd-d3cbb46ec8e8",
    );
  });

  // FXU2: same signature, different scope — proves namespace separation byte-equivalent.
  it("FXU2: (sig-abc, llm_call_id) matches Python output", () => {
    expect(deriveUuidFromSignature("sig-abc", { scope: "llm_call_id" })).toBe(
      "7c0559d1-18d5-4a6b-9e3e-22a039284498",
    );
  });

  // FXU3: longer signature.
  it("FXU3: (sig-xyz-123, decision_id) matches Python output", () => {
    expect(deriveUuidFromSignature("sig-xyz-123", { scope: "decision_id" })).toBe(
      "e316f49c-49fd-49ac-b555-56922bbcc2a0",
    );
  });

  // FXU4: empty signature (degraded but stable).
  it("FXU4: empty signature matches Python output", () => {
    expect(deriveUuidFromSignature("", { scope: "decision_id" })).toBe(
      "938025bf-d785-41ee-b273-ce4a10474327",
    );
  });

  // FXU5: hex-shaped signature in a different scope.
  it("FXU5: (32-hex signature, trace_id) matches Python output", () => {
    expect(deriveUuidFromSignature("d41d8cd98f00b204e9800998ecf8427e", { scope: "trace_id" })).toBe(
      "ab2a25ae-f070-4467-b176-c366a6371d90",
    );
  });
});

describe("deriveIdempotencyKey() — design.md §4.6 LOCKED", () => {
  const baseArgs = {
    tenantId: "tenant-abc",
    sessionId: "sess-1",
    runId: "run-1",
    stepId: "step-1",
    llmCallId: "llm-1",
    trigger: "LLM_CALL_PRE",
  };

  it("returns the LOCKED 'sg-<32 hex>' format", () => {
    const key = deriveIdempotencyKey(baseArgs);
    expect(key).toMatch(/^sg-[0-9a-f]{32}$/);
  });

  it("is deterministic for the same input", () => {
    const k1 = deriveIdempotencyKey(baseArgs);
    const k2 = deriveIdempotencyKey(baseArgs);
    expect(k1).toBe(k2);
  });

  it("produces distinct keys when EACH field is varied independently", () => {
    const baseKey = deriveIdempotencyKey(baseArgs);
    const variants = [
      { ...baseArgs, tenantId: "tenant-xyz" },
      { ...baseArgs, sessionId: "sess-2" },
      { ...baseArgs, runId: "run-2" },
      { ...baseArgs, stepId: "step-2" },
      { ...baseArgs, llmCallId: "llm-2" },
      { ...baseArgs, trigger: "AGENT_STEP_PRE" },
    ];
    for (const v of variants) {
      expect(deriveIdempotencyKey(v)).not.toBe(baseKey);
    }
  });

  it("uses Unit-Separator joining (collision-safe against concatenation aliasing)", () => {
    // Two inputs that would alias under naive string concatenation but
    // differ under \x1f-separated canonicalisation MUST produce distinct
    // keys. e.g. tenant="ab", session="cd" vs tenant="abcd", session=""
    const a = deriveIdempotencyKey({
      tenantId: "ab",
      sessionId: "cd",
      runId: "x",
      stepId: "y",
      llmCallId: "z",
      trigger: "T",
    });
    const b = deriveIdempotencyKey({
      tenantId: "abcd",
      sessionId: "",
      runId: "x",
      stepId: "y",
      llmCallId: "z",
      trigger: "T",
    });
    expect(a).not.toBe(b);
  });

  it("accepts empty strings (degraded but stable)", () => {
    const k = deriveIdempotencyKey({
      tenantId: "",
      sessionId: "",
      runId: "",
      stepId: "",
      llmCallId: "",
      trigger: "",
    });
    expect(k).toMatch(/^sg-[0-9a-f]{32}$/);
  });
});

describe("deriveUuidFromSignature() — design.md §4.6 LOCKED", () => {
  it("returns canonical UUID-shaped output", () => {
    const u = deriveUuidFromSignature("sig-abc", { scope: "decision_id" });
    expect(u).toMatch(UUID_RE);
  });

  it("has version nibble 4 (RFC 4122 v4)", () => {
    const u = deriveUuidFromSignature("sig-abc", { scope: "decision_id" });
    expect(u.split("-")[2]!.charAt(0)).toBe("4");
  });

  it("has variant bits 10xx in the fourth group", () => {
    const u = deriveUuidFromSignature("sig-abc", { scope: "decision_id" });
    const v = u.split("-")[3]!.charAt(0).toLowerCase();
    expect(["8", "9", "a", "b"]).toContain(v);
  });

  it("is deterministic for the same (signature, scope)", () => {
    const a = deriveUuidFromSignature("sig-abc", { scope: "decision_id" });
    const b = deriveUuidFromSignature("sig-abc", { scope: "decision_id" });
    expect(a).toBe(b);
  });

  it("namespaces different scopes — same signature produces different UUIDs", () => {
    const a = deriveUuidFromSignature("sig-abc", { scope: "decision_id" });
    const b = deriveUuidFromSignature("sig-abc", { scope: "llm_call_id" });
    expect(a).not.toBe(b);
  });
});

describe("workloadInstanceId()", () => {
  const savedEnv = process.env.SPENDGUARD_WORKLOAD_INSTANCE_ID;
  afterEach(() => {
    if (savedEnv === undefined) delete process.env.SPENDGUARD_WORKLOAD_INSTANCE_ID;
    else process.env.SPENDGUARD_WORKLOAD_INSTANCE_ID = savedEnv;
  });

  it("returns empty string when env var unset", () => {
    delete process.env.SPENDGUARD_WORKLOAD_INSTANCE_ID;
    expect(workloadInstanceId()).toBe("");
  });

  it("returns the env var value when set", () => {
    process.env.SPENDGUARD_WORKLOAD_INSTANCE_ID = "wl-1234";
    expect(workloadInstanceId()).toBe("wl-1234");
  });
});
