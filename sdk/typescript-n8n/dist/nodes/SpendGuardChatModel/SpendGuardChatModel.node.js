"use strict";
// `SpendGuardChatModel` — n8n community node that wraps any upstream
// `ai_languageModel` sub-node with SpendGuard reserve+commit gating.
//
// Public surface LOCKED per design.md §4 + review-standards.md §2 + §3.
//
// Drop a SpendGuard Chat Model node between any Chat Model sub-node and
// the AI Agent (or any other consumer of `NodeConnectionTypes.AiLanguageModel`).
// `supplyData` resolves the upstream model, attaches D04's
// `SpendGuardCallbackHandler` to its `callbacks`, and returns the SAME
// model instance verbatim. No Proxy, no clone, no spread.
//
// When the AI Agent invokes the model, D04's handler fires
// `LLM_CALL_PRE`; DENY throws and propagates back through n8n's
// `RunManager`, surfacing as a typed `NodeApiError(httpCode: "403")`.
Object.defineProperty(exports, "__esModule", { value: true });
exports.SpendGuardChatModel = void 0;
const n8n_workflow_1 = require("n8n-workflow");
const langchain_1 = require("@spendguard/langchain");
const clientPool_1 = require("../../src/clientPool");
const errors_1 = require("../../src/errors");
const runIdentity_1 = require("../../src/runIdentity");
const DEFAULT_ROUTE = "llm.call";
const DEFAULT_CLAIM = "1000000";
const DEFAULT_UNIT = "usd_micros";
function isCallbackHandlerArrayMember(arr, target) {
    // Identity equality — review-standards §3.5 demands instance-level
    // dedup, not nominal "same constructor name".
    for (const cb of arr) {
        if (cb === target)
            return true;
    }
    return false;
}
function resolveParams(ctx, itemIndex, credentials) {
    return {
        budgetIdFromCredential: String(credentials.budgetId ?? ""),
        budgetIdOverride: ctx.getNodeParameter("budgetIdOverride", itemIndex, ""),
        route: ctx.getNodeParameter("route", itemIndex, DEFAULT_ROUTE) ||
            DEFAULT_ROUTE,
        runIdSource: ctx.getNodeParameter("runIdSource", itemIndex, "executionId"),
        customRunId: ctx.getNodeParameter("customRunId", itemIndex, ""),
        claimAmountAtomic: ctx.getNodeParameter("claimAmountAtomic", itemIndex, DEFAULT_CLAIM) || DEFAULT_CLAIM,
        unit: ctx.getNodeParameter("unit", itemIndex, DEFAULT_UNIT) ||
            DEFAULT_UNIT,
    };
}
class SpendGuardChatModel {
    description = {
        displayName: "SpendGuard Chat Model",
        name: "spendGuardChatModel",
        icon: "file:spendguard.svg",
        group: ["transform"],
        version: 1,
        description: "Wrap an AI Language Model sub-node with SpendGuard reserve+commit gating. Drop between any Chat Model and the AI Agent.",
        defaults: { name: "SpendGuard Chat Model" },
        codex: {
            categories: ["AI"],
            subcategories: {
                AI: ["Language Models"],
            },
            resources: {
                primaryDocumentation: [
                    {
                        url: "https://agenticspendguard.dev/docs/integrations/n8n/",
                    },
                ],
            },
        },
        inputs: [
            {
                type: n8n_workflow_1.NodeConnectionTypes.AiLanguageModel,
                displayName: "Model",
                required: true,
                maxConnections: 1,
            },
        ],
        outputs: [
            {
                type: n8n_workflow_1.NodeConnectionTypes.AiLanguageModel,
                displayName: "Wrapped Model",
            },
        ],
        credentials: [{ name: "spendGuardApi", required: true }],
        properties: [
            {
                displayName: "Budget ID Override",
                name: "budgetIdOverride",
                type: "string",
                default: "",
                description: "When set, overrides the credential's Budget ID for this node only.",
            },
            {
                displayName: "Route",
                name: "route",
                type: "string",
                default: DEFAULT_ROUTE,
                description: "Telemetry label written to the audit chain's `route` field.",
            },
            {
                displayName: "Run ID Source",
                name: "runIdSource",
                type: "options",
                options: [
                    { name: "Execution ID + Node Name", value: "executionId" },
                    { name: "Node Name", value: "nodeName" },
                    { name: "Custom Expression", value: "custom" },
                ],
                default: "executionId",
                description: "How the SpendGuard `runId` is derived for this node's reservations.",
            },
            {
                displayName: "Custom Run ID",
                name: "customRunId",
                type: "string",
                default: "",
                displayOptions: { show: { runIdSource: ["custom"] } },
                description: "Custom `runId` expression — only used when Run ID Source is set to Custom Expression.",
            },
            {
                displayName: "Claim Amount (USD micros)",
                name: "claimAmountAtomic",
                type: "string",
                default: DEFAULT_CLAIM,
                description: "Atomic budget claim used at PRE time, in USD micros. Defaults to 1_000_000 (= $1.00).",
            },
            {
                displayName: "Unit",
                name: "unit",
                type: "string",
                default: DEFAULT_UNIT,
                description: "Unit string carried on the claim. Defaults to `usd_micros` to match the SpendGuard substrate's canonical money unit.",
            },
        ],
    };
    /**
     * n8n's AI sub-node entry point. The AI Agent calls
     * `getInputConnectionData(AiLanguageModel, 0)` on its
     * `ai_languageModel` input; that call lands here. We do the same one
     * connector upstream, attach D04's callback handler, and return the
     * model verbatim.
     *
     * Wiring contract (review-standards.md §3):
     *   - Exactly one call to `getInputConnectionData`.
     *   - The returned `response` is the SAME object reference as the
     *     upstream model — no Proxy / clone / spread.
     *   - `upstream.callbacks` normalised to an array before any push.
     *   - Duplicate-registration guard via identity equality.
     *   - Only the SpendGuard handler is added; no logger / telemetry side
     *     callbacks.
     */
    async supplyData(itemIndex) {
        const credentials = (await this.getCredentials("spendGuardApi"));
        const params = resolveParams(this, itemIndex, credentials);
        const upstreamRaw = await this.getInputConnectionData(n8n_workflow_1.NodeConnectionTypes.AiLanguageModel, itemIndex);
        if (upstreamRaw === undefined || upstreamRaw === null) {
            throw new Error("SpendGuard Chat Model: no upstream ai_languageModel connected. Wire a Chat Model sub-node into the Model input.");
        }
        const upstream = upstreamRaw;
        try {
            const client = await (0, clientPool_1.acquireClient)(credentials);
            const identity = (0, runIdentity_1.resolveRunIdentity)({ ctx: this, params, itemIndex });
            const handlerOpts = {
                client,
                budgetId: params.budgetIdOverride || params.budgetIdFromCredential,
                // Additive optional fields exercised by D37's wiring. D04 v0.1.0's
                // public surface deliberately omits them; the handler tolerates
                // unknown keys (TypeScript-only contract, no runtime checks). A
                // coordinated D04 v0.1.1 minor brings these into the LOCKED surface.
                sessionIdOverride: identity.sessionId,
                runIdOverride: identity.runId,
                stepId: identity.stepId,
            };
            const handler = new langchain_1.SpendGuardCallbackHandler(handlerOpts);
            const existingCallbacks = upstream.callbacks;
            let normalised;
            if (existingCallbacks === undefined || existingCallbacks === null) {
                normalised = [];
            }
            else if (Array.isArray(existingCallbacks)) {
                normalised = existingCallbacks.slice();
            }
            else {
                normalised = [existingCallbacks];
            }
            if (!isCallbackHandlerArrayMember(normalised, handler)) {
                normalised.push(handler);
            }
            // Cast through unknown — `BaseChatModel.callbacks` is a union of
            // (BaseCallbackHandler | BaseCallbackHandlerMethodsClass | Manager)
            // shapes from `@langchain/core`. We always write a plain
            // BaseCallbackHandler[]; the upstream model treats either shape
            // identically.
            upstream.callbacks = normalised;
            return { response: upstream };
        }
        catch (err) {
            throw (0, errors_1.mapToNodeApiError)(this.getNode(), err);
        }
    }
    /**
     * Non-AI-sub-node `execute` is intentionally absent: this node has NO
     * main flow output, only `ai_languageModel`. n8n's loader only calls
     * `supplyData` for sub-nodes. The stub below exists so the n8n
     * eslint plugin's `node-class-description-name-unsuffixed` doesn't trip;
     * it is unreachable at runtime because the node has no `main` output
     * declared in `description.outputs`.
     */
    async execute() {
        return [[]];
    }
}
exports.SpendGuardChatModel = SpendGuardChatModel;
