// SpendGuardChatModelWrapper — the single Flowise INode this package
// publishes. The canvas builder drops it between any BaseChatModel and
// the downstream Chain / Agent; the wrapper attaches D04's
// SpendGuardCallbackHandler to the chat model's `callbacks` array
// in place, returning the SAME chat model reference so downstream nodes
// see a normal BaseChatModel.
//
// Locked decisions (design.md §4–§6, implementation.md §3):
//   - One wrapper node, not per-provider nodes.
//   - Mutate the inner model's `callbacks` array; do not subclass or proxy.
//   - Cache the SpendGuardClient per `(tenantId, sidecarUds)` so
//     re-running `init()` per invocation does not re-open the UDS.
//   - Conservative $1 USD-micros default claim; `claimEstimatorJson`
//     overrides per-route.
//   - `module.exports = { nodeClass }` shim because Flowise's loader
//     reads `nodeClass` off the file's default export.

import { buildClaimEstimator, type ClaimEstimatorFn } from "../claimEstimator.js";
import { getOrCreateClient, type CachedClient } from "../clientCache.js";
import { VERSION } from "../version.js";

// Structural Flowise INode types — kept local so the package compiles
// without `flowise-components` resolved (it's a peerDep). The shapes
// match the Flowise 2.x INode contract documented at
// https://docs.flowiseai.com/integrations/custom-tool .
export interface FlowiseNodeInput {
  label: string;
  name: string;
  type: string;
  default?: string;
  optional?: boolean;
  rows?: number;
  description?: string;
}

export interface FlowiseNodeData {
  inputs?: Record<string, unknown>;
}

export type FlowiseCommonObject = Record<string, unknown>;

/**
 * Minimal structural BaseChatModel — the wrapper only touches `callbacks`.
 * The real type lives in `@langchain/core/language_models/chat_models`
 * (a transitive peer through `@spendguard/langchain`).
 */
export interface MutableChatModel {
  // biome-ignore lint/suspicious/noExplicitAny: callback handler shape is opaque here
  callbacks?: Array<any> | undefined;
}

interface HandlerFactoryDeps {
  client: CachedClient;
  budgetId: string;
  windowInstanceId: string;
  unit: string;
  claimEstimator: ClaimEstimatorFn;
  route: string;
}

/**
 * Lazy import of `@spendguard/langchain` — keeps the peerDep contract
 * honest. Override in tests via `_setHandlerFactoryForTests`.
 */
let handlerFactoryOverride:
  | ((deps: HandlerFactoryDeps) => Promise<unknown>)
  | undefined;

export function _setHandlerFactoryForTests(
  factory: ((deps: HandlerFactoryDeps) => Promise<unknown>) | undefined,
): void {
  handlerFactoryOverride = factory;
}

async function defaultHandlerFactory(deps: HandlerFactoryDeps): Promise<unknown> {
  const mod = (await import("@spendguard/langchain")) as {
    SpendGuardCallbackHandler: new (opts: {
      client: unknown;
      budgetId: string;
      windowInstanceId: string;
      unit: string;
      claimEstimator: ClaimEstimatorFn;
      route: string;
    }) => unknown;
  };
  return new mod.SpendGuardCallbackHandler({
    client: deps.client.raw,
    budgetId: deps.budgetId,
    windowInstanceId: deps.windowInstanceId,
    unit: deps.unit,
    claimEstimator: deps.claimEstimator,
    route: deps.route,
  });
}

export class SpendGuardChatModelWrapper {
  readonly label = "SpendGuard ChatModel Wrapper";
  readonly name = "spendGuardChatModelWrapper";
  readonly version = 1.0;
  readonly type = "BaseChatModel";
  readonly icon = "spendguard.svg";
  readonly category = "Spend Guard";
  readonly description =
    "Wraps any ChatModel with SpendGuard pre-call budget enforcement. " +
    "Drop this between your ChatModel and your Chain / Agent.";
  readonly baseClasses: ReadonlyArray<string> = ["BaseChatModel", "BaseLanguageModel"];

