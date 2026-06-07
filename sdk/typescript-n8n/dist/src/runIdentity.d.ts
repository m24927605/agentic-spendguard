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
export declare function resolveRunIdentity(args: ResolveRunIdentityArgs): RunIdentity;
