// COV_D38_02 — identity derivation tests (tests.md TP-07..TP-09, gate A3.3).
//
// `deriveStepIdentity` MUST be a pure composition of the substrate helpers
// (design §6.3 — zero local hashing, review-standards §4). TP-07/TP-08 pin
// the composition; TP-09 pins byte-equality against the Python reference
// implementation (BLAKE2b cross-language P0, D05 §13).

import { deriveIdempotencyKey, deriveUuidFromSignature } from "@spendguard/sdk";
import { describe, expect, it } from "vitest";
import { STEP_ID_LLM_CALL, deriveStepIdentity } from "../src/identity.js";

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

describe("COV_D38_02 identity derivation (TP-07..TP-09)", () => {
  it("TP-07: deriveStepIdentity equals direct substrate calls for 8 fixture tuples", () => {
    const tuples: Array<{ tenantId: string; stepText: string; externalRunId?: string }> = [
      { tenantId: "t-1", stepText: "hello" },
      { tenantId: "t-1", stepText: "hello", externalRunId: "run-ext" },
      { tenantId: "t-2", stepText: "" },
      { tenantId: "00000000-0000-0000-0000-000000000001", stepText: "multi\nline\ntext" },
      { tenantId: "tenant-unicode", stepText: "héllo wörld — 中文 🎯" },
      { tenantId: "t-pipe", stepText: "text|with|pipes" },
      { tenantId: "t-long", stepText: "x".repeat(10_000) },
      { tenantId: "t-ws", stepText: "  leading and trailing  ", externalRunId: "r-ws" },
    ];
    expect(tuples).toHaveLength(8);

    for (const tuple of tuples) {
      const identity = deriveStepIdentity(tuple);
      // Direct substrate composition per design §6.3.
      const signature = `v1|${tuple.tenantId}|${tuple.stepText}`;
      const llmCallId = deriveUuidFromSignature(signature, { scope: "mastra_llm_call_id" });
      const runId = tuple.externalRunId ?? llmCallId;
      const idempotencyKey = deriveIdempotencyKey({
        tenantId: tuple.tenantId,
        sessionId: runId,
        runId,
        stepId: STEP_ID_LLM_CALL,
        llmCallId,
        trigger: "LLM_CALL_PRE",
      });
      expect(identity).toEqual({
        runId,
        llmCallId,
        decisionId: llmCallId,
        idempotencyKey,
      });
      expect(identity.llmCallId).toMatch(UUID_RE);
      expect(identity.decisionId).toBe(identity.llmCallId);
    }
  });

  it("TP-08: same (tenantId, stepText) → identical ids; differing stepText → all three differ", () => {
    const a1 = deriveStepIdentity({ tenantId: "tenant-a", stepText: "same text" });
    const a2 = deriveStepIdentity({ tenantId: "tenant-a", stepText: "same text" });
    expect(a2).toEqual(a1);

    const b = deriveStepIdentity({ tenantId: "tenant-a", stepText: "same text PLUS" });
    expect(b.llmCallId).not.toBe(a1.llmCallId);
    expect(b.decisionId).not.toBe(a1.decisionId);
    expect(b.idempotencyKey).not.toBe(a1.idempotencyKey);
    // Content-derived run id differs too (no external run id supplied).
    expect(b.runId).not.toBe(a1.runId);
  });

  it("TP-09: golden vectors byte-equal to the Python reference (BLAKE2b P0)", () => {
    // Provenance: generated 2026-06-10 against the Python reference
    // implementation (sdk/python/src/spendguard/ids.py):
    //
    //   sig = f"v1|{tenant}|{text}"
    //   llm = str(derive_uuid_from_signature(sig, scope="mastra_llm_call_id"))
    //   run = ext if ext is not None else llm
    //   derive_idempotency_key(tenant_id=tenant, session_id=run, run_id=run,
    //       step_id="llm_call", llm_call_id=llm, trigger="LLM_CALL_PRE")
    //
    // Python is the reference (sdk/fixtures/cross-language/README.md):
    // drift in either direction is a P0 review-standards §2 blocker.
    const golden = [
      {
        tenantId: "tenant-d38",
        stepText: "hello world",
        llmCallId: "75b3b94c-f14d-4c3d-921d-87502d33a0fe",
        idempotencyKey: "sg-93a5046a708a69df0c7d749f85b1b32f",
      },
      {
        tenantId: "tenant-d38",
        stepText: "hello world",
        externalRunId: "run-ext-1",
        llmCallId: "75b3b94c-f14d-4c3d-921d-87502d33a0fe",
        idempotencyKey: "sg-d4dfefdee31a0ca379bb8f5ff34eedbd",
      },
      {
        tenantId: "tenant-d38",
        stepText: "line one\nline two",
        llmCallId: "2153e2c4-dcb1-4f90-835e-9f8f608eb3d9",
        idempotencyKey: "sg-a4cfde603f4ad7888d5f7abf500ad429",
      },
      {
        tenantId: "00000000-0000-0000-0000-000000000001",
        stepText: "",
        llmCallId: "52451177-b40b-4779-b19c-bf61449e79c2",
        idempotencyKey: "sg-b1599c7e7e60e1a976377593d7b06a27",
      },
    ];

    for (const vector of golden) {
      const identity = deriveStepIdentity({
        tenantId: vector.tenantId,
        stepText: vector.stepText,
        ...(vector.externalRunId !== undefined ? { externalRunId: vector.externalRunId } : {}),
      });
      expect(identity.llmCallId).toBe(vector.llmCallId);
      expect(identity.idempotencyKey).toBe(vector.idempotencyKey);
      expect(identity.runId).toBe(vector.externalRunId ?? vector.llmCallId);
    }
  });
});