  readonly inputs: ReadonlyArray<FlowiseNodeInput> = [
    { label: "Chat Model", name: "chatModel", type: "BaseChatModel" },
    { label: "Tenant ID", name: "tenantId", type: "string" },
    { label: "Budget ID", name: "budgetId", type: "string" },
    { label: "Budget Window Instance ID", name: "windowInstanceId", type: "string" },
    { label: "Unit", name: "unit", type: "string", default: "usd_micros" },
    {
      label: "Sidecar UDS Path",
      name: "sidecarUds",
      type: "string",
      optional: true,
      description:
        "Defaults to env SPENDGUARD_SIDECAR_UDS. Path to the SpendGuard sidecar's Unix-domain socket.",
    },
    {
      label: "Route",
      name: "route",
      type: "string",
      default: "llm.call",
      optional: true,
    },
    {
      label: "Claim Estimator (JSON)",
      name: "claimEstimatorJson",
      type: "string",
      rows: 4,
      optional: true,
      description:
        'JSON describing a fixed claim, e.g. {"amountAtomic":"1000000","scopeId":"default"}. ' +
        "Omit for a conservative $1 USD-micros default per call.",
    },
  ];

  async init(
    nodeData: FlowiseNodeData,
    _input: string,
    _options: FlowiseCommonObject,
  ): Promise<unknown> {
    const chatModel = nodeData.inputs?.chatModel as MutableChatModel | undefined;
    if (!chatModel) {
      throw new Error(
        "SpendGuardChatModelWrapper: chatModel input is required",
      );
    }

    const tenantId = String(nodeData.inputs?.tenantId ?? "").trim();
    const budgetId = String(nodeData.inputs?.budgetId ?? "").trim();
    const windowInstanceId = String(nodeData.inputs?.windowInstanceId ?? "").trim();
    const unit = String(nodeData.inputs?.unit ?? "usd_micros").trim() || "usd_micros";
    const route = String(nodeData.inputs?.route ?? "llm.call").trim() || "llm.call";
    const claimEstimatorJson = String(nodeData.inputs?.claimEstimatorJson ?? "");
    const sidecarUds = String(
      nodeData.inputs?.sidecarUds ?? process.env.SPENDGUARD_SIDECAR_UDS ?? "",
    ).trim();

    if (!tenantId || !budgetId || !windowInstanceId || !sidecarUds) {
      throw new Error(
        "SpendGuardChatModelWrapper: tenantId, budgetId, windowInstanceId, and " +
          "sidecarUds (or env SPENDGUARD_SIDECAR_UDS) are all required",
      );
    }

    const client = await getOrCreateClient({ sidecarUds, tenantId });
    const claimEstimator = buildClaimEstimator({ json: claimEstimatorJson, unit });

    const factory = handlerFactoryOverride ?? defaultHandlerFactory;
    const handler = await factory({
      client,
      budgetId,
      windowInstanceId,
      unit,
      claimEstimator,
      route,
    });

    // Mutate in place so downstream Chain / Agent nodes see the handler.
    // We append to the existing array rather than replace it so any other
    // callbacks the canvas builder configured (LangSmith, custom
    // tracers) survive.
    const existing = Array.isArray(chatModel.callbacks) ? chatModel.callbacks : [];
    chatModel.callbacks = [...existing, handler];
    return chatModel;
  }
}

/** Package version surfaced for debugging. */
export const NODE_VERSION = VERSION;

// Flowise's loader convention: the file's default export should carry a
// `nodeClass` property. tsup emits ESM + a CJS shim; we provide both
// shapes so either loader resolves to the wrapper class.
const _nodeClass = SpendGuardChatModelWrapper;
export default { nodeClass: _nodeClass };
export const nodeClass = _nodeClass;
