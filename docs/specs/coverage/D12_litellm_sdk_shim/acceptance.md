# D12 — Acceptance

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D12 ships when **every gate** in §1 is green AND the §2 invariants are unbreakable AND the §3 ship-checklist is fully checked. Per build-plan §3 "100% feasible" rule: every gate is runnable in the repo's current state, no third-party action required.

## 1. Hard gates

### G1 — Build + import

```bash
cd sdk/python && pip install -e '.[litellm-shim]'
python -c "from spendguard.integrations.litellm_shim import install, uninstall, is_installed; print('ok')"
```

Expected: prints `ok`. No `ImportError`.

### G2 — Unit suite (mock litellm)

```bash
cd sdk/python && pytest tests/integrations/test_litellm_shim.py -v
```

Expected: 26 tests pass (count from `tests.md` §2; final count may rise during implementation but never fall below 22).

### G3 — Integration suite (real litellm + pytest-httpx)

```bash
cd sdk/python && pip install pytest-httpx
pytest tests/integrations/test_litellm_shim_real.py -v
```

Expected: 6 tests pass. NO outbound HTTP traffic leaves the test process (all calls intercepted by `pytest-httpx`).

### G4 — Transitive coverage smoke

```bash
cd sdk/python && pip install crewai dspy
pytest tests/integrations/test_crewai_via_shim.py -v
```

Expected: 3 tests pass when CrewAI + DSPy are installed. T01 + T02 + T03. Tests SKIP gracefully when the framework deps are absent.

### G5 — Existing LiteLLM tests still pass (regression)

```bash
cd sdk/python && pytest tests/integrations/test_litellm.py -v
```

Expected: unchanged baseline. No test count change, no new failures. The 5-LOC `SpendGuardDirectAcompletion._original_acompletion` patch is purely additive — when the kwarg is absent, today's behavior holds bit-for-bit.

### G6 — D11 guardrail tests still pass (regression)

```bash
cd sdk/python && pytest tests/integrations/test_litellm_guardrail.py tests/integrations/test_litellm_guardrail_proxy_inproc.py -v
```

Expected: D11 baseline unchanged. D12 must not touch `litellm_guardrail.py` at all.

### G7 — Demo mode `litellm_sdk_real` passes

```bash
make demo-down
make demo-up DEMO_MODE=litellm_sdk_real
```

Expected:

- All compose services reach healthy.
- Demo driver exits 0.
- stdout contains `[demo] litellm_sdk_real ALL 3 steps PASS`.
- stdout contains `D12_SDK OK: ...` from SQL verification.
- stdout contains `D12_SDK OK: canonical_events` from the outbox closure verifier.
- The TRANSITIVE step (CrewAI inline) shows ≥1 SpendGuard reserve was triggered by a `litellm.acompletion` call inside the CrewAI agent loop.

### G8 — Demo mode `litellm_sdk_deny` passes

```bash
make demo-down
make demo-up DEMO_MODE=litellm_sdk_deny
```

Expected:

- Exit 0.
- stdout contains `[demo] litellm_sdk_deny ALL 3 substeps PASS`.
- counting-provider stub registered ZERO hits during the 2 DENY sub-steps.
- `denied_decision` row present in `ledger_transactions`.

### G9 — Demo tear-down clean

```bash
make demo-down
```

Expected: no orphaned containers, no orphaned volumes, exit 0.

### G10 — Public docs page renders

```bash
cd docs/site && npm run build
```

Expected: build succeeds. `docs/site/dist/docs/integrations/litellm-sdk-shim/index.html` exists. Page contains:

- "1-minute setup" code snippet with `install(...)` call.
- Decision matrix with 3 rows: egress proxy (D02) / guardrail (D11) / SDK shim (D12). Each row has a "when to use" cell.
- "Limitations" section explicitly listing the 3 non-goals from design.md §3 (streaming token-by-token / sync in async / embedding endpoints).
- Cross-link to D11 docs page.

### G11 — README index entry present

```bash
grep -F "litellm-shim" README.md
```

Expected: exactly one row in the adapter integrations table for `LiteLLM SDK shim`.

### G12 — PyPI extra wired

```bash
grep -F "litellm-shim" sdk/python/pyproject.toml
```

Expected: `litellm-shim = [...]` extra defined with `litellm>=1.50` + `pytest-httpx>=0.30`. Existing `litellm` extra unchanged. Existing `litellm-guardrail` extra unchanged.

