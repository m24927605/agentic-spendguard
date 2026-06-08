# HARDEN_D05_UR — review-standards

> Companion to [`design.md`](./design.md), [`implementation.md`](./implementation.md), [`tests.md`](./tests.md), [`acceptance.md`](./acceptance.md). LOCKED gates for Claude Code CLI reviewer per [[feedback_reviewer_claude_code_only]].

## §1. P0 invariants (Blocker if missing)

### §1.1 Substrate signature LOCKED
- `UnitRef.unitId` is `string | undefined` (optional, no default)
- `mapUnitRef` returns `unitId: unit.unitId ?? ""`
- NOT a method, NOT a getter, NOT a computed property — plain field

### §1.2 Backward compatibility
- Existing calls `reserve({...claims: [{unit: { unit: "X", denomination: 0 }}]})` without `unitId` continue to compile and run
- Wire shape carries empty string `unitId` (same as before)
- The sidecar's rejection of empty `unit_id` is UNCHANGED — this is intentional: the substrate change exposes a previously-hidden field, not changes semantics

### §1.3 Cross-language byte-equivalence preserved
- TS `mapUnitRef` does NOT introduce any hash/serialization that breaks cross-language fixture parity
- Python SDK side already exposes `unit_id` — no Python changes needed
- 20 cross-language fixtures continue to pass on both sides

### §1.4 Adapter contract additive only
- Each adapter's options interface grows by 1 optional field — NO BREAKING CHANGES
- Existing adapter constructors with just `client` + `tenant_id` continue to work
- Test suites baseline counts hold (no test removed by HARDEN_D05_UR)

### §1.5 Demo verify SQL HARD restoration
- ZERO `|| echo "skipped"` softening allowed in `demo-verify-agent-real-<name>` Makefile targets
- ZERO `coalesce(0, ...)` placeholder in the verify SQL HARD-gates
- ZERO `RAISE NOTICE` instead of `RAISE EXCEPTION` where the intent is a hard fail

## §2. P1 gates (Major if missing)

### §2.1 JSDoc / docstring rigor
- TS `UnitRef.unitId` JSDoc must explain: ledger UUID, optional, env-var fallback pattern, NOT interchangeable with `unit` slug, recipe-style integration may omit
- Python equivalent docstring on each adapter's `unit_id` parameter
- Each CHANGELOG entry cites HARDEN_D05_UR

### §2.2 Test coverage targets
- TS substrate: ≥8 new tests (per [`tests.md`](./tests.md) §1.1-§1.3 + §3)
- TS adapter (each of 4): ≥3 tests covering options + wire + backward compat
- Python adapter (each of ~9): ≥3 tests covering same
- .NET adapter (D07): ≥3 tests covering same
- Cross-adapter smoke: ≥1 integration test against mock sidecar

### §2.3 Demo run evidence in commit message
- Each commit that closes a demo's softening MUST include the runner's success line transcript as evidence
- e.g., `[demo] agent_real_langchain_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)` (verbatim)

### §2.4 Bundle size delta
- TS substrate minified bundle delta < 200 bytes
- No new runtime dep added to any SDK package

## §3. P2 (Minor — flag, do not block)

### §3.1 Naming consistency
- Adapter options field: `unitId` (TS camelCase) or `unit_id` (Python snake_case)
- Adapter env var: `SPENDGUARD_UNIT_ID` (canonical)
- Adapter docs: cite the UUID, not the unit slug

### §3.2 Test naming
- TS test names follow LangChain-handler/Vercel/openai-agents/inngest test name convention
- Python test names follow `test_unit_id_threads_to_wire_*` pattern

### §3.3 CHANGELOG entry shape
- Keep-a-Changelog format
- Each adapter's CHANGELOG mentions HARDEN_D05_UR by name

## §4. Spec-vs-prompt override rule

