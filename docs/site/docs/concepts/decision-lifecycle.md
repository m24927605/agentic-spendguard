# Decision lifecycle

Each LLM / tool call boundary triggers an 8-stage decision transaction:

| # | Stage | Where | Output |
|---|---|---|---|
| 1 | snapshot | sidecar (in-process) | snapshot_hash |
| 2 | evaluate | sidecar Contract DSL | matched_rules_hash |
| 3 | prepare_effect | sidecar (pure) | effect_hash |
| 4 | reserve | ledger atomic | reservation_id |
| 5 | audit_decision | folded into reserve | audit_outbox row |
| 6 | publish_effect | adapter (in-process) | mutation applied |
| 7 | commit_or_release | ledger | commit_estimated / release |
| 8 | audit_outcome | folded into commit/release | audit_outbox row |

The hard invariant: stages 1–5 happen atomically (single Postgres
transaction); a sidecar crash mid-publish replays stage 6 via
`effect_hash` idempotency.

See `docs/contract-dsl-spec-v1alpha1.md` §6 for the formal spec.
