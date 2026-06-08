# HARDEN_D05_UR — acceptance criteria

> Companion to [`design.md`](./design.md), [`implementation.md`](./implementation.md), [`tests.md`](./tests.md). Defines the LOCKED acceptance gate operators run before sign-off.

## 1. Substrate change is LIVE on main

- [ ] `sdk/typescript/src/client.ts` UnitRef has `unitId?: string` field
- [ ] `sdk/typescript/src/client.ts` `mapUnitRef` returns `unit.unitId ?? ""`
- [ ] Comment block at lines 1622-1627 REWRITTEN to reflect new behavior
- [ ] Locked-surface test extension shipped (U-LS-01 + U-LS-02)
- [ ] Wire-shape test file shipped (U-WS-01..06)

## 2. All 14 adapter demos PASS full ALLOW + DENY + STREAM

Each of the following `make demo-up DEMO_MODE=<name>` MUST succeed end-to-end and emit its canonical success line:

| Demo | Status | Success line |
|------|--------|--------------|
| agent_real_langchain_ts | □ PASS | `[demo] agent_real_langchain_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)` |
| agent_real_vercel_ai_mastra | □ PASS | `[demo] agent_real_vercel_ai_mastra ALL 3 steps PASS (ALLOW + DENY + STREAM)` |
| agent_real_openai_agents_ts | □ PASS | `[demo] agent_real_openai_agents_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)` |
| agent_real_inngest_agent_kit | □ PASS | `[demo] agent_real_inngest_agent_kit ALL 3 steps PASS (ALLOW + DENY + STREAM)` |
| agent_real_adk_stub OR agent_real_adk | □ PASS | matching line |
| agent_real_strands | □ PASS | matching line |
| agent_real_dspy | □ PASS | matching line |
| agent_real_agno | □ PASS | matching line |
| agent_real_beeai | □ PASS | matching line |
| agent_real_autogen | □ PASS | matching line |
| agent_real_smolagents | □ PASS | matching line |
| agent_real_letta | □ PASS | matching line |
| agent_real_llamaindex | □ PASS | matching line |
| agent_real_atomic_agents | □ PASS | matching line |
| maf_python_real | □ PASS | matching line |
| maf_dotnet_real | □ PASS | matching line |

Each demo's runtime is bounded by 600s per demo (10 min) in the marathon precedent; HARDEN_D05_UR demos should not exceed this.

## 3. All 14 demo verify SQLs PASS WITHOUT SOFTENING

For each demo, the `make demo-verify-agent-real-<name>` (or equivalent target) MUST:
- Exit 0
- NOT have `|| echo "skipped"` or `|| true` softening
- Assert `commit_estimated_events >= 1` (HARD)
- Assert `outbox closure event observed` (HARD)
- Assert `denied_decision >= 1` for DENY paths (HARD)
- INV-2 strict-order STILL HOLD

## 4. No regression on baseline

- [ ] `cd sdk/typescript && pnpm run test` — all baseline tests pass (no count drop)
- [ ] `cd sdk/python && pytest` — no test regression
- [ ] `cd sdk/dotnet-agent-framework && dotnet test` — no test regression
- [ ] Each affected adapter package's own test suite passes

## 5. Gates per-package

- [ ] TS substrate: typecheck + lint + build + test all green
- [ ] TS adapters × 4: typecheck + lint + build + test all green
- [ ] Python: pytest + ruff + full SDK regression all green
- [ ] .NET: dotnet build + dotnet test all green
- [ ] Docs site: astro build + astro check 0/0/0

## 6. CHANGELOG entries shipped

- [ ] `sdk/typescript/CHANGELOG.md` Unreleased entry mentions HARDEN_D05_UR
- [ ] `sdk/typescript-langchain/CHANGELOG.md` mentions plumbing
- [ ] `sdk/typescript-vercel-ai/CHANGELOG.md` mentions plumbing
- [ ] `sdk/typescript-openai-agents/CHANGELOG.md` mentions plumbing
- [ ] `sdk/typescript-inngest-agent-kit/CHANGELOG.md` mentions plumbing
- [ ] `sdk/python/CHANGELOG.md` mentions (no-op for Python SDK, but adapter plumbing changes per adapter)

## 7. Documentation rectification

- [ ] D05/9 README runbook reflects the change (cross-language fixture corpus unaffected, but visible note for consumers)
- [ ] Each affected adapter's docs/site-v2/.../*.mdx page Limitations section drops the "D05 UnitRef gap" disclosure (it's now closed)
- [ ] No lingering "cross-slice tracking: D05 UnitRef" mentions in `docs/` or `README.md`

## 8. Memory + spec drift reconciliation

- [ ] `memory/project_coverage_phase_b.md` updated to mark D05 UnitRef gap CLOSED
- [ ] `memory/MEMORY.md` references the closure
- [ ] Per-deliverable spec sets that referenced the gap (D04+D06+...) get a brief note in their respective `design.md` or `implementation.md` files

## 9. Cross-language safety

- [ ] `cd sdk/python && pytest tests/test_cross_language_fixtures.py` continues to pass — no fixture changes
- [ ] `cd sdk/typescript && pnpm run test tests/crossLanguage.test.ts` continues to pass

## 10. Performance + bundle

- [ ] `sdk/typescript/dist/index.js` minified bundle delta < 200 bytes
- [ ] No new package dep on either TS or Python SDK
- [ ] `cargo build --features mitm` (cursor_codec + windsurf_codec) unaffected

## 11. Final operator signal

- [ ] Single command `make demo-verify-all-d05-ur` (NEW) exists and exits 0 after sequencing all 14 demos + their verify SQLs
- [ ] CI nightly green on the HARDEN_D05_UR regression suite for 3 consecutive runs

## 12. Marathon retrospective closure

After all the above, the **single cross-slice tracking item from the marathon** is closed. The orchestrator's memory marker `[Coverage Phase B 23 slices shipped]` updates to indicate HARDEN_D05_UR closes the gap.

## 13. Roll-back plan

If HARDEN_D05_UR introduces a regression caught post-merge:
1. Revert just the TS substrate change (single commit, 10-line diff)
2. Adapters' `unitId?` options remain in place but become dead-code (no-op)
3. Demo softening lines remain reverted but demos will fail again — operator must re-add the soft `|| echo`
4. Total rollback: ~5 minutes

The rollback is clean because the substrate change is purely additive (no removed fields, no changed semantics for callers without `unitId`).
