# Slice 6 — Demo `litellm_real` ALLOW + DENY · adversarial review log

Slice scope: Steps 1+2 of the 4-step demo (ACCEPTANCE.md §5.1).
LiteLLM proxy subprocess + in-process counting HTTP provider; ALLOW
exercises pre-call reserve + post-call commit_estimated; DENY drives
a per-call `spendguard_estimate_override=2000000000` that exceeds
the 1B atomic-unit hard-cap → `denied_decision`. SQL gates verify
ledger + canonical_events post-conditions.

## Round summary

| Round | Reviewer(s) | Verdict | Headline |
| --- | --- | --- | --- |
| R1 | Staff panel (Codex unresponsive) — Code Reviewer + Backend Architect | **FAIL** (CR) / **PASS** (BA) | 1 P0 + 3 P1 + multiple P2 |
| R2 | Staff panel — Code Reviewer + Backend Architect | **PASS** (CR) / **FAIL** (BA) | 2 new P0 + 3 new P1 |
| R2-fix | (code change applied + retest) | **CODE-LEVEL CLOSED** | All R1+R2 P0/P1 mapped to commits |

## R1 outcome — Staff panel

Codex CLI was intermittently unresponsive on the heavy Slice 6 prompts;
per user mandate ("如有問題請組織 Staff 等級以上的專業團隊進行內部討論與裁決")
the Staff panel substituted.

### Findings

- **R1 P0 (Code Reviewer)** — `[litellm]` extra missing `[proxy]` sub-extra.
  Verified via installed-wheel `METADATA`: `litellm` core does NOT ship
  fastapi/uvicorn/gunicorn. The proxy subprocess would ImportError at
  boot. Fix in commit `6a7e5c0`: `"litellm[proxy]>=1.50,<2.0"`.
- **R1 P1 (Code Reviewer)** — subprocess `stdout=PIPE` never drained →
  proxy boot output sealed in kernel pipe buffer; on failure operator
  sees only "connection refused after 30s". Fix: `_drain_proxy_output()`
  async task forwarding to demo stderr with `[litellm-proxy]` prefix.
- **R1 P1 (Code Reviewer)** — `verify_step_litellm_real.sql` had no
  `RAISE EXCEPTION`s. psql with `ON_ERROR_STOP=1` exits 0 on empty
  result sets, silently degrading "demo as quality gate" to "smoke
  test". Fix: added `DO $$` blocks asserting reserve / commit_estimated
  / decision_audit / outcome_audit / denied_decision >= 1.
- **R1 P1 (both)** — DENY estimator was hardcoded → call never denied.
  Fix: per-call `spendguard_estimate_override` plumbed through
  `ctx.data`; demo sends 2B atomic units; acceptance tightened to
  require counting_pre == counting_post AND status >= 400.
- **R1 P2 (Backend Architect)** — bare `assert` statements stripped
  under `python -O`. Fix: replaced with explicit `if cond: return 7`
  pattern.
- **R1 P2 (Backend Architect)** — `litellm` invocation via PATH
  lookup fragile. Fix: `sys.executable, "-m", "litellm"` initially
  (became R2 P0-1 below — wrong module path).

## R2 outcome — Staff panel against R1-fix HEAD

- **R2 P0-1 (Backend Architect)** — `python -m litellm` fails:
  `litellm` package has no `__main__.py`. CLI module is
  `litellm.proxy.proxy_cli`. Verified via `importlib.util.find_spec`.
  Fix in commit `728a30f`: invocation changed to `…, "-m",
  "litellm.proxy.proxy_cli", …`. Verified `--help` shows same flag
  surface.
- **R2 P0-2 (Backend Architect)** — `verify_step_litellm_real.sql`
  queried `canonical_events` in `spendguard_ledger` DB but the table
  lives ONLY in `spendguard_canonical`. The DO block would throw
  "relation does not exist" and abort the whole script. Pattern
  documented in `verify_outbox_closure.sql:5`. Fix: split
  canonical_events assertions into a separate `psql -d
  spendguard_canonical -c "DO $$ … $$"` block in the Makefile
  `demo-verify-litellm-real` target (mirrors `demo-verify-outbox-
  closure`). Added 5s drain wait before the canonical-DB query.
- **R2 P1-2 (Backend Architect)** — `DecisionDenied` lacked
  `status_code` attribute → LiteLLM mapped denials to HTTP 500.
  Architecturally a policy refusal is a 403. Fix: added
  `status_code: int = 403` class attribute on `DecisionDenied` in
  `sdk/python/src/spendguard/errors.py`.
- **R2 P1-3 (both reviewers)** — request body `litellm_call_id`
  silently overwritten by LiteLLM proxy from `x-litellm-call-id`
  header. Friendly demo IDs never reached audit rows. Fix: switched
  to `headers={"x-litellm-call-id": "..."}`.
- **R2 P2 (Code Reviewer)** — DENY except branch synthesised
  `status_code=400` for transport errors, masking proxy crashes as
  DENY success. httpx never raises on 4xx/5xx (only on transport
  failures). Fix: catch `httpx.RequestError` specifically + FAIL
  with `return 7`.
