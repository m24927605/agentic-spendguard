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

## Demo gate — DEFERRED to manual operator verification

Per `feedback_demo_quality_gate.md` ("Codex 綠燈不夠;每個 service 必
須真跑 demo"), the slice is NOT shippable to PR1 until `make demo-up
DEMO_MODE=litellm_real` produces exit 0 with:

1. `[demo] (1) ALLOW positive control: counting_calls=N+1 completion_tokens=7`
2. `[demo] (2) DENY step: HTTP 403 …`
3. `[demo] (2) DENY negative control: counting hits pre=N post=N` (no change)
4. `SLICE6 LEDGER OK: reserve=1 commit_estimated=1`
5. `SLICE6 CANONICAL OK: decision>=1 outcome>=1`
6. `SLICE6 DENY OK: denied_decision>=1`

The autonomous session attempted `make demo-up DEMO_MODE=litellm_real`
but compose-up of 12 services + image build exceeded the agent's
practical timeout. This is tracked as task #10 "Demo gate — 2 modes
real-run" (BLOCKING for PR1 merge).

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

**CODE-LEVEL CLOSED at R2-fix** (HEAD `728a30f`).
**DEMO GATE PENDING** — task #10 must produce a passing `make demo-up
DEMO_MODE=litellm_real` run before Slice 6 ships to PR1.

Next: Slice 7 — `litellm_deny` 3 fail-closed scenarios.
