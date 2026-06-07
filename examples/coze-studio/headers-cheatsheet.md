# Coze Studio + SpendGuard — header cheatsheet

The SpendGuard sidecar HTTP companion (`D09 SLICE 1`) reads three HTTP headers
to resolve every Coze Studio model call into the right tenant + budget +
window-instance. This page explains what each header is, where the value comes
from in the Coze workspace UI, and how the companion behaves when one is
missing or malformed.

> **TL;DR.** Paste three rows into Coze's "Model Provider → OpenAI → Custom
> Headers" form. All three are required. Missing tenant → companion returns
> `400 MISSING_TENANT` (INV-4 of the D31 spec).

## The three headers

| Header | Required | Format | Source |
|--------|----------|--------|--------|
| `X-SpendGuard-Tenant-Id` | yes | Free-form UTF-8 (≤ 64 chars). Recommended: the Coze workspace ID. | Coze: **Workspace → Settings → General → Workspace ID**. |
| `X-SpendGuard-Budget-Id` | yes | UUID v4 (canonical 8-4-4-4-12 lowercase). | SpendGuard control-plane: `GET /api/v1/budgets` row's `budget_id`. |
| `X-SpendGuard-Window-Instance-Id` | yes | UUID v4 (canonical 8-4-4-4-12 lowercase). | SpendGuard control-plane: `GET /api/v1/budgets/{id}/window-instances` row's `window_instance_id`. |

### `X-SpendGuard-Tenant-Id`

The companion treats this as the audit-chain `tenant_id`. The value flows into
`audit_outbox.tenant_id` and `ledger_transactions.tenant_id`, and is the unit
the SpendGuard control plane partitions on for `GET /api/v1/audit?tenant_id=...`
queries.

**Format**

- UTF-8 string. The companion does NOT validate UUID shape on this header —
  free-form is intentional so operators can use the Coze workspace ID directly
  (Coze workspace IDs are ULID-like, not UUIDs).
- Length cap: 64 bytes. The companion rejects with `400 TENANT_TOO_LONG` if
  longer.
- Empty / missing → `400 MISSING_TENANT`. **There is no "default tenant" silent
  fallback** (INV-4 of the D31 spec).

**Where to find it in Coze**

1. Sign in to Coze Studio.
2. **Workspace → Settings → General**.
3. Copy the "Workspace ID" field. Looks like `7234567890123456789` or a
   ULID-formatted string depending on the Coze version.

**Multiple workspaces.** If one Coze install hosts multiple workspaces and you
want a separate SpendGuard budget for each, paste a different workspace ID
into each workspace's provider config. The same SpendGuard sidecar serves all
of them (companion is multi-tenant).

### `X-SpendGuard-Budget-Id`

The companion uses this to look up which `BudgetBinding` to apply on the
reserve. The value flows into `ledger_transactions.budget_id` and into the
`RequestDecision` envelope as `budget_id`.

**Format**

- Canonical UUID v4: 8-4-4-4-12 lowercase hex with hyphens.
  Example: `44444444-4444-4444-8444-444444444444`.
- Anything else → `400 INVALID_BUDGET_ID`.

**Where to find it**

- SpendGuard control plane: `GET /api/v1/budgets` returns
  `[{"budget_id": "...", "name": "...", ...}]`. Copy the UUID of the budget
  you want this workspace to draw from.
- For the demo (`DEMO_MODE=coze_studio_real`): the seed SQL writes
  `44444444-4444-4444-8444-444444444444`.

### `X-SpendGuard-Window-Instance-Id`

A SpendGuard "window instance" is a time-bounded budget window (e.g. "the
2026-06 monthly window of budget `44444…`"). The companion uses this to
resolve which window to charge against on the reserve.

**Format**

- Canonical UUID v4 (same shape as `X-SpendGuard-Budget-Id`).
- Anything else → `400 INVALID_WINDOW_INSTANCE_ID`.

**Where to find it**

- SpendGuard control plane:
  `GET /api/v1/budgets/{budget_id}/window-instances?state=open` returns the
  open window instance for the budget. Copy its `window_instance_id`.
- The control plane rolls window instances over automatically (daily / monthly
  per budget config); the snippet does NOT need to be edited at rollover.
  However, **if you copy a closed window's UUID, the companion returns
  `409 WINDOW_INSTANCE_CLOSED`** at reserve time.
- For the demo: the seed SQL writes
  `55555555-5555-5555-8555-555555555555`.

## Companion error responses

| Header missing / malformed | HTTP | Body shape |
|----------------------------|------|------------|
| `X-SpendGuard-Tenant-Id` missing | 400 | `{"error":{"code":"MISSING_TENANT","message":"X-SpendGuard-Tenant-Id required"}}` |
| `X-SpendGuard-Tenant-Id` > 64 bytes | 400 | `{"error":{"code":"TENANT_TOO_LONG","message":"X-SpendGuard-Tenant-Id ≤ 64 bytes"}}` |
| `X-SpendGuard-Budget-Id` missing | 400 | `{"error":{"code":"MISSING_BUDGET","message":"X-SpendGuard-Budget-Id required"}}` |
| `X-SpendGuard-Budget-Id` not UUID | 400 | `{"error":{"code":"INVALID_BUDGET_ID","message":"X-SpendGuard-Budget-Id must be UUID v4"}}` |
| `X-SpendGuard-Window-Instance-Id` missing | 400 | `{"error":{"code":"MISSING_WINDOW_INSTANCE","message":"X-SpendGuard-Window-Instance-Id required"}}` |
| `X-SpendGuard-Window-Instance-Id` not UUID | 400 | `{"error":{"code":"INVALID_WINDOW_INSTANCE_ID","message":"X-SpendGuard-Window-Instance-Id must be UUID v4"}}` |
| `X-SpendGuard-Window-Instance-Id` closed | 409 | `{"error":{"code":"WINDOW_INSTANCE_CLOSED","message":"window instance is no longer open for reservations"}}` |

Coze surfaces all of these as a workflow error in its own UI; the message body
appears in the chat-flow execution trace.

## What about the OpenAI key?

The Coze workspace YAML wires `api_key: ${OPENAI_API_KEY}` — the literal
string `${OPENAI_API_KEY}`, with `$` and `{}`. Coze expands the env-var at
request time, then passes the resulting bearer to the sidecar companion via
the standard `Authorization: Bearer ...` header. The companion forwards it
unchanged to upstream OpenAI.

**Never hardcode a literal `sk-...` key in the snippet, the README, or this
cheatsheet.** D31 INV-6 + acceptance gate G1 will fail the slice if a literal
key shows up anywhere under `examples/coze-studio/`.

## See also

- `examples/coze-studio/coze-workspace-config.yaml` — the snippet operators
  paste.
- `examples/coze-studio/README.md` — full operator walkthrough including
  cert bootstrap.
- `docs/site/docs/integrations/coze-studio.md` — public docs page with
  decision matrix (D31 vs D02/D03 egress proxy).
- `docs/specs/coverage/D31_coze_studio/design.md` §3.3 — design lock for the
  header contract.
