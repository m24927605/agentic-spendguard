# Wire protocol (proto)

Sidecar ↔ ledger ↔ canonical ingest gRPC contracts live under
[proto/spendguard/](https://github.com/m24927605/agentic-flow-cost-evaluation/tree/main/proto/spendguard):

- `common/v1/common.proto` — shared types (BudgetClaim, CloudEvent,
  Idempotency, Fencing, Replay, Error, etc.)
- `ledger/v1/ledger.proto` — ledger SP RPCs (ReserveSet,
  CommitEstimated, ProviderReport, InvoiceReconcile, Release,
  RecordDeniedDecision, …)
- `canonical_ingest/v1/canonical.proto` — AppendEvents (audit chain)
- `sidecar_adapter/v1/adapter.proto` — adapter UDS surface
  (RequestDecision, EmitTraceEvent, ConfirmPublishOutcome, etc.)

SDK proto stubs are auto-generated via `make proto` in
`sdk/python/`.
