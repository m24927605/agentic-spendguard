import { NodeApiError } from "n8n-workflow";
import type { INode } from "n8n-workflow";
export declare function mapToNodeApiError(node: INode, err: unknown): NodeApiError;
