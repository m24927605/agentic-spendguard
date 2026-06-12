import type { BudgetClaim, PricingFreeze, SpendGuardClient } from "@spendguard/sdk";

import { OpenClawSpendGuardConfigError } from "./errors.js";
import type { OpenClawProviderContext } from "./provider.js";

export type OpenClawClaimEstimator = (input: {
  request: unknown;
  context: OpenClawProviderContext;
  flattenedPrompt: string;
}) => readonly BudgetClaim[];

export interface OpenClawSpendGuardOptions {
  client: SpendGuardClient;
  tenantId: string;
  budgetId: string;
  windowInstanceId: string;
  unitId: string;
  pricing: PricingFreeze;
  route?: string;
  claimEstimator?: OpenClawClaimEstimator;
  runIdProvider?: (ctx: OpenClawProviderContext) => string;
}

export function validateOptions(options: OpenClawSpendGuardOptions): OpenClawSpendGuardOptions {
  if (!options) {
    throw new OpenClawSpendGuardConfigError("OpenClaw SpendGuard options are required");
  }
  requireObject(options.client, "client");
  requireNonEmptyString(options.tenantId, "tenantId");
  requireNonEmptyString(options.budgetId, "budgetId");
  requireNonEmptyString(options.windowInstanceId, "windowInstanceId");
  requireNonEmptyString(options.unitId, "unitId");
  requirePricing(options.pricing);
  if (options.route !== undefined) {
    requireNonEmptyString(options.route, "route");
  }
  if (options.claimEstimator !== undefined && typeof options.claimEstimator !== "function") {
    throw new OpenClawSpendGuardConfigError("claimEstimator must be a function");
  }
  if (options.runIdProvider !== undefined && typeof options.runIdProvider !== "function") {
    throw new OpenClawSpendGuardConfigError("runIdProvider must be a function");
  }
  return options;
}

function requireObject(value: unknown, name: string): void {
  if (value === null || typeof value !== "object") {
    throw new OpenClawSpendGuardConfigError(`${name} is required`);
  }
}

function requireNonEmptyString(value: unknown, name: string): void {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new OpenClawSpendGuardConfigError(`${name} is required`);
  }
}

function requirePricing(value: unknown): asserts value is PricingFreeze {
  requireObject(value, "pricing");
  const pricing = value as Partial<PricingFreeze>;
  requireNonEmptyString(pricing.pricingVersion, "pricing.pricingVersion");
  if (!(pricing.pricingHash instanceof Uint8Array) || pricing.pricingHash.length === 0) {
    throw new OpenClawSpendGuardConfigError("pricing.pricingHash is required");
  }
}
