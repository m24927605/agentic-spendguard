"use strict";
// D37 — SpendGuard API credential definition.
//
// Public surface LOCKED per docs/specs/coverage/D37_n8n/design.md §4 +
// review-standards.md §2:
//   - credential name: "spendGuardApi"
//   - displayName: "SpendGuard API"
//   - properties in order: tenantId, socketPath, budgetId,
//     windowInstanceId, runtimeKind
//   - NO `test` function (handshake is lazy — the sidecar UDS may not
//     be reachable from the n8n UI host, only from the runner pod where
//     the workflow executes).
//
// Drift here breaks the audit-chain idempotency invariant. Any addition
// or reorder requires a coordinated D37 minor bump per
// review-standards.md §2.13.
Object.defineProperty(exports, "__esModule", { value: true });
exports.SpendGuardApi = void 0;
class SpendGuardApi {
    name = "spendGuardApi";
    displayName = "SpendGuard API";
    documentationUrl = "https://agenticspendguard.dev/docs/integrations/n8n/";
    properties = [
        {
            displayName: "Tenant ID",
            name: "tenantId",
            type: "string",
            default: "",
            required: true,
            description: "SpendGuard tenant identifier (UUID).",
        },
        {
            displayName: "Sidecar UDS Path",
            name: "socketPath",
            type: "string",
            default: "/var/run/spendguard/sidecar.sock",
            description: "Unix domain socket path the runner pod uses to reach the SpendGuard sidecar; required for v0.1.x.",
        },
        {
            displayName: "Budget ID",
            name: "budgetId",
            type: "string",
            default: "",
            required: true,
            description: "UUID of the SpendGuard budget to charge by default.",
        },
        {
            displayName: "Window Instance ID",
            name: "windowInstanceId",
            type: "string",
            default: "",
            required: true,
            description: "UUID of the active SpendGuard window instance.",
        },
        {
            displayName: "Runtime Kind",
            name: "runtimeKind",
            type: "string",
            default: "n8n",
            description: "Telemetry attribution; forwarded to SpendGuard. Override only if you need to disambiguate multi-tenant n8n installs.",
        },
    ];
}
exports.SpendGuardApi = SpendGuardApi;