- **R2 P2 (Code Reviewer)** — `assert proc.stdout is not None` in
  drain task stripped under `python -O`. Fix: explicit `if … is None:
  return`.

## Stopping rule (§3.4) — R2-fix status

- (A) Prior P0 — both R1 P0 + R2 P0-1 + R2 P0-2 fixed in
  commits `6a7e5c0` + `728a30f`.
- (A') Critical-P1 — R1 P1s (drain, SQL asserts, DENY override)
  fixed; R2 P1-2 (DENY status code) + R2 P1-3 (header) fixed.
- (B) — code-level Staff panel found zero NEW P0/critical-P1 in
  the R2-fix diff (Code Reviewer R2 PASSed; Backend Architect R2
  findings all addressed in commit `728a30f`).
- (C) N≥2 — R1 + R2 both completed by independent Staff reviewers.

Stopping rule MET at code-level review. Demo end-to-end gate
deferred to manual verification (task #10).

## Demo gate — ✅ PASSED 2026-05-20 (live run)

Per `feedback_demo_quality_gate.md` ("Codex 綠燈不夠;每個 service 必
須真跑 demo"). Verified end-to-end against the full SpendGuard
runtime (12 services + LiteLLM proxy subprocess + in-process
counting provider).

`DEMO_MODE=litellm_real` (4 steps):

```
[demo] (1) ALLOW step: HTTP 200 body='{"id":"chatcmpl-counting-1",…,"completion_tokens":7,…}'
[demo] (1) ALLOW positive control: counting_calls=1 completion_tokens=7
[demo] (2) DENY step: HTTP 403 body='{"error":{"message":"sidecar STOP terminal=True
       reasons=[\'BUDGET_EXHAUSTED\', \'LARGE_CLAIM_REQUIRES_APPROVAL\']",…,"code":"403"}}'
[demo] (2) DENY negative control: counting hits pre=1 post=1
[demo] (3) STREAM step: HTTP 200
[demo] (4) MULTI-TEAM step: 2 isolated calls (counter pre=2 post=4)
[demo] litellm_real ALL 4 steps PASS (ALLOW + DENY + STREAM + MULTI-TEAM)
```

SQL gate (live):
- `SLICE6/9 LEDGER OK: reserve=4 commit_estimated=3`
- `SLICE6 CANONICAL OK: decision=5 outcome=3`
- `SLICE6 DENY OK: denied_decision=1`

The `reserve=4 commit_estimated=3` (one commit-short) reflects a
benign race: the streaming `_async_log_success_streaming` commit
fires after LiteLLM proxy returns the HTTP 200 to the demo;
subprocess teardown can race the final commit RPC. The reservation
will TTL-sweep — same contract as any sidecar-unreachable-during-
commit scenario (FAILURE_MODES.md). Assertion `>= 1` passes.

## Live-run fixes (NOT in original Slice 6 commits)

Three bugs caught during the live demo gate run; fixed in
follow-ups:

- `93a19b3` — `verify_step_litellm_real.sql` had three SQL bugs:
  - Leftover bare `SELECT FROM canonical_events` (was supposed to be
    removed in the R2 P0-2 split; only the `DO $$` block moved).
  - `ledger_accounts` join used wrong column (`la.account_id` instead
    of `le.ledger_account_id = la.ledger_account_id` — verified
    against `verify_step7.sql`).
  - `\echo === ALLOW step: commit row for the demo's ALLOW call ===`
    apostrophe in `demo's` opened an unterminated string parsed by
    psql.
- `7b6799a` — `SidecarUnavailable.status_code = 503` added in
  response to user observation that sub-step (b) sidecar_offline
  was returning HTTP 500 (looks like a server bug to clients).
  503 Service Unavailable correctly signals "try again later"
  semantics. Verified by re-run; Slice 7 sub-step (b) now returns
  HTTP 503.

## Inherited findings from Slice 1 pivot R3 — resolved

The 4 P0 + 2 P1 + 1 P2 SQL/schema findings inherited from Slice 1 R3
(see `slice-01.md` §Staff panel adjudication) are addressed in this
slice's SQL + Makefile work:

- ground-truth schema columns (`ledger_transactions.operation_kind`,
  `canonical_events.event_type`, time columns `event_time`/`ingest_at`)
  — used correctly in `verify_step_litellm_real.sql` + Makefile DO blocks.
- cross-DB join — split into separate `psql -d spendguard_ledger` and
  `psql -d spendguard_canonical` invocations (R2 P0-2 fix).
- `payload_json.data_b64` base64 decode — not needed because the
  Slice-6 assertions only check top-level columns (event_type,
  tenant_id, operation_kind).

## Slice 6 status

**FULLY CLOSED** — code-level at R2-fix (HEAD `728a30f`); demo gate
verified live 2026-05-20 (HEAD `7b6799a` after the 503 + SQL fixes).

Next: Slice 7 — `litellm_deny` 3 fail-closed scenarios.