If during impl the slice-doc prompt conflicts with `design.md` / `implementation.md`:
- LOCKED spec wins (per [[feedback_dont_stop_design]] rule + general spec authority)
- Implementer flags as deviation in slice ship message
- Reviewer accepts if LOCKED spec cite is correct

This is the same override rule used in D05/7 R1 (where slice doc had wrong shape; LOCKED design won).

## §5. Round-pass rule

Per [[feedback_hardening_workflow]]:
- R1 reviewer dispatches `superpowers:code-reviewer` (Claude Code CLI subagent)
- Round PASSES if reviewer's finding list is **empty** after fixes for that round
- A `Major` finding counts as a finding; a `Minor` does NOT block the round
- A `Blocker` finding requires R2

## §6. R5 escalation

Per the user's directive 2026-06-08:
- Max **5 R-rounds per slice**
- If R5 still has Blockers, dispatch in parallel: Software Architect + Backend Architect + AI Engineer + Security Engineer + Senior Developer (5 Staff+ specialists)
- Each independently writes verdict + recommended fix
- Orchestrator synthesizes final fix and executes
- Slice ships at the synthesized fix, NOT a vote — orchestrator decides per [[feedback_codex_iteration_pattern]] precedent

## §7. Marathon discipline

- No new test fabrication (per [[project_slice_05_shipped]] R2 lesson)
- No silent spec drift (per D05/7 slice-author bug retrospective)
- Each declared deviation cites the LOCKED spec section + a one-line reason
- Demo-as-quality-gate (per [[feedback_demo_quality_gate]]) — Codex / R1 PASS is insufficient; demo MUST run

## §8. Specific Claude Code review prompt template

Each R1 review dispatch MUST include this in the prompt:

> Reviewer role: per [[feedback_reviewer_claude_code_only]], you are the LOCKED Claude Code CLI reviewer for HARDEN_D05_UR. Find ALL Blocker + Major findings. Cite design.md / implementation.md / tests.md / acceptance.md by section. State PASS / NEEDS WORK explicitly. Quantify findings (B-N, M-N, m-N). Verify TS substrate compiles + tests pass. Run the actual `make demo-up DEMO_MODE=<name>` if a slice closes a demo's softening. Be terse.

## §9. R5 panel composition (LOCKED for HARDEN_D05_UR)

When R5 escalates:
1. **Software Architect** — system-design + abstraction consistency (UnitRef interface design)
2. **Backend Architect** — Rust services + ledger schema + sidecar reserve flow
3. **AI Engineer** — TS SDK substrate + cross-language invariants + LLM-facing surfaces
4. **Security Engineer** — input validation, audit chain integrity, no PII leak
5. **Senior Developer** — multi-language idiomatic correctness + bundle hygiene + test rigor

Each panelist gets the slice context + R1-R5 transcript + their domain section of the spec set.

## §10. Anti-patterns (flag during review)

- ❌ Adding `unitId: string` (required, not optional) — breaks backward compat
- ❌ Adding server-side derivation of unit_id from unit_name — design anti-scope §3
- ❌ Removing the existing `unitName: unit.unit` line — breaks the wire shape's free-form slot
- ❌ Adding UUID validation in the SDK — that's the ledger's contract, not the SDK's
- ❌ Forgetting to update one of the 14+ demo overlays — partial closure leaves residual softening
- ❌ Removing the legacy "ledger resolves canonical truth server-side" comment without writing a replacement comment — context loss

## §11. Sign-off rubric

A HARDEN_D05_UR slice is sign-off-ready when:
1. All gates green (typecheck + lint + build + test)
2. ≥1 Claude Code reviewer R1 PASS verdict
3. Zero open Blocker / Major findings (Minors allowed, tracked)
4. All declared deviations cite LOCKED spec section + reason
5. Demo (if affected) PASSED with real success-line evidence
6. Memory updated per [`acceptance.md`](./acceptance.md) §8

When ALL N slices are sign-off-ready, the HARDEN_D05_UR pass is COMPLETE.
