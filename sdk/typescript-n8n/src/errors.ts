// `mapToNodeApiError` — translates SpendGuard substrate errors into the
// `NodeApiError` shape n8n surfaces in execution logs.
//
// LOCKED per design.md §7 / review-standards.md §6:
//   - DecisionStopped / DecisionDenied / DecisionSkipped → 403
//   - ApprovalRequired                                   → 428
//   - SidecarUnavailable                                 → 503
//   - HandshakeError                                     → 502
//   - generic Error                                      → pass-through
//   - null / undefined                                   → defensive 500
//
// The mapper NEVER includes prompt text in the message or description
// (A12.4 privacy invariant). Decision IDs / approval request IDs land in
// the description so operators can correlate to the SpendGuard console.

import {
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
  HandshakeError,
  SidecarUnavailable,
} from "@spendguard/sdk";
import { NodeApiError } from "n8n-workflow";
import type { INode } from "n8n-workflow";

function readReasonCodes(err: unknown): string[] {
  const value = (err as { reasonCodes?: unknown })?.reasonCodes;
  if (!Array.isArray(value)) return [];
  return value.filter((v): v is string => typeof v === "string");
}

function readStringField(err: unknown, field: string): string | undefined {
  const value = (err as Record<string, unknown> | null | undefined)?.[field];
  return typeof value === "string" ? value : undefined;
}

export function mapToNodeApiError(node: INode, err: unknown): NodeApiError {
  if (err === null || err === undefined) {
    return new NodeApiError(
      node,
      { message: "SpendGuard returned an empty error" },
      {
        message: "SpendGuard returned an empty error",
        description: "Unknown SpendGuard substrate failure; check sidecar logs.",
        httpCode: "500",
      },
    );
  }

  // ApprovalRequired ALSO extends DecisionDenied — must be checked FIRST
  // so the 428 surface wins. Stopped/Skipped extend DecisionDenied for
  // free; the explicit checks above stay only for telemetry-stable
  // error-class names. Order matters for instanceof dispatch.
  if (err instanceof ApprovalRequired) {
    const approvalRequestId = readStringField(err, "approvalRequestId");
    return new NodeApiError(
      node,
      { message: (err as Error).message ?? "Approval required" },
      {
        message: "SpendGuard requires approval before this call can proceed.",
        description: `Approval request ${approvalRequestId ?? "(unknown)"}. Approve in the SpendGuard console and re-run the workflow.`,
        httpCode: "428",
      },
    );
  }

  // The structural `statusCode === 403` marker is checked alongside
  // `instanceof` so a foreign-realm DENY (dual copy of @spendguard/sdk in the
  // n8n install tree) still surfaces as a 403 NodeApiError rather than a
  // generic failure. Every `DecisionDenied` subclass locks `statusCode === 403`.
  if (
    err instanceof DecisionStopped ||
    err instanceof DecisionDenied ||
    err instanceof DecisionSkipped ||
    (typeof err === "object" &&
      err !== null &&
      (err as { statusCode?: unknown }).statusCode === 403)
  ) {
    const reasonCodes = readReasonCodes(err);
    const reasons = reasonCodes.length > 0 ? reasonCodes.join(", ") : "decision_denied";
    const decisionId = readStringField(err, "decisionId");
    const auditEventId = readStringField(err, "auditDecisionEventId");
    return new NodeApiError(
      node,
      { message: (err as Error).message ?? "SpendGuard denied" },
      {
        message: `SpendGuard denied: ${reasons}`,
        description: `Decision ID ${decisionId ?? "(unknown)"}. Audit event: ${auditEventId ?? "(pending)"}.`,
        httpCode: "403",
      },
    );
  }

  if (err instanceof SidecarUnavailable) {
    return new NodeApiError(
      node,
      { message: (err as Error).message ?? "SpendGuard sidecar unavailable" },
      {
        message: "SpendGuard sidecar unavailable.",
        description:
          "The runner pod could not reach the SpendGuard sidecar UDS; check the socket path credential field.",
        httpCode: "503",
      },
    );
  }

  if (err instanceof HandshakeError) {
    return new NodeApiError(
      node,
      { message: (err as Error).message ?? "SpendGuard handshake failed" },
      {
        message: "SpendGuard handshake failed.",
        description: "TLS / SVID / version negotiation rejected the runner; check sidecar logs.",
        httpCode: "502",
      },
    );
  }

  const errObj = err as Error;
  return new NodeApiError(node, {
    message: errObj.message ?? String(err),
    name: errObj.name ?? "Error",
  });
}
