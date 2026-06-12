import type {
  OpenClawStreamFn,
  ProviderPlugin as OpenClawProvider,
} from "openclaw/plugin-sdk/provider-model-shared";
import type { ProviderWrapStreamFnContext } from "openclaw/plugin-sdk/plugin-entry";
import type { BudgetClaim, ReserveRequest, UnitRef } from "@spendguard/sdk";

import {
  OpenClawSpendGuardConfigError,
  OpenClawSpendGuardNotImplementedError,
} from "./errors.js";
import { flattenOpenClawPrompt } from "./flatten.js";
import { OPENCLAW_STEP_ID, OPENCLAW_TRIGGER, prepareOpenClawIdentity } from "./identity.js";
import { validateOptions, type OpenClawSpendGuardOptions } from "./options.js";

export type { OpenClawProvider };

export type OpenClawProviderContext = ProviderWrapStreamFnContext;

const DEFAULT_ROUTE = "openclaw-provider";
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };
const CHARS_PER_TOKEN_HEURISTIC = 4;
const DEFAULT_MICROS_PER_TOKEN = 1_000n;

function notImplementedCatalog(feature: string) {
  return async (): Promise<never> => {
    throw new OpenClawSpendGuardNotImplementedError(feature);
  };
}

export function createSpendGuardOpenClawProvider(
  upstream: OpenClawProvider,
  options: OpenClawSpendGuardOptions,
): OpenClawProvider {
  const opts = validateOptions(options);

  const wrapped: OpenClawProvider = {
    ...upstream,
    wrapStreamFn: (ctx: unknown) => {
      const typedCtx = ctx as ProviderWrapStreamFnContext;
      const inner = resolveInnerStreamFn(upstream, typedCtx);
      return async (params: unknown) => {
        if (inner === undefined) {
          throw new OpenClawSpendGuardNotImplementedError("OpenClaw stream dispatch");
        }
        await reserveBeforeDispatch(params, typedCtx, opts);
        return await inner(params);
      };
    },
  };

  return wrapped;
}

export function buildOpenClawReserveRequest(
  request: unknown,
  context: OpenClawProviderContext,
  options: OpenClawSpendGuardOptions,
): ReserveRequest {
  const flattenedPrompt = flattenOpenClawPrompt(request);
  const providedRunId = options.runIdProvider?.(context);
  const externalRunId =
    typeof providedRunId === "string" && providedRunId.length > 0 ? providedRunId : undefined;
  const identity = prepareOpenClawIdentity({
    tenantId: options.tenantId,
    flattenedPrompt,
    ...(externalRunId !== undefined ? { externalRunId } : {}),
  });
  const projectedClaims = projectClaims(request, context, flattenedPrompt, options);

  return {
    trigger: OPENCLAW_TRIGGER,
    runId: identity.runId,
    stepId: OPENCLAW_STEP_ID,
    llmCallId: identity.llmCallId,
    decisionId: identity.decisionId,
    route: options.route ?? DEFAULT_ROUTE,
    projectedClaims,
    idempotencyKey: identity.idempotencyKey,
  };
}

function resolveInnerStreamFn(
  upstream: OpenClawProvider,
  context: ProviderWrapStreamFnContext,
): OpenClawStreamFn | undefined {
  const upstreamWrapped = upstream.wrapStreamFn?.(context);
  return upstreamWrapped ?? context.streamFn;
}

async function reserveBeforeDispatch(
  request: unknown,
  context: OpenClawProviderContext,
  options: OpenClawSpendGuardOptions,
): Promise<void> {
  await options.client.reserve(buildOpenClawReserveRequest(request, context, options));
}

function projectClaims(
  request: unknown,
  context: OpenClawProviderContext,
  flattenedPrompt: string,
  options: OpenClawSpendGuardOptions,
): BudgetClaim[] {
  const claims =
    options.claimEstimator?.({ request, context, flattenedPrompt }) ??
    defaultClaims(flattenedPrompt, options);
  if (claims.length === 0) {
    throw new OpenClawSpendGuardConfigError("claimEstimator must return at least one claim");
  }
  return claims.map((claim, index) => normalizeClaim(claim, index, options));
}

function defaultClaims(
  flattenedPrompt: string,
  options: OpenClawSpendGuardOptions,
): readonly BudgetClaim[] {
  const estimatedTokens = BigInt(
    Math.max(1, Math.ceil(flattenedPrompt.length / CHARS_PER_TOKEN_HEURISTIC)),
  );
  return [
    {
      scopeId: options.budgetId,
      amountAtomic: (estimatedTokens * DEFAULT_MICROS_PER_TOKEN).toString(),
      unit: { ...DEFAULT_UNIT, unitId: options.unitId },
      windowInstanceId: options.windowInstanceId,
    },
  ];
}

function normalizeClaim(
  claim: BudgetClaim,
  index: number,
  options: OpenClawSpendGuardOptions,
): BudgetClaim {
  if (claim.unit === undefined || claim.unit === null) {
    throw new OpenClawSpendGuardConfigError(`claim[${index}].unit is required`);
  }
  const normalized: BudgetClaim = {
    ...claim,
    unit: { ...claim.unit },
  };
  normalized.unit.unitId = requireNonEmpty(
    normalized.unit.unitId ?? options.unitId,
    `claim[${index}].unit.unitId`,
  );
  normalized.windowInstanceId = requireNonEmpty(
    normalized.windowInstanceId ?? options.windowInstanceId,
    `claim[${index}].windowInstanceId`,
  );
  if (!/^[1-9][0-9]*$/.test(normalized.amountAtomic)) {
    throw new OpenClawSpendGuardConfigError(
      `claim[${index}].amountAtomic must be a positive integer`,
    );
  }
  return normalized;
}

function requireNonEmpty(value: string | undefined, name: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new OpenClawSpendGuardConfigError(`${name} is required`);
  }
  return value;
}
