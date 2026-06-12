# COV_D40B_03 - OpenClaw commit, failure, and streaming paths

> **Deliverable:** D40b OpenClaw provider plugin adapter
> **Slice:** 3 of 6
> **Spec set:** [`docs/specs/coverage/D40b_openclaw_provider_plugin/`](../specs/coverage/D40b_openclaw_provider_plugin/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Settle successful, failed, timed-out, aborted, and streaming calls. Commit must reuse reserve-time unit/pricing tuple and issue exactly one terminal settlement per reservation.

## LOCKED design quotes

From `design.md` §5:

> success: `client.commitEstimated(outcome="SUCCESS")`
>
> provider error: `client.commitEstimated(outcome="PROVIDER_ERROR")`, rethrow
>
> abort/timeout: `client.commitEstimated(outcome="RUN_ABORTED" or "CLIENT_TIMEOUT")`

From `review-standards.md` §3:

> Reserve claims carry those values.
>
> Commit reuses reserve-time unit/pricing tuple.

## Files touched

| File | Why |
|---|---|
| `src/provider.ts` | Commit/failure/streaming settlement. |
| `src/usage.ts` | Usage extraction. |
| `tests/provider.test.ts` | Success/failure tests. |
| `tests/streaming.test.ts` | Stream terminal commit tests. |

## VERIFY-AT-IMPL pins

Pin `OB-V4` and `OB-V5`.

## Test/verification plan

- TP-D40B-07..11.

## Anti-scope

- No demo overlay.
- No docs publish.
- No local hash implementation.
