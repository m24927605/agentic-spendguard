// COV_S05_04 SLICE 4 — handshake / reserve / commitEstimated RPC body tests.
//
// Spec coverage:
//   - design.md §4.2 (LOCKED public surface)
//   - design.md §4.5 (handshake lifecycle + idempotency)
//   - design.md §4.7 (reserve → DecisionOutcome / DecisionDenied subclasses)
//   - design.md §4.8 (commitEstimated LLM_CALL_POST single-event)
//   - review-standards.md §1.5 P0 BLOCKER (requestDecision === reserve identity)
//   - implementation.md §4 (skeleton bodies + disabled-mode helpers)
//   - tests.md §3.1 C-01..C-05, §3.2 C-31..C-34
//
// Each test runs against a fresh `MockSidecar` so the wire path is exercised
// end-to-end (UDS bind → protobuf-ts client → server handler → ack). No
// `vi.spyOn` of private fields; the wire is the contract.

import { afterEach, describe, expect, it } from "vitest";

import type {
  DecisionRequest as ProtoDecisionRequest,
  TraceEvent as ProtoTraceEvent,
} from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import {
  DecisionRequest_Trigger,
  DecisionResponse_Decision,
  HandshakeRequest_CapabilityLevel,
  LlmCallPostPayload_Outcome,
  TraceEventAck_Status,
  TraceEvent_EventKind,
} from "../src/_proto/spendguard/sidecar_adapter/v1/adapter.js";
import {
  ApprovalRequired,
  DecisionSkipped,
  DecisionStopped,
  HandshakeError,
  SpendGuardClient,
  SpendGuardError,
} from "../src/index.js";
import { MockSidecar, makeDegradeResponse, makeStopResponse } from "./_support/mockSidecar.js";

// Restore SPENDGUARD_* env between tests so a stray var doesn't leak.
const ENV_KEYS = [
  "SPENDGUARD_SOCKET_PATH",
  "SPENDGUARD_SIDECAR_UDS",
  "SPENDGUARD_TENANT_ID",
  "SPENDGUARD_DISABLE",
  "SPENDGUARD_RUN_PROJECTION_DEFAULT",
] as const;
const savedEnv: Record<string, string | undefined> = {};
for (const k of ENV_KEYS) savedEnv[k] = process.env[k];
afterEach(() => {
  for (const k of ENV_KEYS) {
    if (savedEnv[k] === undefined) delete process.env[k];
    else process.env[k] = savedEnv[k];
  }
});

/** A canonical happy-path ReserveRequest for tests that don't need a custom shape. */
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

/** A canonical happy-path CommitEstimatedRequest. */
function commitReq(overrides: Partial<Parameters<SpendGuardClient["commitEstimated"]>[0]> = {}) {
  return {
    runId: "run-1",
    stepId: "step-1",
    llmCallId: "llm-1",
    decisionId: "d-1",
    reservationId: "mock-reservation-1",
    estimatedAmountAtomic: "500",
    unit: { unit: "USD_MICROS", denomination: 1 },
    pricing: {
      pricingVersion: "v2026.05.09-1",
      pricingHash: new Uint8Array([0x01, 0x02]),
    },
    providerEventId: "pe-1",
    outcome: "SUCCESS" as const,
    ...overrides,
  };
}

// ── §4.5 — handshake() wires real RPC, idempotent, capability check ────────

