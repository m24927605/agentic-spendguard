// Convert the no-code `claimEstimatorJson` input into a callable that the
// D04 SpendGuardCallbackHandler can use as its `claimEstimator`.
//
// Public surface — LOCKED at design.md §4. The empty-JSON default
// produces a conservative $1 USD-micros claim per call so a Flowise
// builder can drop the wrapper onto a canvas without writing code; the
// JSON override is the operator's escape hatch to tighten per-route.

/**
 * Minimal claim shape — kept structural so we don't pull a type from
 * `@spendguard/langchain` at module load (it's a peerDep). The
 * SpendGuardCallbackHandler accepts an array of these per call.
 */
export interface ClaimEntry {
  scopeId: string;
  amountAtomic: string;
  unit: string;
}

export type ClaimEstimatorFn = () => ClaimEntry[];

export interface BuildClaimEstimatorArgs {
  /** Operator-supplied JSON. Empty / whitespace-only → conservative default. */
  json: string;
  /** Default unit when the JSON omits it (e.g. `usd_micros`). */
  unit: string;
}

/**
 * `1_000_000` atomic units in `usd_micros` ≡ $1 USD. Conservative and
 * documented in the node's description and the docs page; safe for a
 * no-code drop-in path.
 */
export const DEFAULT_CLAIM_ATOMIC = "1000000" as const;
export const DEFAULT_CLAIM_SCOPE = "default" as const;

interface ClaimJsonShape {
  amountAtomic?: string;
  scopeId?: string;
  unit?: string;
}

/**
 * Returns a `ClaimEstimatorFn` matching the shape D04's
 * SpendGuardCallbackHandler expects. The function is stateless — the
 * returned closure captures the parsed JSON and reuses it across calls.
 *
 * Locked at design.md §5: the wrapper hot-path MUST NOT JSON.parse on
 * every chat invocation; parsing happens once at `init()`.
 */
export function buildClaimEstimator({ json, unit }: BuildClaimEstimatorArgs): ClaimEstimatorFn {
  if (!json.trim()) {
    return (): ClaimEntry[] => [
      { scopeId: DEFAULT_CLAIM_SCOPE, amountAtomic: DEFAULT_CLAIM_ATOMIC, unit },
    ];
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch (err) {
    throw new Error(
      `SpendGuardChatModelWrapper: claimEstimatorJson is not valid JSON: ${(err as Error).message}`,
    );
  }
  if (typeof parsed !== "object" || parsed === null) {
    throw new Error("SpendGuardChatModelWrapper: claimEstimatorJson must be a JSON object");
  }
  const claim = parsed as ClaimJsonShape;
  if (typeof claim.amountAtomic !== "string" || !claim.amountAtomic) {
    throw new Error(
      "SpendGuardChatModelWrapper: claimEstimatorJson must include 'amountAtomic' as a decimal string",
    );
  }
  if (!/^[0-9]+$/.test(claim.amountAtomic)) {
    throw new Error(
      `SpendGuardChatModelWrapper: claimEstimatorJson.amountAtomic must be a decimal string, got '${claim.amountAtomic}'`,
    );
  }
  const scopeId =
    typeof claim.scopeId === "string" && claim.scopeId ? claim.scopeId : DEFAULT_CLAIM_SCOPE;
  const claimUnit = typeof claim.unit === "string" && claim.unit ? claim.unit : unit;
  return (): ClaimEntry[] => [
    { scopeId, amountAtomic: claim.amountAtomic as string, unit: claimUnit },
  ];
}
