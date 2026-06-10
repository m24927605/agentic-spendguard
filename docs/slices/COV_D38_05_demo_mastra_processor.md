# COV_D38_05 — D38 Mastra adapter: demo mode `mastra_processor`

> **Deliverable**: D38 Mastra dedicated adapter (`@spendguard/mastra`)
> **Slice**: 5 of 7 (M — demo overlay + runner + SQL gates + Makefile)
> **Spec set**: [`docs/specs/coverage/D38_mastra/`](../specs/coverage/D38_mastra/)
> **Precedence**: `design.md` is LOCKED and trumps this doc (review-standards §1). Any disagreement here is a slice-author bug — follow design.md and flag the drift.

## Scope

Ship the demo-as-quality-gate proof: `examples/mastra-processor/` 3-step runner (ALLOW + DENY + STREAM against the counting-stub), `deploy/demo/mastra_processor/` compose overlay (first demo runner on the `node:22.13-bookworm-slim` base — Mastra floor; the image gate must NOT "fix" it back to node:20.10), `deploy/demo/verify_step_mastra_processor.sql` with `COV_D38_GATE` HARD assertions copied from the `verify_step_langchain_ts.sql` structure, and the `deploy/demo/Makefile` mode branches + `demo-verify-mastra-processor` target. Pins V6 (does the model-router string path honor a base-URL override). The DENY step is the live fail-closed proof: counting-stub `/_count` UNCHANGED across step 2.

The demo run must be REAL (demo-as-quality-gate memory rule) — reviewer sign-off requires the demo actually executed at head.

## Files touched

Exact set per implementation.md §1 / §6 / §8 (row COV_D38_05):

| File | Why |
|------|-----|
| `examples/mastra-processor/package.json` | NEW — example runner package |
| `examples/mastra-processor/index.mjs` | NEW — 3-step ALLOW + DENY + STREAM runner (implementation.md §6 sketch) |
| `examples/mastra-processor/README.md` | NEW — runner usage |
| `deploy/demo/mastra_processor/docker-compose.yaml` | NEW — counting-stub (verbatim copy per per-overlay isolation convention) + `mastra-processor-runner` on `node:22.13-bookworm-slim` |
| `deploy/demo/verify_step_mastra_processor.sql` | NEW — `COV_D38_GATE` assertions, structure copied from `verify_step_langchain_ts.sql` |
| `deploy/demo/Makefile` | `DEMO_MODE=mastra_processor` branches (demo-up echo/compose + run/verify dispatch, mirroring the `vercel_ai_mastra` branches at lines ~163-178 and ~747-751) + NEW target `demo-verify-mastra-processor` (mirrors `demo-verify-langchain-ts`) + `.PHONY` |

## LOCKED surface quoted verbatim — design.md §10 (Demo overlay, name LOCKED)

