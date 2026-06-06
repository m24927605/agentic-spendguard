# D08 — Review Standards

Use this checklist with `superpowers:code-reviewer` on every D08 slice. R1 runs the full checklist; R2-R5 focus only on findings still open from the previous round. Findings categorised P0 / P1 / P2 / Polish; P0 + P1 are blockers.

## 1. Behaviour invariant (P0 — blocker)

The adapter exists to enforce one claim. Reviewer must verify it in code AND in tests.

| Check | Pass condition |
|---|---|
| 1.1 | `core.ts::bracketedGetResponse` calls `inner.getResponse` AFTER `await opts.client.reserve(...)` resolves. No code path reaches the inner call before the PRE await. |
| 1.2 | `withSpendGuard` and `SpendGuardAgentsModel.getResponse` both delegate to `bracketedGetResponse` — no duplicate brackets, no skipped brackets. |
| 1.3 | A test asserts `mockInnerModel.callCount === 0` after every DENY / STOP / SKIP path (WS-02, WS-03, WS-04, M-02, M-03, M-04). |
| 1.4 | `client.reserve` throws ⇒ the function does NOT catch the typed exception, does NOT call `commitEstimated`. (DecisionDenied / DecisionStopped / DecisionSkipped / ApprovalRequired all propagate.) |
| 1.5 | `streamResponse` is documented and tested as **pass-through with no PRE/POST**. JSDoc must say so. |
| 1.6 | The demo `--mock` mode explicitly asserts the invariant ("DENY ⇒ inner Model is NEVER invoked") in its output and exits non-zero if violated. |

Any 1.1-1.6 failure = P0 blocker. The adapter is unfit for ship.

## 2. Cross-language determinism (P0 — blocker)

The signature → UUIDs → idempotency-key chain must be byte-identical to Python.

| Check | Pass condition |
|---|---|
| 2.1 | `signatureOf(input, sys)` for fixture vectors equals Python `_signature(input, system_instructions)` output. |
| 2.2 | `deriveUuidFromSignature(sig, { scope: "decision_id" })` matches Python `derive_uuid_from_signature(sig, scope="decision_id")` for every fixture entry. |
| 2.3 | `deriveUuidFromSignature(sig, { scope: "llm_call_id" })` matches Python for every fixture entry. |
| 2.4 | `deriveIdempotencyKey({...})` matches Python `derive_idempotency_key(**{...})` for every fixture entry. |
| 2.5 | The fixture file `sdk/fixtures/cross-language/v1.json` has an `openai_agents` section with ≥ 32 vectors, committed. |
| 2.6 | `pnpm run test tests/crossLanguageSignature.test.ts` runs and all vectors pass. |
| 2.7 | The `signature.ts` rendering quirk (string `repr` vs JSON object `JSON.stringify`) is documented in JSDoc AND the fixture covers both shapes. |
| 2.8 | `defaultEstimator.MODEL_BASELINE_TOKENS` is byte-identical to Python's table (DE-05 reads both). |

Drift here breaks audit-chain dedup across Python and TS deploys of the same agent runtime. P0.

## 3. Public-surface lock (P0 — blocker)

| Check | Pass condition |
|---|---|
| 3.1 | Every symbol listed in `design.md` §4.1 is exported from `src/index.ts`. |
| 3.2 | Every signature matches `design.md` §4.1-§4.4 verbatim — function names, parameter names, optionality, return types. |
| 3.3 | Subpath exports in `package.json#exports` match `design.md` §6 + `implementation.md` §2 list. |
| 3.4 | `tests/_support/contractSnapshot.ts` typechecks against v0.1.0 surface. |
| 3.5 | No `default export` anywhere. |
| 3.6 | `withSpendGuard` and `SpendGuardAgentsModel` are sibling entries — neither is reachable only via a subpath. |
| 3.7 | All public type aliases (`SpendGuardModelOptions`, `ClaimEstimator`, `RunContext`) are exported with `type` keyword from `src/index.ts`. |

## 4. ESM-only + tree-shakeability (P1)

