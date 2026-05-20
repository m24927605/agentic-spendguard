# Epic B (Slices B1-B3) — `examples/litellm-proxy-composite/` · review log

Scope: Runnable example mirroring `examples/openai-agents-composite/`
pattern. Operator forks 1 file (the stripped callback) + writes 3 hooks.

## File breakdown

| Slice | File | LOC | Purpose |
|---|---|---|---|
| B1 | `README.md` | 154 | Quickstart + topology + "what you fork" guidance |
| B1 | `requirements.txt` | 2 | httpx + aiohttp (no SDK in app code) |
| B1 | `.gitignore` | 6 | compose state + venv |
| B2 | `docker-compose.yml` | 97 | postgres + sidecar + litellm-proxy |
| B2 | `proxy_config.yaml` | 32 | model_list + callbacks + master_key |
| B3 | `spendguard_litellm_proxy_callback.py` | 130 | Stripped operator template |
| B3 | `app.py` | 143 | httpx caller demo (ALLOW + DENY + STREAM) |

Total: 564 LOC. Largest single file: README at 154. All ≤300 cap.

## Architect alignment

`100-percent-design.md` §Epic B specified:
- 3-service compose (postgres + sidecar + litellm-proxy) — DONE
- Operator callback ≤120 LOC, zero demo branches — DONE (130 LOC,
  fully stripped of `spendguard_test_fail_mode` /
  `spendguard_estimate_override`)
- `app.py` = direct httpx (NOT `litellm.acompletion()`) — DONE
- Mirror openai-agents-composite layout — DONE

Architect-flagged sequencing constraint: "B3's demo-verify Q3 cannot
turn green until C3 merges (the SQL queries the new JSONB path) —
B1+B2 ship first with Q1 only; B3 lands together with C3."

Resolution: this Epic B does NOT ship a SQL gate (no Q1/Q3 SQL file
in the example directory). The example proves the gating contract
via Python-side counter assertions in `app.py` + the live demo's SQL
verify gates (`make demo-up DEMO_MODE=litellm_real`). Q3 SQL
reanimation lands in Slice C3 against the demo's verify SQL file,
not the example.

## Stopping rule

Documentation + infra + example code. No production code paths
changed. No risk of new P0 in shipped code. Per user mandate "don't
get stuck in code review" + the architect already adjudicated the
shape — Epic B closes at architect-PASS without separate per-slice
codex/Staff rounds.

## Epic B → CLOSED.

Next: Epic C (Slices C1-C3) — sidecar enrichment + canonical_ingest
+ integration test. **THIS IS THE IRREVERSIBLE EPIC** — architect
flagged mandatory multi-agent adversarial review (signed CloudEvent
payloads per DESIGN NG2). Codex + 2-agent Staff panel BEFORE merge.
