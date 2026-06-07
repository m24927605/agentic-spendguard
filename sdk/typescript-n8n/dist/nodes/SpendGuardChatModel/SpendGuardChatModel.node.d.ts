import type { IExecuteFunctions, INodeExecutionData, INodeType, INodeTypeDescription, ISupplyDataFunctions, SupplyData } from "n8n-workflow";
export declare class SpendGuardChatModel implements INodeType {
    description: INodeTypeDescription;
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
    supplyData(this: ISupplyDataFunctions, itemIndex: number): Promise<SupplyData>;
    /**
     * Non-AI-sub-node `execute` is intentionally absent: this node has NO
     * main flow output, only `ai_languageModel`. n8n's loader only calls
     * `supplyData` for sub-nodes. The stub below exists so the n8n
     * eslint plugin's `node-class-description-name-unsuffixed` doesn't trip;
     * it is unreachable at runtime because the node has no `main` output
     * declared in `description.outputs`.
     */
    execute(this: IExecuteFunctions): Promise<INodeExecutionData[][]>;
}
