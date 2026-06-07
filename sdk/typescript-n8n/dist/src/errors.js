"use strict";
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
Object.defineProperty(exports, "__esModule", { value: true });
exports.mapToNodeApiError = mapToNodeApiError;
const sdk_1 = require("@spendguard/sdk");
const n8n_workflow_1 = require("n8n-workflow");
function readReasonCodes(err) {
    const value = err?.reasonCodes;
    if (!Array.isArray(value))
        return [];
    return value.filter((v) => typeof v === "string");
}
function readStringField(err, field) {
    const value = err?.[field];
    return typeof value === "string" ? value : undefined;
}
function mapToNodeApiError(node, err) {
    if (err === null || err === undefined) {
        return new n8n_workflow_1.NodeApiError(node, { message: "SpendGuard returned an empty error" }, {
            message: "SpendGuard returned an empty error",
            description: "Unknown SpendGuard substrate failure; check sidecar logs.",
            httpCode: "500",
        });
    }
    // ApprovalRequired ALSO extends DecisionDenied — must be checked FIRST
    // so the 428 surface wins. Stopped/Skipped extend DecisionDenied for
    // free; the explicit checks above stay only for telemetry-stable
    // error-class names. Order matters for instanceof dispatch.
    if (err instanceof sdk_1.ApprovalRequired) {
        const approvalRequestId = readStringField(err, "approvalRequestId");
        return new n8n_workflow_1.NodeApiError(node, { message: err.message ?? "Approval required" }, {
            message: "SpendGuard requires approval before this call can proceed.",
            description: `Approval request ${approvalRequestId ?? "(unknown)"}. Approve in the SpendGuard console and re-run the workflow.`,
            httpCode: "428",
        });
    }
    if (err instanceof sdk_1.DecisionStopped ||
        err instanceof sdk_1.DecisionDenied ||
        err instanceof sdk_1.DecisionSkipped) {
        const reasonCodes = readReasonCodes(err);
        const reasons = reasonCodes.length > 0 ? reasonCodes.join(", ") : "decision_denied";
        const decisionId = readStringField(err, "decisionId");
        const auditEventId = readStringField(err, "auditDecisionEventId");
        return new n8n_workflow_1.NodeApiError(node, { message: err.message ?? "SpendGuard denied" }, {
            message: `SpendGuard denied: ${reasons}`,
            description: `Decision ID ${decisionId ?? "(unknown)"}. Audit event: ${auditEventId ?? "(pending)"}.`,
            httpCode: "403",
        });
    }
    if (err instanceof sdk_1.SidecarUnavailable) {
        return new n8n_workflow_1.NodeApiError(node, { message: err.message ?? "SpendGuard sidecar unavailable" }, {
            message: "SpendGuard sidecar unavailable.",
            description: "The runner pod could not reach the SpendGuard sidecar UDS; check the socket path credential field.",
            httpCode: "503",
        });
    }
    if (err instanceof sdk_1.HandshakeError) {
        return new n8n_workflow_1.NodeApiError(node, { message: err.message ?? "SpendGuard handshake failed" }, {
            message: "SpendGuard handshake failed.",
            description: "TLS / SVID / version negotiation rejected the runner; check sidecar logs.",
            httpCode: "502",
        });
    }
    const errObj = err;
    return new n8n_workflow_1.NodeApiError(node, {
        message: errObj.message ?? String(err),
        name: errObj.name ?? "Error",
    });
}
