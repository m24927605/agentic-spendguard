# D13 — Subscription-Tier Meter Mode (Claude Code Pro + Codex ChatGPT-OAuth)

**Status:** Spec — Tier 3, build plan §2.3. **Parent:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) Archetype II. **Depends on:** [`D02`](../D02_closed_cli_install/design.md) (CA + `HTTPS_PROXY`). **Owner:** Backend Architect.

## 1. Problem

Claude Code Pro/Max and Codex on ChatGPT Plus/Pro use OAuth subscriptions. Traffic still hits `api.anthropic.com/v1/messages` and `chatgpt.com/backend-api/codex/responses`, so D02's proxy sees the calls — but the vendor settles quota internally and SpendGuard never sees the dollar. Today `routing.rs` routes BYOK only; a Pro request lands on `/v1/messages` and is charged to the BYOK ledger as if paid per-token, inventing a phantom dollar over the $20/mo flat fee.

## 2. Goals / non-goals

**In:** detect subscription vs BYOK at the proxy edge; meter via tokenize + retail price + audit row tagged `reservation_source = subscription_meter`; never charge BYOK ledger; three modes — `meter` (default audit-only), `soft_cap` (alert, never block), `hard_cap` (synthetic 429); spec `spendguard-importer-{anthropic,openai}` for Day-2 reconciliation.

**Out:** live enforcement of vendor quota (vendor owns it); shipping the importers (D13 = contract only); Cursor/Windsurf protocols (D17/D18); Gemini OAuth free tier (legal red line per strategy §"Archetype V").

## 4. Architecture

```
inbound → routing::route()  (existing — picks ProviderConfig)
       → subscription::classify()  { Byok | ClaudeCodePro | CodexChatGpt | Unknown }
            Byok  → decision::estimate_call_cost → sidecar::RequestDecision
                                                 → reservation + ledger + audit
            subscription kinds  → subscription_meter::meter_only_estimate
                                  (NO sidecar reserve, NO ledger write)
                                → evaluate_cap → { Pass | SoftCapAlert | HardCapBlock(429) }
                                → audit_outbox row, reservation_source=subscription_meter
```

Classification runs after `route()` and before `estimate_call_cost`, additive fork.

### 4.1 Classification

| Signal | Claude Code Pro | Codex / ChatGPT-OAuth |
|--------|------------------|------------------------|
| Host + path | `api.anthropic.com/v1/messages` | `chatgpt.com/backend-api/codex/responses` (new row) |
| `Authorization` prefix | `Bearer sk-ant-oat01-…` | `Bearer eyJ…` (JWT, OpenAI issuer) |
| `User-Agent` | `claude-cli/<ver>` | `codex_cli_rs/<ver>` |
| BYOK distinguisher | `sk-ant-api03-…` | `sk-proj-…` / `sk-…` |

Classification is `Byok` unless **both** key prefix AND User-Agent match. We don't parse JWTs (no crate dep, no side-channel); we sniff the `eyJ` base64url header prefix.

### 4.2 Routing addition

`routing.rs` appends one row: `^/backend-api/codex/responses$` → `chatgpt.com/backend-api/codex/responses`, `OpenAiResponses` shape, `OpenAi` encoder. The Claude Code path reuses `/v1/messages`; classification, not routing, picks BYOK vs subscription.

### 4.3 Meter-only audit row

`common.proto` gains `ReservationSource` enum (`UNSPECIFIED=0 / BYOK=1 / SUBSCRIPTION_METER=2`). `audit_outbox` gains `reservation_source TEXT NOT NULL DEFAULT 'byok'` (mig 0044), partial index on subscription rows. Sidecar **must** skip `ledger_entries` + `reservations` writes when the field is `SUBSCRIPTION_METER`. ASP wire format carries the field for importer correlation.

### 4.4 Modes

`SPENDGUARD_SUBSCRIPTION_MODE = meter | soft_cap | hard_cap` (default `meter`); per-tenant override via `subscription_caps` (mig 0045): `(tenant_id, budget_id, mode, threshold_usd, threshold_window)` with RLS isolation. Default window = UTC calendar month.

### 4.5 Hard-cap synthetic 429

When mode is `hard_cap` and running meter ≥ threshold, proxy short-circuits before forwarding and returns HTTP 429 with `Retry-After: <seconds until window reset, max 86400>` and body `{"error":{"type":"rate_limit_exceeded","message":"spendguard subscription cap reached","code":"spendguard_subscription_cap"}}`. Response shape is vendor-matched (Anthropic uses `error.type`, OpenAI uses `error.code`) so CLIs treat it identically to a vendor rate-limit and exit. Distinct `code = spendguard_subscription_cap` distinguishes SpendGuard-injected from vendor-injected 429s. Audit row: `decision = STOP_RUN_PROJECTION`, `reason_code = subscription_cap_exceeded`.

## 5. Importer integration point

Stub crates `services/importer_anthropic/` (`import_source = anthropic_console_usage`) and `services/importer_openai/` (`import_source = openai_admin_usage`) ship empty with locked contract. New `audit_outbox.import_source TEXT NULL` column (mig 0046, CHECK-constrained); reconciler joins meter + importer rows on `(tenant_id, window_bucket)`. D13 ships schema + skeletons + contract tests only; `live` feature flag pulls no HTTP deps in default build.

## 6. Fixtures

Recorded HARs at `services/egress_proxy/tests/fixtures/subscription/`: `claude_code_pro_session`, `codex_chatgpt_plus_session`, `byok_anthropic`, `byok_openai`, `ambiguous_cli_byok`. Tokens/JWTs replaced with `FAKE_*` sentinels; `PROVENANCE.md` pins capture date, redaction script SHA-256, and no-PII assertion.

## 7. Slices

7 slices: `COV_60` classifier (S); `COV_61` mig 0044 + proto + sidecar branch (M); `COV_62` Codex routing + `meter_only_estimate` (M); `COV_63` mig 0045 + alerts (M); `COV_64` hard-cap 429 (M); `COV_65` mig 0046 + importer stubs (S); `COV_66` demos + docs (M). Skeleton in [`implementation.md`](implementation.md) §4.

## 8. Locked decisions

1. `reservation_source` is additive, default `byok` (ASP wire compat).
2. Classification requires BOTH header AND key/JWT prefix — UA alone forgeable.
3. Hard-cap returns 429 (not 402: subscriptions never bill us; not 503: CLIs interpret as transient).
4. Threshold window default = UTC calendar month (matches Anthropic billing).
5. No `ledger_entries` write for subscription meter (reserving BYOK ledger double-counts the flat fee).
6. R5 panel summarizer: Security Engineer (Authorization parsing + hard-cap DoS surfaces dominate over architecture framing).
7. Importer crates ship empty — `live` feature gated off, default build pulls no HTTP deps.
8. Codex upstream treated as `OpenAiResponses` shape with documented divergence — diff is in extensions the meter never reads.
9. Authorization token prefix capped at 13 chars — constant-length extraction, never logged beyond prefix.
10. Synthetic 429 `Retry-After` bounded at 86400s — prevents misconfigured windows asking CLIs to wait > 24h.
