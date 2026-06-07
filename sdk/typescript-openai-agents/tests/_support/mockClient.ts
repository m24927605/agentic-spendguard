// In-process `SpendGuardClient` double. Implements only the RPCs SLICE 2
// touches (`reserve`, `commitEstimated`) plus the `tenantId` getter the
// adapter reads to thread the idempotency-key canonical tuple. Everything
// else stays undefined so touching it from the SLICE 2 path fails the test
// loudly.

import type {
  CommitEstimatedRequest,
  DecisionOutcome,
  ReserveRequest,
  SpendGuardClient,
} from "@spendguard/sdk";
import { vi } from "vitest";

export interface MockSpendGuardClient {
  client: SpendGuardClient;
  reserve: ReturnType<typeof vi.fn<(req: ReserveRequest) => Promise<DecisionOutcome>>>;
  commitEstimated: ReturnType<typeof vi.fn<(req: CommitEstimatedRequest) => Promise<void>>>;
}

export function makeMockClient(tenantId = "tenant-d08-slice2-test"): MockSpendGuardClient {
  const reserve = vi.fn<(req: ReserveRequest) => Promise<DecisionOutcome>>();
  const commitEstimated = vi.fn<(req: CommitEstimatedRequest) => Promise<void>>();
  reserve.mockResolvedValue(makeOutcome());
  commitEstimated.mockResolvedValue(undefined);
  const client = {
    tenantId,
    reserve,
    commitEstimated,
  } as unknown as SpendGuardClient;
  return { client, reserve, commitEstimated };
}

export function makeOutcome(overrides: Partial<DecisionOutcome> = {}): DecisionOutcome {
  return {
    decisionId: "decision-id-substrate-minted",
    auditDecisionEventId: "audit-evt-1",
    decision: "CONTINUE",
    mutationPatchJson: "{}",
    effectHash: new Uint8Array(0),
    ledgerTransactionId: "ledger-tx-1",
    reservationIds: ["reservation-id-substrate-minted"],
    ttlExpiresAtSeconds: 0,
    reasonCodes: [],
    matchedRuleIds: [],
    ...overrides,
  };
}
