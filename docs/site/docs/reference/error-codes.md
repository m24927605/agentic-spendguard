# Error codes

| Code | Meaning | Sidecar action |
|---|---|---|
| `FENCING_EPOCH_STALE` | Sidecar's lease expired | Fail-closed; re-acquire (GA gate) |
| `RESERVATION_STATE_CONFLICT` | Reservation not in expected state | Adapter SHOULD re-query reservation context |
| `RESERVATION_TTL_EXPIRED` | Reservation TTL passed before commit | TTL sweeper releases automatically |
| `PRICING_FREEZE_MISMATCH` | Bundle pricing differs from claim's pricing | Operator: re-issue bundle |
| `OVERRUN_RESERVATION` | Estimated > original reserved | Adapter MUST split into separate calls |
| `MULTI_RESERVATION_COMMIT_DEFERRED` | Multi-claim ReserveSet not in POC | Use single-claim |
| `IDEMPOTENCY_CONFLICT` | Idempotency key reused with different body | Surface to caller as hard error |
| `AUDIT_INVARIANT_VIOLATED` | SP found audit chain inconsistency | Page on-call; not retriable |

See `services/sidecar/src/domain/error.rs` and
`proto/spendguard/common/v1/common.proto` `Error.Code` enum for the
authoritative list.
