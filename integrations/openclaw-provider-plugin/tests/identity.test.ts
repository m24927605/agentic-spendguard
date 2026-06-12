import { describe, expect, it } from "vitest";

import { prepareOpenClawIdentity } from "../src/identity.js";
import { deriveIdempotencyKey, deriveUuidFromSignature } from "@spendguard/sdk";

describe("OpenClaw identity", () => {
  it("delegates UUID and idempotency derivation to the SpendGuard SDK", () => {
    const identity = prepareOpenClawIdentity({
      tenantId: "tenant_1",
      externalRunId: "run_1",
      flattenedPrompt: "hello",
    });
    const signature = "v1|openclaw|tenant_1|run_1|hello";
    const llmCallId = deriveUuidFromSignature(signature, { scope: "openclaw_llm_call_id" });

    expect(identity.llmCallId).toBe(llmCallId);
    expect(identity.decisionId).toBe(
      deriveUuidFromSignature(signature, { scope: "openclaw_decision_id" }),
    );
    expect(identity.idempotencyKey).toBe(
      deriveIdempotencyKey({
        tenantId: "tenant_1",
        sessionId: "run_1",
        runId: "run_1",
        stepId: "llm_call",
        llmCallId,
        trigger: "LLM_CALL_PRE",
      }),
    );
  });
});