describe("handshake() — design.md §4.5", () => {
  it("issues HandshakeRequest with sdkVersion + tenantId; caches outcome", async () => {
    const mock = await MockSidecar.start();
    try {
      let captured: { sdkVersion: string; tenantId: string } | null = null;
      mock.hooks.onHandshake = (req) => {
        captured = {
          sdkVersion: req.sdkVersion,
          tenantId: req.tenantIdAssertion,
        };
        return {
          sidecarVersion: "mock-1.2.3",
          schemaBundle: {
            schemaBundleId: "schema-id-1",
            schemaBundleHash: new Uint8Array([0x01]),
            canonicalSchemaVersion: "spendguard.v1alpha1",
          },
          contractBundle: {
            bundleId: "contract-id-1",
            bundleHash: new Uint8Array([0x02]),
            bundleSignature: new Uint8Array(),
            signingKeyId: "key-1",
          },
          capabilityRequired: HandshakeRequest_CapabilityLevel.L3_POLICY_HOOK,
          protocolVersion: 1,
          sessionId: "session-abc",
          signingKeyId: "key-1",
          announcementSignature: new Uint8Array([0xff]),
        };
      };
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-1",
      });
      await client.connect();
      const outcome = await client.handshake();
      expect(outcome.sessionId).toBe("session-abc");
      expect(outcome.sidecarVersion).toBe("mock-1.2.3");
      expect(outcome.schemaBundleId).toBe("schema-id-1");
      expect(outcome.contractBundleId).toBe("contract-id-1");
      expect(outcome.signingKeyId).toBe("key-1");
      expect(captured).toEqual({
        sdkVersion: expect.any(String),
        tenantId: "tenant-1",
      });
      // sessionId getter now resolves without throwing.
      expect(client.sessionId).toBe("session-abc");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("is idempotent — second call returns cached outcome without re-issuing the RPC", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-1",
      });
      await client.connect();
      const a = await client.handshake();
      const b = await client.handshake();
      expect(a).toBe(b); // same object reference — proves the cache hit
      expect(mock.handshakesServed).toBe(1);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("coalesces concurrent handshake() callers into a single in-flight RPC", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-1",
      });
      await client.connect();
      const [a, b, c] = await Promise.all([
        client.handshake(),
        client.handshake(),
        client.handshake(),
      ]);
      expect(a).toBe(b);
      expect(b).toBe(c);
      expect(mock.handshakesServed).toBe(1);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("throws HandshakeError on protocol version mismatch", async () => {
    const mock = await MockSidecar.start({
      onHandshake: () => ({
        sidecarVersion: "mock",
        capabilityRequired: HandshakeRequest_CapabilityLevel.L3_POLICY_HOOK,
        protocolVersion: 2, // mismatch
        sessionId: "s",
        signingKeyId: "",
        announcementSignature: new Uint8Array(),
      }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await expect(client.handshake()).rejects.toThrowError(HandshakeError);
      await expect(client.handshake()).rejects.toThrowError(
        /protocol version mismatch.*adapter=1.*sidecar=2/,
      );
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("throws HandshakeError when sidecar requires higher capability than advertised", async () => {
    const mock = await MockSidecar.start({
      onHandshake: () => ({
        sidecarVersion: "mock",
        capabilityRequired: HandshakeRequest_CapabilityLevel.L4_RUNTIME_NATIVE, // 0x50 > 0x40
        protocolVersion: 1,
        sessionId: "s",
        signingKeyId: "",
        announcementSignature: new Uint8Array(),
      }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await expect(client.handshake()).rejects.toThrowError(/requires capability/);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("clears the in-flight lock on failure so a retry can re-enter", async () => {
    let attempt = 0;
    const mock = await MockSidecar.start({
      onHandshake: () => {
        attempt += 1;
        if (attempt === 1) {
          // First call fails with a version mismatch — second call succeeds.
          return {
            sidecarVersion: "mock",
            capabilityRequired: HandshakeRequest_CapabilityLevel.L3_POLICY_HOOK,
            protocolVersion: 2,
            sessionId: "s1",
            signingKeyId: "",
            announcementSignature: new Uint8Array(),
          };
        }
        return {
          sidecarVersion: "mock",
          capabilityRequired: HandshakeRequest_CapabilityLevel.L3_POLICY_HOOK,
          protocolVersion: 1,
          sessionId: "s2",
          signingKeyId: "",
          announcementSignature: new Uint8Array(),
        };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await expect(client.handshake()).rejects.toThrowError(HandshakeError);
      // Retry succeeds — lock was cleared after the first failure.
      const outcome = await client.handshake();
      expect(outcome.sessionId).toBe("s2");
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── §4.7 — reserve() ALLOW / DENY / DEGRADE ───────────────────────────────

describe("reserve() — design.md §4.7", () => {
  it("CONTINUE: returns DecisionOutcome with reservationIds + reasonCodes", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      const outcome = await client.reserve(reserveReq());
      expect(outcome.decision).toBe("CONTINUE");
      expect(outcome.reservationIds).toEqual(["mock-reservation-1"]);
      expect(outcome.reasonCodes).toEqual(["mock_allow"]);
      expect(outcome.ledgerTransactionId).toBe("mock-tx-1");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("DEGRADE: returns DecisionOutcome with mutationPatchJson", async () => {
    const mock = await MockSidecar.start({
      onRequestDecision: () => makeDegradeResponse({ decisionId: "d-degrade" }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      const outcome = await client.reserve(reserveReq());
      expect(outcome.decision).toBe("DEGRADE");
      expect(outcome.mutationPatchJson).toMatch(/replace.*model.*gpt-4o-mini/);
      expect(outcome.reasonCodes).toContain("mock_degrade");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("STOP: raises DecisionStopped with reasonCodes propagated", async () => {
    const mock = await MockSidecar.start({
      onRequestDecision: () =>
        makeStopResponse({
          decisionId: "d-stop",
          reasonCodes: ["budget_exhausted", "tenant_throttle"],
          matchedRuleIds: ["r-stop"],
        }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      const err = await client
        .reserve(reserveReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(DecisionStopped);
      const stopped = err as DecisionStopped;
      expect(stopped.decisionId).toBe("d-stop");
      expect(stopped.reasonCodes).toEqual(["budget_exhausted", "tenant_throttle"]);
      expect(stopped.matchedRuleIds).toEqual(["r-stop"]);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("STOP_RUN_PROJECTION: raises DecisionStopped (same lattice as STOP)", async () => {
    const mock = await MockSidecar.start({
      onRequestDecision: () => ({
        ...makeStopResponse(),
        decision: DecisionResponse_Decision.STOP_RUN_PROJECTION,
        reasonCodes: ["run_projection_exceeded"],
      }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await expect(client.reserve(reserveReq())).rejects.toBeInstanceOf(DecisionStopped);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("SKIP: raises DecisionSkipped", async () => {
    const mock = await MockSidecar.start({
      onRequestDecision: () => ({
        ...makeStopResponse(),
        decision: DecisionResponse_Decision.SKIP,
        reasonCodes: ["dedup_recent_call"],
      }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await expect(client.reserve(reserveReq())).rejects.toBeInstanceOf(DecisionSkipped);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("REQUIRE_APPROVAL: raises ApprovalRequired with approvalRequestId + tenantId", async () => {
    const mock = await MockSidecar.start({
      onRequestDecision: () => ({
        ...makeStopResponse(),
        decision: DecisionResponse_Decision.REQUIRE_APPROVAL,
        approvalRequestId: "ap-7",
        approverRole: "platform_admin",
        reasonCodes: ["awaiting_human_approval"],
      }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "tenant-xyz",
      });
      await client.connect();
      await client.handshake();
      const err = await client
        .reserve(reserveReq())
        .then(() => null)
        .catch((e: unknown) => e);
      expect(err).toBeInstanceOf(ApprovalRequired);
      const approval = err as ApprovalRequired;
      expect(approval.approvalRequestId).toBe("ap-7");
      expect(approval.approverRole).toBe("platform_admin");
      expect(approval.tenantId).toBe("tenant-xyz");
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("forwards trigger / route / idempotencyKey / decisionId verbatim on the wire", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      const handshake = await client.handshake();
      await client.reserve(
        reserveReq({
          trigger: "AGENT_STEP_PRE",
          route: "anthropic|claude-3-5",
          idempotencyKey: "sg-deadbeefdeadbeef",
          decisionId: "d-explicit",
          runId: "run-explicit",
        }),
      );
      expect(captured).not.toBeNull();
      const req = captured as unknown as ProtoDecisionRequest;
      expect(req.trigger).toBe(DecisionRequest_Trigger.AGENT_STEP_PRE);
      expect(req.route).toBe("anthropic|claude-3-5");
      expect(req.idempotency?.key).toBe("sg-deadbeefdeadbeef");
      expect(req.ids?.decisionId).toBe("d-explicit");
      expect(req.ids?.runId).toBe("run-explicit");
      expect(req.sessionId).toBe(handshake.sessionId);
      // SLICE 7 wires withRunPlan; until then plannedStepsHint stays at proto3
      // default 0 — anti-scope gate.
      expect(req.plannedStepsHint).toBe(0);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("folds runProjectionDefault into runtime_metadata.run_projection_policy when caller did not pass one", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        runProjectionDefault: "STRICT_CEILING",
      });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq());
      const req = captured as unknown as ProtoDecisionRequest;
      const meta = req.inputs?.runtimeMetadata;
      expect(meta).toBeDefined();
      const policyField = meta?.fields?.run_projection_policy;
      expect(policyField?.kind?.oneofKind).toBe("stringValue");
      if (policyField?.kind?.oneofKind === "stringValue") {
        expect(policyField.kind.stringValue).toBe("STRICT_CEILING");
      }
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("caller-supplied decisionContextJson.run_projection_policy wins over runProjectionDefault", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
        runProjectionDefault: "STRICT_CEILING",
      });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          decisionContextJson: { run_projection_policy: "ELASTIC" },
        }),
      );
      const req = captured as unknown as ProtoDecisionRequest;
      const meta = req.inputs?.runtimeMetadata;
      const policyField = meta?.fields?.run_projection_policy;
      expect(policyField?.kind?.oneofKind).toBe("stringValue");
      if (policyField?.kind?.oneofKind === "stringValue") {
        expect(policyField.kind.stringValue).toBe("ELASTIC");
      }
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("omits runtime_metadata entirely when neither caller nor runProjectionDefault provides anything", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await client.reserve(reserveReq());
      const req = captured as unknown as ProtoDecisionRequest;
      // Proto3 default for unset message field = undefined; encoded as
      // absent on the wire.
      expect(req.inputs?.runtimeMetadata).toBeUndefined();
      await client.close();
    } finally {
      await mock.close();
    }
  });

  // SLICE 4 R1 M-3 closure: req.promptText is no longer silently discarded —
  // buildRuntimeMetadataStruct now calls computePromptHash and writes
  // runtime_metadata.prompt_hash. Mirrors Python parity.
  it("M-3 closure: req.promptText drives runtime_metadata.prompt_hash via HMAC-SHA256(promptText, tenantId)", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "00000000-0000-0000-0000-000000000001",
      });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          promptText: "hello world",
        }),
      );
      const req = captured as unknown as ProtoDecisionRequest;
      const meta = req.inputs?.runtimeMetadata;
      expect(meta).toBeDefined();
      const hashField = meta?.fields?.prompt_hash;
      expect(hashField?.kind?.oneofKind).toBe("stringValue");
      if (hashField?.kind?.oneofKind === "stringValue") {
        // Same fixture as tests/promptHash.test.ts FX1 — byte-identical to
        // Python `spendguard.prompt_hash.compute("hello world",
        // "00000000-0000-0000-0000-000000000001")`.
        expect(hashField.kind.stringValue).toBe(
          "5d55a1ebc9782455de0979780fd6cf686127dadcba580f230ddc3fea31516d0d",
        );
      }
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("caller-supplied decisionContextJson.prompt_hash wins over computed value", async () => {
    let captured: ProtoDecisionRequest | null = null;
    const mock = await MockSidecar.start({
      onRequestDecision: (req) => {
        captured = req;
        return {
          decisionId: req.ids?.decisionId ?? "",
          auditDecisionEventId: "",
          decision: DecisionResponse_Decision.CONTINUE,
          reasonCodes: [],
          matchedRuleIds: [],
          mutationPatchJson: "",
          effectHash: new Uint8Array(),
          ledgerTransactionId: "",
          reservationIds: [],
          ttlExpiresAt: { seconds: "0", nanos: 0 },
          approvalRequestId: "",
          approverRole: "",
          terminal: false,
          runCodeTriggered: "",
        };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "00000000-0000-0000-0000-000000000001",
      });
      await client.connect();
      await client.handshake();
      await client.reserve(
        reserveReq({
          promptText: "hello world",
          decisionContextJson: { prompt_hash: "pre-computed-by-upstream-tokenizer" },
        }),
      );
      const req = captured as unknown as ProtoDecisionRequest;
      const hashField = req.inputs?.runtimeMetadata?.fields?.prompt_hash;
      expect(hashField?.kind?.oneofKind).toBe("stringValue");
      if (hashField?.kind?.oneofKind === "stringValue") {
        expect(hashField.kind.stringValue).toBe("pre-computed-by-upstream-tokenizer");
      }
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── §1.5 P0 — requestDecision === reserve identity ────────────────────────

describe("requestDecision === reserve identity (review-standards §1.5 P0)", () => {
  it("client.reserve === client.requestDecision (Boolean-true identity)", () => {
    const client = new SpendGuardClient({
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    });
    // The P0 BLOCKER: must be the SAME function reference, not just functions
    // with equivalent behavior. Instance-field initializer `bind()` guarantees
    // this; a plain method declaration would NOT.
    expect((client as unknown as { reserve: unknown }).reserve).toBe(
      (client as unknown as { requestDecision: unknown }).requestDecision,
    );
  });

  it("two distinct client instances share the same alias (prototype method)", () => {
    // Without bind, the instance field holds the prototype method itself, so
    // `c1.requestDecision === c2.requestDecision === SpendGuardClient.prototype.reserve`.
    // This is intentional (review-standards §1.5 P0); JS method-call dispatch
    // preserves `this` so each instance calls its own state via dot-call.
    const c1 = new SpendGuardClient({ socketPath: "/tmp/x.sock", tenantId: "t" });
    const c2 = new SpendGuardClient({ socketPath: "/tmp/x.sock", tenantId: "t" });
    expect(c1.requestDecision).toBe(c1.reserve);
    expect(c2.requestDecision).toBe(c2.reserve);
    expect(c1.requestDecision).toBe(c2.requestDecision);
  });

  it("requestDecision invocation behaves identically to reserve against a real mock", async () => {
    const mock = await MockSidecar.start();
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      const outA = await client.reserve(reserveReq());
      const outB = await client.requestDecision(reserveReq());
      expect(outA.decision).toBe(outB.decision);
      expect(outA.reservationIds).toEqual(outB.reservationIds);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── §4.8 — commitEstimated() single-event LLM_CALL_POST ───────────────────

describe("commitEstimated() — design.md §4.8", () => {
  it("emits a single LLM_CALL_POST event with estimated_amount_atomic", async () => {
    let captured: ProtoTraceEvent | null = null;
    const mock = await MockSidecar.start({
      onEmitTraceEvents: (event) => {
        captured = event;
        return { eventId: "ack-1", status: TraceEventAck_Status.ACCEPTED };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      const handshake = await client.handshake();
      await client.commitEstimated(
        commitReq({ reservationId: "res-42", estimatedAmountAtomic: "987" }),
      );
      expect(captured).not.toBeNull();
      const ev = captured as unknown as ProtoTraceEvent;
      expect(ev.sessionId).toBe(handshake.sessionId);
      expect(ev.kind).toBe(TraceEvent_EventKind.LLM_CALL_POST);
      expect(ev.payload.oneofKind).toBe("llmCallPost");
      if (ev.payload.oneofKind === "llmCallPost") {
        expect(ev.payload.llmCallPost.reservationId).toBe("res-42");
        expect(ev.payload.llmCallPost.estimatedAmountAtomic).toBe("987");
        // SLICE 5 wires the provider-report path; until then provider amount stays empty.
        expect(ev.payload.llmCallPost.providerReportedAmountAtomic).toBe("");
        expect(ev.payload.llmCallPost.outcome).toBe(LlmCallPostPayload_Outcome.SUCCESS);
      }
      expect(mock.traceEventsServed).toBe(1);
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("forwards actualInputTokens / actualOutputTokens / delta ratios when supplied", async () => {
    let captured: ProtoTraceEvent | null = null;
    const mock = await MockSidecar.start({
      onEmitTraceEvents: (event) => {
        captured = event;
        return { eventId: "ack-2", status: TraceEventAck_Status.ACCEPTED };
      },
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await client.commitEstimated(
        commitReq({
          actualInputTokens: 128,
          actualOutputTokens: 256,
          deltaBRatio: 1.05,
          deltaCRatio: 0.92,
        }),
      );
      const ev = captured as unknown as ProtoTraceEvent;
      if (ev.payload.oneofKind === "llmCallPost") {
        expect(ev.payload.llmCallPost.actualInputTokens).toBe("128");
        expect(ev.payload.llmCallPost.actualOutputTokens).toBe("256");
        expect(ev.payload.llmCallPost.deltaBRatio).toBeCloseTo(1.05, 5);
        expect(ev.payload.llmCallPost.deltaCRatio).toBeCloseTo(0.92, 5);
      } else {
        throw new Error("payload oneofKind mismatch");
      }
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("raises SpendGuardError when the sidecar acks with status != ACCEPTED", async () => {
    const mock = await MockSidecar.start({
      onEmitTraceEvents: () => ({
        eventId: "ack-bad",
        status: TraceEventAck_Status.REJECTED,
        error: {
          code: 13,
          message: "ledger commit rejected",
          details: {},
        },
      }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await expect(client.commitEstimated(commitReq())).rejects.toThrowError(SpendGuardError);
      await expect(client.commitEstimated(commitReq())).rejects.toThrowError(
        /EmitTraceEvents rejected.*REJECTED/,
      );
      await client.close();
    } finally {
      await mock.close();
    }
  });

  it("raises SpendGuardError when sidecar ack is QUARANTINED", async () => {
    const mock = await MockSidecar.start({
      onEmitTraceEvents: () => ({
        eventId: "ack-q",
        status: TraceEventAck_Status.QUARANTINED,
      }),
    });
    try {
      const client = new SpendGuardClient({
        socketPath: mock.socketPath,
        tenantId: "t",
      });
      await client.connect();
      await client.handshake();
      await expect(client.commitEstimated(commitReq())).rejects.toThrowError(/QUARANTINED/);
      await client.close();
    } finally {
      await mock.close();
    }
  });
});

// ── implementation.md §4 lines 822-825 — disabled-mode short-circuits ─────

describe("disabled mode (design.md §5.1 / implementation.md §4 lines 822-825)", () => {
  it("handshake() returns a synthetic disabled outcome without UDS contact", async () => {
    const client = new SpendGuardClient({
      socketPath: "/dev/null/this-path-cannot-exist",
      tenantId: "test",
      disabled: true,
    });
    const outcome = await client.handshake();
    expect(outcome.sessionId).toBe("disabled-noop-session");
    expect(outcome.sidecarVersion).toBe("disabled");
    expect(client.sessionId).toBe("disabled-noop-session");
  });

  it("handshake() is still idempotent in disabled mode", async () => {
    const client = new SpendGuardClient({
      socketPath: "/dev/null/x",
      tenantId: "test",
      disabled: true,
    });
    const a = await client.handshake();
    const b = await client.handshake();
    expect(a).toBe(b);
  });

  it("reserve() returns synthetic CONTINUE with disabled_mode reason code", async () => {
    const client = new SpendGuardClient({
      socketPath: "/dev/null/x",
      tenantId: "test",
      disabled: true,
    });
    await client.handshake();
    const outcome = await client.reserve(reserveReq());
    expect(outcome.decision).toBe("CONTINUE");
    expect(outcome.reasonCodes).toContain("disabled_mode");
    expect(outcome.reservationIds).toEqual([]);
    expect(outcome.decisionId).toBe("d-1");
  });

  it("commitEstimated() resolves to undefined without contacting the sidecar", async () => {
    const client = new SpendGuardClient({
      socketPath: "/dev/null/x",
      tenantId: "test",
      disabled: true,
    });
    await client.handshake();
    await expect(client.commitEstimated(commitReq())).resolves.toBeUndefined();
  });

  it("requestDecision === reserve identity holds in disabled mode too", async () => {
    const client = new SpendGuardClient({
      socketPath: "/dev/null/x",
      tenantId: "test",
      disabled: true,
    });
    expect(client.requestDecision).toBe(client.reserve);
  });
});