| Check | Pass condition |
|---|---|
| 4.1 | `"type": "module"` in `package.json`. |
| 4.2 | `"sideEffects": false` in `package.json`. |
| 4.3 | No CJS artefact in `dist/`. |
| 4.4 | Subpath import test (TS-01): `import { runContext } from "@spendguard/openai-agents/run-context"` does NOT pull `core.ts` or `signature.ts` into the bundle. |
| 4.5 | tsup config emits ESM-only (`format: ["esm"]`). |
| 4.6 | Each subpath has paired `.js` + `.d.ts`. |
| 4.7 | `@openai/agents` and `@spendguard/sdk` are listed under `external` in tsup config (NOT bundled). |

## 5. Peer-dep correctness (P1)

| Check | Pass condition |
|---|---|
| 5.1 | `@openai/agents` is a peer dep, not a regular dep. |
| 5.2 | `@spendguard/sdk` is a peer dep at `^0.1.0`. |
| 5.3 | Peer-dep range for `@openai/agents` is `>=0.3 <1` (locked in `design.md` §10). |
| 5.4 | Published tarball (`pnpm pack` output) does NOT contain `node_modules/@openai/agents` or `node_modules/@spendguard/sdk`. |
| 5.5 | Customer install (simulated `pnpm add @spendguard/openai-agents @openai/agents @spendguard/sdk` in tmp dir) resolves without warnings. |
| 5.6 | Adapter typechecks against BOTH `@openai/agents` 0.3.0 and a simulated 0.4.x declaration file. |

## 6. Bundle-size budget (P1)

| Check | Pass condition |
|---|---|
| 6.1 | `pnpm run size` script exists and is wired into `prepack`. |
| 6.2 | `dist/index.js` minified ≤ 60 KB. |
| 6.3 | `dist/index.js` gzipped ≤ 18 KB. |
| 6.4 | A budget breach is a build failure, not a warning. |
| 6.5 | `pnpm pack` tarball ≤ 200 KB. |

## 7. Run-context correctness (P1)

| Check | Pass condition |
|---|---|
| 7.1 | `runContext` uses `node:async_hooks` `AsyncLocalStorage`. |
| 7.2 | The storage key is `Symbol.for("@spendguard/run-context/v1")` — a global registry key, not a module-local symbol. |
| 7.3 | Nested `runContext` calls: inner wins inside, outer restored after (RC-03). |
| 7.4 | Plan visible across `await`, `Promise.all`, `setImmediate`, `process.nextTick` (RC-04..RC-06). |
| 7.5 | `currentRunContext()` outside any context throws with a helpful error message (RC-02). |
| 7.6 | Cross-package shared key works: a sibling import (`@spendguard/openai-agents/run-context`) sees the same storage as the main entry (RC-07). |
| 7.7 | When D05 v0.2 ships its own shared `run-context`, this package can be transitively re-exported with zero behaviour change — call out the migration path in a code comment. |

## 8. Demo wiring (P1)

| Check | Pass condition |
|---|---|
| 8.1 | `examples/openai-agents-ts-composite/demo.ts` has both `--mock` and `--real` modes. |
| 8.2 | `--mock` runs without `@openai/agents` installed (in-process MockSpendGuardClient + MockInnerModel). |
| 8.3 | `--real` requires `OPENAI_API_KEY` and the demo sidecar — sys.exit-equivalent with a clear message if missing. |
| 8.4 | The Python runner (`deploy/demo/demo/run_demo.py`) `agent_real_openai_agents_ts` mode spawns the prebuilt TS demo via `node …/dist/demo.js --real` and forwards exit code. |
| 8.5 | The Makefile `demo-up` depends on `demo-ts-build` which builds both `@spendguard/sdk` and the example. |
| 8.6 | `deploy/demo/Dockerfile` installs Node 20 LTS and copies the prebuilt `dist/` directories. |
| 8.7 | Audit-chain verification (A5.4 + A5.5) is part of the demo regression — not just "demo exited 0". |

## 9. Default estimator parity (P1)

