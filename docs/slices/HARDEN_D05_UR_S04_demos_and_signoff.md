# HARDEN_D05_UR_S04 — demo overlays + verify SQL restoration + sign-off

> **Pass**: HARDEN_D05_UR
> **Slice**: 4 of 4 (M — final closure)

## Scope

Restore the marathon-softened verify SQL gates across the ~14 affected demos. Add the `SPENDGUARD_UNIT_ID` env var to each demo overlay. Run all 14+ demos. Memory + docs reconciliation. Sign-off.

## Per-demo file pattern

For each of:
- `agent_real_langchain_ts`
- `agent_real_vercel_ai_mastra`
- `agent_real_openai_agents_ts`
- `agent_real_inngest_agent_kit`
- `agent_real_adk` (or `_adk_stub`)
- `agent_real_strands`
- `agent_real_dspy`
- `agent_real_agno`
- `agent_real_beeai`
- `agent_real_autogen`
- `agent_real_smolagents`
- `agent_real_letta`
- `agent_real_llamaindex`
- `agent_real_atomic_agents`
- `maf_python_real`
- `maf_dotnet_real`

Do:
- `deploy/demo/<name>/docker-compose.yaml` — add `SPENDGUARD_UNIT_ID=00000000-0000-4000-8000-000000000001` (or the demo's existing canonical) to adapter-runner service env
- `deploy/demo/<name>/seed_*.sql` or equivalent — ensure ledger_units row exists with that UUID
- `deploy/demo/verify_step_agent_real_<name>.sql` — restore HARD gates per [`tests.md`](../specs/harden_d05_unit_ref/tests.md) §4.1; remove `coalesce(0, ...)` and `RAISE NOTICE` softening
- `deploy/demo/Makefile` — remove `|| echo "skipped...D05 UnitRef gap"` softening on the verify target

## Run cadence

After each demo overlay is updated:
```bash
timeout 600 make demo-up DEMO_MODE=<name>
make demo-verify-agent-real-<name>
```
Each MUST exit 0 + emit success line. Accumulate evidence transcripts.

## Memory + docs reconciliation

- `memory/project_coverage_phase_b.md` — mark D05 UnitRef gap CLOSED, cite the HARDEN_D05_UR commit
- `memory/MEMORY.md` — add `[HARDEN_D05_UR closed](harden_d05_ur_closed.md)` entry
- NEW `memory/harden_d05_ur_closed.md` — completion memo with the substrate-fix shape + closing date
- Each adapter docs/site-v2/.../*.mdx — remove "D05 UnitRef gap" Limitations bullet
- Each adapter CHANGELOG.md — already updated in SLICE 2/3; verify

## Final master target

NEW `deploy/demo/Makefile` target:
```
demo-verify-all-d05-ur:
    @echo "[harden_d05_ur] running all 14+ demos sequentially..."
    @make demo-up DEMO_MODE=agent_real_langchain_ts && make demo-verify-agent-real-langchain-ts
    ...
    @make demo-up DEMO_MODE=maf_dotnet_real && make demo-verify-maf-dotnet-real
    @echo "[harden_d05_ur] ALL 14+ demos PASS"
```

## Anti-scope

- ❌ Substrate / adapter contract changes (done in SLICE 1/2/3)
- ❌ New adapter additions
- ❌ Demo image changes (other than env var)

## Acceptance gates

Per [`acceptance.md`](../specs/harden_d05_unit_ref/acceptance.md):
- §2 all 14+ demos PASS with canonical success lines
- §3 all 14+ verify SQLs PASS without softening
- §7 docs rectification complete
- §8 memory updated
- §11 `make demo-verify-all-d05-ur` exit 0
- §12 marathon retrospective closure

## Reviewer

Claude Code CLI per LOCKED standards. Single R1 reviews the full SLICE 4 sweep + master target.

## Backlinks

- Spec set: [`implementation.md`](../specs/harden_d05_unit_ref/implementation.md) §3; [`acceptance.md`](../specs/harden_d05_unit_ref/acceptance.md) §2-§12
- Marathon completion implies CLOSED HARDEN_D05_UR
