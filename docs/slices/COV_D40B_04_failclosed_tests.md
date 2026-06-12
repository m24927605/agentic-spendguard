# COV_D40B_04 - OpenClaw fail-closed and package test completion

> **Deliverable:** D40b OpenClaw provider plugin adapter
> **Slice:** 4 of 6
> **Spec set:** [`docs/specs/coverage/D40b_openclaw_provider_plugin/`](../specs/coverage/D40b_openclaw_provider_plugin/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Complete the full package test matrix: public surface, fail-closed, tuple matching, hash-reuse, bundle size, and build gates.

## LOCKED design quotes

From `review-standards.md` §2:

> No catch-and-continue around reserve.
>
> DENY and sidecar outage both prevent upstream provider invocation.

From `review-standards.md` §5:

> IDs and idempotency keys delegate to `@spendguard/sdk`.
>
> No local hash library or crypto import.

## Files touched

| File | Why |
|---|---|
| `tests/*.test.ts` | Complete TP-D40B matrix. |
| `scripts/size-budget.sh` | Bundle gate if not already present. |
| `package.json` | Test/build/size scripts only if missing. |

## Test/verification plan

- TP-D40B-01..12.
- TA-D40B-01..02.

## Anti-scope

- No behavior changes except test-driven fixes.
- No demo or docs publish.
