import type {
  OpenClawStreamFn,
  ProviderPlugin as OpenClawProvider,
} from "openclaw/plugin-sdk/provider-model-shared";
import type { ProviderWrapStreamFnContext } from "openclaw/plugin-sdk/plugin-entry";
import type {
  BudgetClaim,
  CommitEstimatedRequest,
  DecisionOutcome,
  ReserveRequest,
  UnitRef,
} from "@spendguard/sdk";

import {
  OpenClawSpendGuardConfigError,
  OpenClawSpendGuardError,
  OpenClawSpendGuardNotImplementedError,
} from "./errors.js";
import { flattenOpenClawPrompt } from "./flatten.js";
import { OPENCLAW_STEP_ID, OPENCLAW_TRIGGER, prepareOpenClawIdentity } from "./identity.js";
import { validateOptions, type OpenClawSpendGuardOptions } from "./options.js";
import { extractOpenClawUsage, mergeOpenClawUsage, type OpenClawUsage } from "./usage.js";

export type { OpenClawProvider };

export type OpenClawProviderContext = ProviderWrapStreamFnContext;

const DEFAULT_ROUTE = "openclaw-provider";
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };
const CHARS_PER_TOKEN_HEURISTIC = 4;
const DEFAULT_MICROS_PER_TOKEN = 1_000n;

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
        const pending = await reserveBeforeDispatch(params, typedCtx, opts);
        try {
          const response = await inner(params);
          return settleSuccessfulResult(response, pending, opts, params);
        } catch (err) {
          await settleFailure(pending, opts, err, classifyFailure(err, params));
          throw err;
        }
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
): Promise<PendingOpenClawCall> {
  const reserveRequest = buildOpenClawReserveRequest(request, context, options);
  const outcome = await options.client.reserve(reserveRequest);
  return pendingFromDecision(reserveRequest, outcome, options);
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

interface PendingOpenClawReservation {
  reservationId: string;
  projectedAmountAtomic: string;
  unit: UnitRef;
}

interface PendingOpenClawCall {
  runId: string;
  llmCallId: string;
  decisionId: string;
  pricing: OpenClawSpendGuardOptions["pricing"];
  reservations: readonly PendingOpenClawReservation[];
}

type SettlementOutcome = "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";

function pendingFromDecision(
  reserveRequest: ReserveRequest,
  decision: DecisionOutcome,
  options: OpenClawSpendGuardOptions,
): PendingOpenClawCall {
  return {
    runId: reserveRequest.runId,
    llmCallId: reserveRequest.llmCallId,
    decisionId: decision.decisionId || reserveRequest.decisionId,
    pricing: options.pricing,
    reservations: decision.reservationIds.map((reservationId, index) => {
      const claim = reserveRequest.projectedClaims[index] ?? reserveRequest.projectedClaims[0]!;
      return {
        reservationId,
        projectedAmountAtomic: claim.amountAtomic,
        unit: { ...claim.unit },
      };
    }),
  };
}

async function settleSuccessfulResult(
  response: unknown,
  pending: PendingOpenClawCall,
  options: OpenClawSpendGuardOptions,
  request: unknown,
): Promise<unknown> {
  if (isAsyncIterable(response)) {
    return wrapAsyncIterable(response, pending, options, request);
  }
  await settleSuccess(pending, options, extractOpenClawUsage(response));
  return response;
}

async function* wrapAsyncIterable(
  iterable: AsyncIterable<unknown>,
  pending: PendingOpenClawCall,
  options: OpenClawSpendGuardOptions,
  request: unknown,
): AsyncIterable<unknown> {
  let usage: OpenClawUsage | undefined;
  let settled = false;
  let completed = false;
  try {
    for await (const chunk of iterable) {
      usage = mergeOpenClawUsage(usage, extractOpenClawUsage(chunk));
      yield chunk;
    }
    completed = true;
  } catch (err) {
    settled = true;
    await settleFailure(pending, options, err, classifyFailure(err, request));
    throw err;
  } finally {
    if (!settled && !completed) {
      await settleFailure(pending, options, new Error("OpenClaw stream aborted"), "RUN_ABORTED");
    }
  }
  completed = true;
  settled = true;
  await settleSuccess(pending, options, usage);
}

async function settleSuccess(
  pending: PendingOpenClawCall,
  options: OpenClawSpendGuardOptions,
  usage: OpenClawUsage | undefined,
): Promise<void> {
  await settle(pending, options, "SUCCESS", usage);
}

async function settleFailure(
  pending: PendingOpenClawCall,
  options: OpenClawSpendGuardOptions,
  err: unknown,
  outcome: Exclude<SettlementOutcome, "SUCCESS">,
): Promise<void> {
  await settle(pending, options, outcome, undefined, errorMessage(err));
}

async function settle(
  pending: PendingOpenClawCall,
  options: OpenClawSpendGuardOptions,
  outcome: SettlementOutcome,
  usage?: OpenClawUsage,
  error?: string,
): Promise<void> {
  const errors: unknown[] = [];
  for (const reservation of pending.reservations) {
    const req: CommitEstimatedRequest = {
      runId: pending.runId,
      stepId: OPENCLAW_STEP_ID,
      llmCallId: pending.llmCallId,
      decisionId: pending.decisionId,
      reservationId: reservation.reservationId,
      estimatedAmountAtomic: estimateAmountAtomic(reservation, outcome, usage),
      unit: reservation.unit,
      pricing: pending.pricing,
      providerEventId: outcome === "SUCCESS" ? (usage?.providerEventId ?? "") : "",
      outcome,
      ...(outcome === "SUCCESS"
        ? successUsageFields(usage)
        : failureMetadata(outcome, error)),
    };
    try {
      await options.client.commitEstimated(req);
    } catch (err) {
      errors.push(err);
    }
  }
  if (errors.length > 0) {
    throw new OpenClawSpendGuardSettlementError(
      `OpenClaw settlement failed for ${errors.length} reservation(s)`,
      errors,
    );
  }
}

export class OpenClawSpendGuardSettlementError extends OpenClawSpendGuardError {
  readonly errors: readonly unknown[];

  constructor(message: string, errors: readonly unknown[]) {
    super(message);
    this.errors = errors;
  }
}

function estimateAmountAtomic(
  reservation: PendingOpenClawReservation,
  outcome: SettlementOutcome,
  usage: OpenClawUsage | undefined,
): string {
  if (outcome === "SUCCESS" && usage !== undefined) {
    const total = BigInt(usage.inputTokens ?? 0) + BigInt(usage.outputTokens ?? 0);
    if (total > 0n) return total.toString();
  }
  return reservation.projectedAmountAtomic;
}

function successUsageFields(
  usage: OpenClawUsage | undefined,
): Partial<CommitEstimatedRequest> {
  if (usage === undefined) return {};
  return {
    actualInputTokens: usage.inputTokens ?? 0,
    actualOutputTokens: usage.outputTokens ?? 0,
  };
}

function failureMetadata(
  outcome: Exclude<SettlementOutcome, "SUCCESS">,
  error: string | undefined,
): Partial<CommitEstimatedRequest> {
  const metadata = JSON.stringify({
    spendguard_outcome: outcome,
    error_message: error ?? "",
  });
  if (outcome === "PROVIDER_ERROR") {
    return { providerResponseMetadata: metadata };
  }
  return {
    providerResponseMetadata: metadata,
  };
}

function classifyFailure(
  err: unknown,
  request?: unknown,
): Exclude<SettlementOutcome, "SUCCESS"> {
  if (isAbortSignalAborted(request) || isAbortError(err)) return "RUN_ABORTED";
  if (isTimeoutError(err)) return "CLIENT_TIMEOUT";
  return "PROVIDER_ERROR";
}

function isAsyncIterable(value: unknown): value is AsyncIterable<unknown> {
  return (
    value !== null &&
    typeof value === "object" &&
    Symbol.asyncIterator in value &&
    typeof (value as { [Symbol.asyncIterator]?: unknown })[Symbol.asyncIterator] === "function"
  );
}

function isAbortSignalAborted(value: unknown): boolean {
  if (value === null || typeof value !== "object") return false;
  const signal = (value as { signal?: unknown }).signal;
  return (
    signal !== null &&
    typeof signal === "object" &&
    (signal as { aborted?: unknown }).aborted === true
  );
}

function isAbortError(err: unknown): boolean {
  if (err === null || typeof err !== "object") return false;
  const record = err as { name?: unknown; code?: unknown };
  return record.name === "AbortError" || record.code === "ABORT_ERR";
}

function isTimeoutError(err: unknown): boolean {
  if (err === null || typeof err !== "object") return false;
  const record = err as { name?: unknown; code?: unknown; message?: unknown };
  if (record.name === "TimeoutError" || record.code === "ETIMEDOUT") return true;
  return typeof record.message === "string" && /timeout|timed out/i.test(record.message);
}

function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  return String(err);
}
