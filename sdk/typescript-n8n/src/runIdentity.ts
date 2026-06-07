// `resolveRunIdentity` — derives the `(sessionId, runId, stepId)` triple
// the SpendGuard substrate's audit chain keys on, from n8n's runtime
// context.
//
// LOCKED per design.md §5 / review-standards.md §4:
//   - sessionId = executionId (via ISupplyDataFunctions.getExecutionId())
//   - stepId    = node name (the n8n node display name)
//   - runId     = depends on runIdSource:
//                  "executionId" → `${executionId}:${nodeName}`
//                  "nodeName"    → nodeName
//                  "custom"      → customRunId (or executionId fallback
//                                  when customRunId is empty)
//
// The `runId` is what lands on `Reserve.runId` AND drives the substrate's
// `deriveIdempotencyKey` derivation — round-trip parity is the P-01 +
// P-02 gate from tests.md §5.

import type { ISupplyDataFunctions } from "n8n-workflow";

export type RunIdSource = "executionId" | "nodeName" | "custom";

export interface RunIdentity {
  sessionId: string;
  runId: string;
  stepId: string;
}

export interface ResolveRunIdentityArgs {
  ctx: ISupplyDataFunctions;
  params: {
    runIdSource: RunIdSource;
    customRunId: string;
  };
  itemIndex: number;
}

export function resolveRunIdentity(args: ResolveRunIdentityArgs): RunIdentity {
  const executionId = String(args.ctx.getExecutionId());
  const nodeName = args.ctx.getNode().name;

  let runId: string;
  switch (args.params.runIdSource) {
    case "nodeName":
      runId = nodeName;
      break;
    case "custom":
      runId =
        args.params.customRunId && args.params.customRunId.length > 0
          ? args.params.customRunId
          : `${executionId}:${nodeName}`;
      break;
    default:
      runId = `${executionId}:${nodeName}`;
      break;
  }

  return { sessionId: executionId, runId, stepId: nodeName };
}
