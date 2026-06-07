# `spendguard.audit.import.devin_acu` — CloudEvent Schema (v1alpha1)

**Status:** D14 COV_70 deliverable. **Owner:** Backend Architect.
**Parent:** [`design.md`](design.md) §4.3.
**Companion code:** `services/importer_devin/src/cloudevent_envelope.rs` (builder), `services/importer_devin/tests/cloudevent_envelope_golden.rs` (byte-equal golden).

## 1. CloudEvents 1.0 envelope (top-level)

| Field | Type | Required? | Description |
|-------|------|-----------|-------------|
| `specversion` | `string` | yes | Always `"1.0"`. CloudEvents spec version. |
| `type` | `string` | yes | Always `"spendguard.audit.import.devin_acu"`. NO version suffix — `data.schema_version` carries the contract version. |
| `source` | `string` | yes | Always `"spendguard-importer-devin"` — matches the crate name. |
| `id` | `string` | yes | UUIDv5 derived deterministically from `(devin_team_id, devin_session_id, window_end)` so re-running the same window does not double-emit; canonical_ingest dedups via `event_replay_dedup`. |
| `time` | `string` | yes | RFC 3339 timestamp of envelope construction (NOT of the underlying billing event — that is `data.window_end`). |
| `datacontenttype` | `string` | yes | Always `"application/json"`. |
| `subject` | `string` | yes | `tenant/<tenant_id>/devin/team/<devin_team_id>/session/<devin_session_id>`. MUST NOT contain the bearer token, the customer email, or any inbound credential (review-standards T7). |
| `data` | `object` | yes | The payload — see §2. |

## 2. `data` payload (v1alpha1)

| Field | Type | Required? | Description |
|-------|------|-----------|-------------|
| `schema_version` | `string` | yes | Always `"v1alpha1"` for D14. Additive evolution lands as `v1alpha2`. |
| `tenant_id` | `string` | yes | SpendGuard tenant. |
| `budget_id` | `string` | yes | SpendGuard budget. |
| `devin_team_id` | `string` | yes | Devin Team API team identifier. Opaque. |
| `devin_session_id` | `string` | yes | Devin session identifier. Opaque. |
| `acu_consumed` | `number` | yes | Raw ACU value from the Devin Team API. |
| `usd_per_acu` | `number \| null` | yes | The rate looked up from the price table at conversion time. `null` for enterprise plans without a published rate. |
| `amount_micro_usd` | `integer \| null` | yes | `round(acu_consumed × usd_per_acu × 1_000_000)`. `null` when `usd_per_acu` is `null` (enterprise negotiated rate). Saturates at `i64::MAX` on overflow. |
| `pricing_version` | `string` | yes | Opaque version string stamped at the moment of conversion. Rate back-revision MUST bump this so historical rows are not retroactively rewritten. |
| `window_start` | `string` | yes | RFC 3339 timestamp — start of the billing window. |
| `window_end` | `string` | yes | RFC 3339 timestamp — end of the billing window. Same `window_end` ↦ same envelope `id`. |
| `reservation_source` | `string` | yes | Always `"subscription_meter"`. NEVER `"byok"` or any other value — this is what makes the row skip `ledger_entries` in canonical_ingest (D13 §4.3 fork). |
| `import_source` | `string` | yes | Always `"devin_team_api"`. Matches the value mig 0059 adds to the `audit_outbox.import_source` CHECK. |
| `ingestion_mode` | `string` | yes | One of `"fixture"` or `"live"`. Serializer rejects any other value. |
| `fixture_provenance_sha256` | `string \| null` | yes (nullable) | `Some(64-hex)` when `ingestion_mode == "fixture"` — the SHA-256 of the source fixture file. `None` when `ingestion_mode == "live"`. The mode-conditional invariant is enforced by `import_record_to_audit_row`. |

## 3. Schema evolution rules

* **Additive only.** New fields land as `data.<field>` with a default-zero / default-null
  contract. Downstream consumers default-zero unknown fields (project-wide additive-only
  convention).
* **Version bump on additive evolution.** Adding a new field bumps `data.schema_version` to
  `v1alpha2`. The CloudEvent `type` does NOT version-suffix — the version lives only in
  `data.schema_version`.
* **No breaking changes.** Removing a field, renaming a field, or narrowing a type
  is a Blocker. The next compatibility break would ship a fresh `type` namespace
  (e.g. `spendguard.audit.import.devin_acu_v2`).
* **Constants are pinned.** `type`, `source`, `data.reservation_source`,
  `data.import_source` are frozen forever — they are the routing keys downstream
  (canonical_ingest dispatch + audit_outbox CHECK constraint + dashboard filters).

## 4. Golden fixtures

The byte-equal golden test
`services/importer_devin/tests/cloudevent_envelope_golden.rs`
pins one envelope per `(plan, ingestion_mode)` combination:

* `team` × `fixture` — the headline success path.
* `enterprise` × `fixture` — the nullable `amount_micro_usd` + `usd_per_acu` path.
* `team` × `live` — confirms `fixture_provenance_sha256 = null` in live mode.

Any envelope change requires all three updated in the same PR
(review-standards S5).

## 5. Sample payload

```json
{
  "specversion": "1.0",
  "type": "spendguard.audit.import.devin_acu",
  "source": "spendguard-importer-devin",
  "id": "018f4a3a-d971-7c14-91fe-d014de71aca0",
  "time": "2026-06-08T12:00:00Z",
  "datacontenttype": "application/json",
  "subject": "tenant/demo/devin/team/TEAM_FIXTURE_001/session/SESSION_FIXTURE_001",
  "data": {
    "schema_version": "v1alpha1",
    "tenant_id": "demo",
    "budget_id": "devin-budget",
    "devin_team_id": "TEAM_FIXTURE_001",
    "devin_session_id": "SESSION_FIXTURE_001",
    "acu_consumed": 12.5,
    "usd_per_acu": 2.25,
    "amount_micro_usd": 28125000,
    "pricing_version": "devin-acu-v1-2026-06",
    "window_start": "2026-06-01T00:00:00Z",
    "window_end": "2026-06-01T01:00:00Z",
    "reservation_source": "subscription_meter",
    "import_source": "devin_team_api",
    "ingestion_mode": "fixture",
    "fixture_provenance_sha256": "aa4c172164a8a6a5d4e97c6bde4ac455e01f5f37932b8a3561ef213049144807"
  }
}
```
