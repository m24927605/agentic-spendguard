# D11 — Acceptance

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D11 ships when **every gate** in §1 is green AND the §2 invariants are unbreakable AND the §3 ship-checklist is fully checked. Per build-plan §3 "100% feasible" rule: every gate below is runnable in the repo's current state, no third-party action required.

## 1. Hard gates

### G1 — Build + import

```bash
cd sdk/python && pip install -e '.[litellm-guardrail]'
python -c "from spendguard.integrations.litellm_guardrail import SpendGuardGuardrail; print(SpendGuardGuardrail.__name__)"
```

Expected: prints `SpendGuardGuardrail`. No `ImportError`.

### G2 — Unit suite

```bash
cd sdk/python && pytest tests/integrations/test_litellm_guardrail.py -v
```

Expected: 18 tests pass (count from `tests.md` §2; final count may rise during implementation but never fall below 18).

### G3 — In-proc proxy integration suite

```bash
cd sdk/python && pytest tests/integrations/test_litellm_guardrail_proxy_inproc.py -v
```

Expected: 5 tests pass (4 from `tests.md` §3 + 1 co-registration `I05`).

### G4 — Existing LiteLLM callback path still passes (regression)

```bash
cd sdk/python && pytest tests/integrations/test_litellm.py -v
```

Expected: unchanged baseline (no test count change, no new failures). This gate proves D11 is purely additive.

### G5 — Demo mode boots and passes

```bash
make demo-down       # clean slate
make demo-up DEMO_MODE=litellm_guardrail
```

Expected:

- All compose services reach healthy.
- Demo driver exits 0.
- stdout contains `[demo] litellm_guardrail ALL 3 steps PASS (ALLOW + DENY + STREAM)`.
- stdout contains `D11_GUARDRAIL OK: litellm decisions=N` for `N ≥ 2`.
- stdout contains `D11_GUARDRAIL OK: canonical_events` from the outbox closure verifier.

### G6 — Demo mode tear-down clean

```bash
make demo-down
```

Expected: no orphaned containers, no orphaned volumes, exit 0.

### G7 — Public docs page renders

```bash
cd docs/site && npm run build
```

Expected: build succeeds. `docs/site/dist/docs/integrations/litellm-guardrail/index.html` exists. Page contains:

- "1-minute setup" section with the yaml snippet.
- Decision matrix comparing forked-callback / guardrail / egress-proxy paths.
- Link to `examples/litellm-proxy-composite/`.

### G8 — README index entry present

```bash
grep -F "litellm-guardrail" README.md
```

Expected: exactly one row in the adapter integrations table.

### G9 — PyPI extra wired

```bash
grep -F "litellm-guardrail" sdk/python/pyproject.toml
```

Expected: `litellm-guardrail = [...]` extra defined with `litellm[proxy]>=1.55.0`. Existing `litellm` extra unchanged.

### G10 — No proto / no schema / no Rust changes

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.(proto|sql|rs)$' | grep -v '^deploy/demo/verify_step_litellm_guardrail\.sql$'
```

Expected: empty output. Only the new demo verify SQL is in the .sql allow-list; nothing else proto/SQL/Rust touched.

## 2. Invariants (must never regress)

| ID | Invariant | How verified |
|----|-----------|--------------|
| INV-1 | **DENY never hits the upstream provider.** Counting stub MUST register zero hits across all `denied` decisions in the demo. | `verify_step_litellm_guardrail.sql` provider-counter assertion + I03 strict-order event |
| INV-2 | **Pre-call reservation precedes upstream HTTP.** RequestDecision fires before the LiteLLM-driven outbound request. | I02 `asyncio.Event` strict-ordering check |
| INV-3 | **Fail-closed default.** Sidecar DEGRADE → HTTP 503. Only `SPENDGUARD_LITELLM_FAIL_OPEN=1` permits otherwise. | U14 + U18 + I04 |
| INV-4 | **No double-charge during cutover.** Operator may register both legacy callback and new guardrail temporarily; sidecar dedup on `idempotency_key` prevents two reserves per `litellm_call_id`. | I05 |
| INV-5 | **End-of-stream commit uses real usage when present.** `response.usage.completion_tokens` is what lands in the commit row; estimator-fallback only fires when `usage` is None and logs a WARN. | U17 + demo step 3 |
| INV-6 | **No mutation of `data` on success.** Pre-call hook returns `data` unmodified — no `spendguard` key written, no header rewrite. | U12 (asserts identity) + I02 (proxy logs unchanged shape) |
| INV-7 | **Operator does not need to write Python for the single-tenant case.** When 8 env vars are set, no Python fork is needed. | G5 succeeds with no operator-written `.py` outside `deploy/demo/litellm_guardrail/spendguard_guardrail_bootstrap.py` |

## 3. Ship checklist

```
[ ] G1 build + import passes
[ ] G2 unit suite passes (≥18 tests)
[ ] G3 in-proc proxy integration passes (≥5 tests, incl. I05 co-registration)
[ ] G4 existing LiteLLM callback baseline unchanged
[ ] G5 `make demo-up DEMO_MODE=litellm_guardrail` exits 0 + success lines printed
[ ] G6 `make demo-down` clean
[ ] G7 docs site build succeeds + new page renders
[ ] G8 README adapter table updated
[ ] G9 pyproject.toml extra defined
[ ] G10 no proto / SQL / Rust drift outside the demo verify SQL
[ ] INV-1 .. INV-7 all green
[ ] All 7 slices merged in order S1 → S7 with R1-R5 review loop per build-plan §1.1
[ ] Memory write-back entry `project_coverage_D11_shipped.md` drafted per build-plan §8
```

## 4. Definition of done (per build-plan §7)

- All slices merged into main.
- Acceptance gates G1..G10 + invariants INV-1..INV-7 green.
- README adapter row landed.
- `docs/site/docs/integrations/litellm-guardrail.md` page live.
- `Makefile` demo-mode entry live.
- Memory entry written per build-plan §8.

## 5. Out-of-scope explicit declarations

D11 does NOT close any of:

- LiteLLM direct-SDK gap (Issue #8842) — D12.
- Per-team budgets on virtual keys beyond reading `team_id` — operator concern.
- Provider-side billing reconciliation beyond `response.usage` — separate workstream.
- Streaming token-by-token cap enforcement — end-of-stream only.

These are documented in `docs/site/docs/integrations/litellm-guardrail.md` "Limitations" section so the operator's expectation matches the ship surface.