### G13 — No proto / no schema / no Rust changes

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.(proto|sql|rs)$' | grep -v '^deploy/demo/verify_step_litellm_sdk\.sql$'
```

Expected: empty output. Only the new demo verify SQL is in the .sql allow-list; nothing else proto/SQL/Rust touched.

### G14 — `litellm.py` patch is minimal and backwards-compatible

```bash
git diff main..HEAD -- sdk/python/src/spendguard/integrations/litellm.py | wc -l
```

Expected: ≤ 25 changed lines (the 5-LOC `_original_acompletion` kwarg + doc comment). Any number above 25 fails the gate — D12 must not touch `litellm.py` beyond the strict minimum.

## 2. Invariants (must never regress)

| ID | Invariant | How verified |
|----|-----------|--------------|
| INV-1 | **DENY never hits the upstream provider.** Counting stub (demo) and `pytest-httpx` mock (unit/integration) MUST register zero hits across all DENY decisions. | U18 + I02 + T02 + demo deny driver + SQL stub-counter delta |
| INV-2 | **Pre-call reservation precedes provider HTTP.** Sidecar `RequestDecision` RPC fires before any HTTP request to a provider endpoint. | U17 (strict order list) + I01 (event-based wire-level order) |
| INV-3 | **Fail-closed default.** Sidecar DEGRADE → `SidecarUnavailable` raised → provider not called. Only `SPENDGUARD_LITELLM_FAIL_OPEN=1` permits otherwise. | U19 + U20 |
| INV-4 | **Idempotent install.** `install()` with the same config twice is a no-op. With different config, raises explicitly. No silent re-patch over a previous patch. | U03 + U04 |
| INV-5 | **Clean uninstall.** After `uninstall()`, `litellm.acompletion is _ORIGINAL`, `Router.acompletion is _ORIGINAL_ROUTER_METHOD`, no recursive wrapper layers. | U05 + I06 |
| INV-6 | **No recursive double-reserve.** If `litellm`-internal code paths re-enter `litellm.acompletion` mid-call, contextvar guard skips re-reserve. Exactly 1 reserve per outermost call. | U21 |
| INV-7 | **No sync-in-async deadlock.** `litellm.completion()` from inside a running loop raises loudly; never enters `asyncio.run()`. | U10 |
| INV-8 | **Transitive coverage proven.** A CrewAI `Crew.kickoff_async()` with shim installed triggers ≥1 SpendGuard reserve via `litellm.acompletion`. | T01 + demo step 3 |
| INV-9 | **No mutation of the user's call kwargs.** Shim does NOT inject `spendguard` keys, does NOT rewrite `messages`, does NOT add headers visible to the caller. | U07 (asserts kwargs identity) |

## 3. Ship checklist

```
[ ] G1 build + import passes
[ ] G2 unit suite passes (≥ 22 tests; goal 26)
[ ] G3 integration suite passes (6 tests)
[ ] G4 transitive coverage smoke passes (3 tests when frameworks installed)
[ ] G5 existing test_litellm.py baseline unchanged
[ ] G6 D11 guardrail tests unchanged
[ ] G7 `make demo-up DEMO_MODE=litellm_sdk_real` exits 0 + success lines printed
[ ] G8 `make demo-up DEMO_MODE=litellm_sdk_deny` exits 0 + INV-1 stub-counter delta zero
[ ] G9 `make demo-down` clean
[ ] G10 docs site builds + new page renders with decision matrix
[ ] G11 README adapter table updated
[ ] G12 pyproject.toml extra defined
[ ] G13 no proto / SQL / Rust drift outside the demo verify SQL
[ ] G14 litellm.py diff ≤ 25 lines
[ ] INV-1 .. INV-9 all green
[ ] All 7 slices merged in order S1 → S7 with R1-R5 review loop per build-plan §1.1
[ ] Memory write-back entry `project_coverage_D12_shipped.md` drafted per build-plan §8
```

## 4. Definition of done (per build-plan §7)

- All 7 slices merged into main.
- Acceptance gates G1..G14 + invariants INV-1..INV-9 green.
- README adapter row landed: `| LiteLLM SDK shim | Python | pip install 'spendguard-sdk[litellm-shim]' |`.
- `docs/site/docs/integrations/litellm-sdk-shim.md` live with 3-path decision matrix.
- `Makefile` carries `DEMO_MODE=litellm_sdk_real` + `DEMO_MODE=litellm_sdk_deny` entries.
- Memory entry written per build-plan §8.
- Cross-link added from D11 docs page noting "for direct SDK callers see D12 shim".

## 5. Out-of-scope explicit declarations

D12 does NOT close any of:

- LiteLLM Issue #8842 upstream fix — separate workstream.
- Token-by-token streaming gating — end-of-stream commit only (same as D11 + `SpendGuardDirectAcompletion`).
- `litellm.embedding` / `litellm.aembedding` / `litellm.aimage_generation` / `litellm.atranscription` — deferred to D12.1 if observed traffic warrants.
- Sync `completion()` from inside an async context — raises rather than bridges (deadlock prevention).
- Automatic install via Python import hook — operator MUST call `install()` explicitly so monkey-patching is observable.
- Per-framework hand-written adapters for the 7 frameworks D12 transitively covers (CrewAI/DSPy/Strands/SmolAgents/BeeAI/AutoGen/Atomic Agents) — those adapter tickets (D20-D28) can be deferred or closed-as-covered once D12 ships.

These limitations are documented in `docs/site/docs/integrations/litellm-sdk-shim.md` "Limitations" section so operator expectation matches shipping surface.

## 6. Post-ship implications for the build plan

D12 shipping changes the framework coverage plan:

- D20 (Strands) — covered transitively. Strands' `HookProvider.before_invocation` adapter is no longer required for budget gating; only audit context enrichment remains as future work.
- D21 (DSPy) — covered transitively (proven by T03). DSPy `BaseCallback` adapter no longer required.
- D22 (Agno) — covered transitively if Agno routes through litellm (verify in D22 spec phase). Today's strategy memo says yes.
- D23 (BeeAI), D24 (AutoGen), D25 (SmolAgents), D28 (Atomic Agents) — same. Each adapter spec should re-evaluate scope after D12 ships.

The strategy memo `framework-coverage-2026-06.md` "LiteLLM SDK gap" section should be updated post-ship to mark Issue #8842 as "worked around in SpendGuard via D12 shim; upstream fix still desired but no longer blocking."