| Check | Pass condition |
|---|---|
| 9.1 | `MODEL_BASELINE_TOKENS` keys + values are byte-identical to Python's `_default_estimator.MODEL_BASELINE_TOKENS`. |
| 9.2 | DE-05 test reads Python source via a JSON snapshot generator (committed under `sdk/python/scripts/dump_default_estimator.py`) and verifies. |
| 9.3 | When `claimEstimator` is omitted in options, the default is applied; when explicit non-null, explicit wins (parity with Python SLICE_12 backward-compat decision). |
| 9.4 | The returned `BudgetClaim` shape has `budgetId`, `unit`, `amountAtomic` (string), `direction: "DEBIT"`, `windowInstanceId` — verified against the D05 proto type. |
| 9.5 | Unknown model falls back to 800 tokens (DE-03). |

## 10. Error semantics (P2)

| Check | Pass condition |
|---|---|
| 10.1 | `DecisionDenied` / `DecisionStopped` / `DecisionSkipped` / `ApprovalRequired` thrown by `client.reserve` propagate unchanged. |
| 10.2 | `commitEstimated` failure after a successful inner call is NOT swallowed — caller sees it, with `cause` carrying the inner response so adapters can recover. |
| 10.3 | When the inner call throws, `commitEstimated` is NOT invoked (the reservation expires via sidecar TTL sweep). |
| 10.4 | `ApprovalRequired.resume(client)` is the documented path — JSDoc points users at it. |
| 10.5 | Wrapper does not invent new error types — every thrown error originates from `@spendguard/sdk` or `@openai/agents` or is a standard `Error`. |

## 11. Documentation completeness (P2)

| Check | Pass condition |
|---|---|
| 11.1 | `README.md` quickstart compiles when copy-pasted (S08_01 ships a smoke check). |
| 11.2 | Every public function / class has JSDoc with a 1-line summary + `@throws` block. |
| 11.3 | `streamResponse` JSDoc explicitly documents "pass-through, no PRE/POST gating — POC scope; tracked at POST_D08". |
| 11.4 | `docs/site/docs/integrations/openai-agents-ts.md` exists and explains the wrapper invariant, install, quickstart, and how to wire the demo. |
| 11.5 | `CHANGELOG.md` 0.1.0 entry cites the Python sibling source path. |
| 11.6 | The repo-root `README.md` adapter integrations table has a `@spendguard/openai-agents` row with a working npm link (the link is added in S08_06 even if publish hasn't fired yet — `<link will be live after first publish>` is acceptable). |

## 12. AI-only workflow hygiene (P2)

Per `feedback_no_github_pr_for_ai_workflows` — D08 is dev+review+merge fully AI. No GH PR. R1-R5 review loop logs are committed to the slice commit message footer.

| Check | Pass condition |
|---|---|
| 12.1 | No GH PR opened for D08 slices. |
| 12.2 | Each slice commit message includes a `Reviewed-by: superpowers:code-reviewer (R<n>)` trailer. |
| 12.3 | If R5 panel arbitration was triggered, the arbitration memo is committed under `docs/arbitration/D08_S08_<slice>_R5.md` and linked from the merge commit. |
| 12.4 | `feedback_demo_quality_gate` honoured: A5.3 demo run is mandatory pre-merge for S08_05 — codex/reviewer green alone is insufficient. |

## 13. Things the reviewer must NOT flag

Per `feedback_dont_stop_per_slice` and the locked design decisions:

| Anti-finding | Why |
|---|---|
| "Add stream-per-chunk gating" | Locked OUT of v0.1.x. POST_D08. |
| "Apply DEGRADE mutations to the inner request" | Locked parity with Python — surface `MutationApplyFailed`. |
| "Bundle `@openai/agents` to avoid peer dep" | Peer dep is intentional; bundling breaks SDK upgrades. |
| "Add browser support" | UDS-only in v0.1.x (D05 §6). |
| "Switch composition to inheritance only" | Composition primary, subclass secondary — both ship. |
| "Add OTel auto-wiring at this layer" | OTel is D05's concern; adapter inherits whatever the `client` is configured with. |
| "Add a `disable` flag at the adapter layer" | D05's `SPENDGUARD_DISABLE` env var covers this. |

If R1-R5 surfaces any of these, the reviewer is wrong. Apply `superpowers:receiving-code-review` rigour — verify against `design.md` §3 (non-goals) and §7 (locked decisions) before changing code.
