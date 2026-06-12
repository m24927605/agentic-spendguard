import { deriveIdempotencyKey, deriveUuidFromSignature } from "@spendguard/sdk";

export const OPENCLAW_STEP_ID = "llm_call";
export const OPENCLAW_TRIGGER = "LLM_CALL_PRE";

const RUN_ID_SCOPE = "openclaw_run_id";
const LLM_CALL_ID_SCOPE = "openclaw_llm_call_id";
const DECISION_ID_SCOPE = "openclaw_decision_id";

export interface OpenClawIdentity {
  runId: string;
  llmCallId: string;
  decisionId: string;
  idempotencyKey: string;
}

export function prepareOpenClawIdentity(input: {
  tenantId: string;
  flattenedPrompt: string;
  externalRunId?: string;
}): OpenClawIdentity {
  const runId =
    input.externalRunId ??
    deriveUuidFromSignature(`v1|openclaw_run|${input.tenantId}|${input.flattenedPrompt}`, {
      scope: RUN_ID_SCOPE,
    });
  const signature = `v1|openclaw|${input.tenantId}|${runId}|${input.flattenedPrompt}`;
  const llmCallId = deriveUuidFromSignature(signature, { scope: LLM_CALL_ID_SCOPE });
  const decisionId = deriveUuidFromSignature(signature, { scope: DECISION_ID_SCOPE });
  return {
    runId,
    llmCallId,
    decisionId,
    idempotencyKey: deriveIdempotencyKey({
      tenantId: input.tenantId,
      sessionId: runId,
      runId,
      stepId: OPENCLAW_STEP_ID,
      llmCallId,
      trigger: OPENCLAW_TRIGGER,
    }),
  };
}