> - Overlay: `deploy/demo/mastra_processor/docker-compose.yaml` — `counting-stub` (verbatim copy per existing per-overlay isolation convention) + `mastra-processor-runner` (**`node:22.13-bookworm-slim`** — Mastra needs Node ≥22.13; this is the first demo runner off the node:20.10 base, called out so the image gate doesn't "fix" it back).
> - Runner script: `examples/mastra-processor/index.mjs`, 3 steps mirroring `langchain_ts` / `vercel_ai_mastra`:
>   - step 1 **ALLOW** — `agent.generate(...)` small prompt → counter +1, SUCCESS commit.
>   - step 2 **DENY** — second `SpendGuardProcessor` whose `claimEstimator` projects a claim past the demo contract's 1B-atomic hard cap → sidecar DENY pre-call → step aborts → counter UNCHANGED.
>   - step 3 **STREAM** — `agent.stream(...)` → one reserve at step open, one commit after stream end.
>   - Success line (LOCKED spelling, D11/6 §6.7 pattern): `[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)`.
> - Model source: PRIMARY — model-router string `"openai/gpt-4o-mini"` pointed at the counting-stub via base-URL override (`[VERIFY-AT-IMPL: V6]`: whether `MastraModelGateway` honors `OPENAI_BASE_URL`/per-provider `baseURL` config). LOCKED FALLBACK if V6 fails: explicit AI SDK provider instance with `baseURL` at the counting-stub — the Processor attach point is identical for both model sources, and a vitest integration test (TP-22) separately proves the processor mounts on a router-string agent.
> - Verify: `deploy/demo/verify_step_mastra_processor.sql` — gate structure copied from `verify_step_langchain_ts.sql` (`COV_D38_GATE` prefix): reserve ≥ 2, commit_estimated ≥ 2, denied_decision ≥ 1, INV-2 strict-order (earliest reserve < earliest `spendguard.audit.outcome`), canonical decision rows ≥ 2; plus the cross-DB canonical_events check and outbox-closure check in the Makefile target `demo-verify-mastra-processor` (mirrors `demo-verify-langchain-ts`).
> - Demo env: same tenant/budget/window/unit constants as the sibling overlays (`SPENDGUARD_UNIT_ID=66666666-6666-4666-8666-666666666666` proves day-1 unitId threading end-to-end).

### Runner sketch — implementation.md §6 (copy verbatim)

```
env: SPENDGUARD_SIDECAR_UDS, SPENDGUARD_TENANT_ID, SPENDGUARD_BUDGET_ID,
     SPENDGUARD_UNIT_ID, SPENDGUARD_COUNTING_STUB_URL, OPENAI_BASE_URL, OPENAI_API_KEY

client = new SpendGuardClient({ socketPath, tenantId, runtimeKind: "mastra-js" })
guard  = new SpendGuardProcessor({ client, tenantId, budgetId, unitId: process.env.SPENDGUARD_UNIT_ID })
agent  = new Agent({ model: "openai/gpt-4o-mini" /* V6; LOCKED fallback: explicit
         provider instance with baseURL at the counting-stub */,
         <processor-mount key per V5>: [guard] })

step 1 ALLOW : pre=/_count → agent.generate("ping") → post=/_count; assert post === pre+1
step 2 DENY  : denyGuard = new SpendGuardProcessor({ ...same, claimEstimator: () =>
               [{ scopeId: budgetId, amountAtomic: "2000000000", unit }] })  // > 1B hard cap
               assert agent2.generate(...) rejects with DecisionDenied (direct or on
               the cause chain, per V2); assert /_count UNCHANGED
step 3 STREAM: pre=/_count → agent.stream("count to 3") drained → assert exactly one
               reserve + one commit for the step; post === pre+1

success line (LOCKED): [demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

Compose/Makefile detail (implementation.md §6 verbatim):

> Compose overlay `deploy/demo/mastra_processor/docker-compose.yaml`: copy `deploy/demo/vercel_ai_mastra/docker-compose.yaml` structure verbatim with: service `mastra-processor-runner`, image **`node:22.13-bookworm-slim`**, named volume `mastra-processor-runner-modules`, `file:` dep rewrite to `/opt/spendguard/sdk/typescript` + `/opt/spendguard/sdk/typescript-mastra`, same env constants (incl. `SPENDGUARD_UNIT_ID: "66666666-6666-4666-8666-666666666666"`), same counting-stub block.
>
> Makefile (deploy/demo/Makefile): `DEMO_MODE=mastra_processor` branches in the `demo-up` echo/compose section and the run/verify dispatch section (mirror the `vercel_ai_mastra` branches at lines ~163-178 and ~747-751); new target `demo-verify-mastra-processor` mirroring `demo-verify-langchain-ts` (ledger SQL via `verify_step_mastra_processor.sql`, cross-DB canonical_events decision/outcome check, outbox-closure check); add to `.PHONY`. The `demo-verify-all-d05-ur` master target is NOT touched (HARDEN_D05_UR scope is closed).

Demo naming decision — design.md §11.12:

> **Demo mode name `mastra_processor`**, overlay `deploy/demo/mastra_processor/`, verify file `verify_step_mastra_processor.sql`, gate prefix `COV_D38_GATE`.

## VERIFY-AT-IMPL pins owned by this slice (design.md §12)

| ID | Question (design §12 verbatim) | Pre-declared alternatives (design §12 verbatim) | PIN (record at impl) |
|---|---|---|---|
| V6 | Does the model-router string path honor a base-URL override (env or per-provider config) for `"openai/..."`? | router-string demo / LOCKED explicit-instance fallback + TP-22 router-mount test (§10) | _unpinned_ |

If V6 pins NO: use the LOCKED explicit-instance fallback in the demo runner and record that the router-path enforcement claim is carried by TP-22 (vitest) — do NOT invent a third wiring (e.g. patching Mastra internals).

## Test/verification plan (tests.md §4: TA-03, TA-04, TA-05, TA-09, TA-11)

| ID | One-liner |
|----|-----------|
| TA-03 | `make demo-up DEMO_MODE=mastra_processor` exit 0 with LOCKED success line printed |
| TA-04 | DENY step proof: counting-stub `/_count` UNCHANGED across step 2 — zero provider HTTP on DENY, live fail-closed proof |
| TA-05 | `make -C deploy/demo demo-verify-mastra-processor` — SQL HARD gates + canonical + outbox-closure blocks pass |
| TA-09 | unitId E2E: with `SPENDGUARD_UNIT_ID` set, ledger reserve rows exist (empty unit_id would be rejected with `INVALID_REQUEST: claim[0].unit.unit_id empty`) |
| TA-11 | Node engine gate: runner image `node:22.13-bookworm-slim`; package engines `>=22.13.0` (not "harmonized" to 20.10) |

## Acceptance gates (acceptance.md §8 subset: A5.1..A5.8)

```sh
# A5.1 / A5.2 — demo up + LOCKED success line + DENY counter-unchanged proof
make demo-up DEMO_MODE=mastra_processor
# expect: [demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)

# A5.3 / A5.4 — SQL HARD gates + canonical + outbox blocks (ON_ERROR_STOP=1)
make -C deploy/demo demo-verify-mastra-processor

# A5.5 — Makefile wiring complete
grep -n "mastra_processor" deploy/demo/Makefile

# A5.6 — Node floor honored in the overlay
grep -n "node:22.13" deploy/demo/mastra_processor/docker-compose.yaml

# A5.7 — day-1 unitId E2E constant
grep -n "SPENDGUARD_UNIT_ID" deploy/demo/mastra_processor/docker-compose.yaml
# expect: 66666666-6666-4666-8666-666666666666

# A5.8 — D06 demo files byte-untouched
git diff --stat -- deploy/demo/vercel_ai_mastra deploy/demo/verify_step_vercel_ai_mastra.sql   # empty
```

## Anti-scope (review-standards §13 row COV_D38_05)

- Demo/example/Makefile/SQL ONLY — NO `sdk/typescript-mastra/src/` behavior changes.
- `deploy/demo/vercel_ai_mastra/**` and `deploy/demo/verify_step_vercel_ai_mastra.sql` are READ-ONLY (design §9.4 / A5.8).
- The `demo-verify-all-d05-ur` master target is NOT touched (HARDEN_D05_UR scope is closed).
- NO docs page / README content / publish workflow — COV_D38_06.
- NO per-chunk stream gating in the STREAM step — one reserve + one commit per design §8; NO auxiliary-LLM demo coverage; NO AI SDK v6 V3 middleware (design §4, §9.3).
- NO new demo modes beyond `mastra_processor`; NO renaming (name LOCKED, design §11.12).

## Backlinks

- [`design.md`](../specs/coverage/D38_mastra/design.md) — §10 (demo overlay LOCKED), §11.12, §12 (V6), §13
- [`implementation.md`](../specs/coverage/D38_mastra/implementation.md) — §6 (runner + compose + Makefile detail), §1 (companion trees), §8
- [`tests.md`](../specs/coverage/D38_mastra/tests.md) — §3 (TA-03/TA-04/TA-05/TA-09/TA-11), §4
- [`acceptance.md`](../specs/coverage/D38_mastra/acceptance.md) — §5 (A5.1..A5.8), §8
- [`review-standards.md`](../specs/coverage/D38_mastra/review-standards.md) — §11 (demo correctness), §5.3 (unitId E2E), §13
