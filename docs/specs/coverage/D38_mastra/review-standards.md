# D38 — Review Standards

Use with `superpowers:code-reviewer` on every D38 slice (reviewer-Claude-Code-only policy, 2026-06-08 — NOT codex CLI). R1 runs the full checklist; R2–R5 focus on findings still open from the prior round. Findings are P0 / P1 / P2 / Polish; P0 + P1 block.

## 1. Precedence rule (P0 — read first)

**The LOCKED `design.md` trumps slice docs.** This is the D05/7 slice-author-bug lesson (coverage Phase B §1.2 P0): slice docs are derived artifacts and may contain authoring errors; when a slice doc and `design.md` disagree, the implementation MUST follow `design.md` and the reviewer MUST flag the slice-doc drift as a finding (P1) rather than accept the slice doc's version. A `[VERIFY-AT-IMPL]` pin may select between the pre-declared alternatives in design §12 — it may NOT introduce a third option or weaken a LOCKED decision.

## 2. Fail-closed enforcement (P0 — blocker)

| Check | Pass condition |
|---|---|
| 2.1 | NO catch-and-continue around `client.reserve()` anywhere. Every reserve-path error (DecisionDenied + subclasses, SidecarUnavailable, HandshakeError, SpendGuardError) aborts the step |
| 2.2 | NO fail-open option exists: `SpendGuardProcessorOptions` has no `failOpen` / `degradeOnUnavailable` / `enforcementMode` / equivalent (TP-04) |
| 2.3 | NO env var read weakens enforcement (grep `process.env` in `src/` — only construction-time config sourcing like `SPENDGUARD_UNIT_ID` in *examples*, never an enforcement bypass in `src/`) |
| 2.4 | DENY-before-inner-call proven against a REAL `@mastra/core` Agent: DENY ⇒ 0 `doGenerate`/`doStream` invocations (TP-10); demo proves it live via the counting-stub (TA-04/A5.2) |
| 2.5 | If V2 pins Mastra's `abort()` as the halt mechanism, the typed substrate error is preserved on the `cause` chain and tests assert reachability |
| 2.6 | The commit-path error swallow (design §7.4) is the ONLY swallow, is post-dispatch only, logs at error level, and is backed by the TTL-sweep settlement note. Do NOT flag it as fail-open; DO flag any swallow that creeps into the pre-dispatch path |
| 2.7 | The shipped D04/D06 "operational degradation" branch was NOT copy-pasted in (a `console.warn(... proceeds without budget gate ...)` string anywhere in `src/` is an automatic P0) |

## 3. Public-surface lock (P0 — blocker)

| Check | Pass condition |
|---|---|
| 3.1 | `src/index.ts` + `src/options.ts` + the `SpendGuardProcessor` class shell match design.md §5 **verbatim** (the §1.2-style copy gate). Field-for-field, name-for-name, default-for-default |
| 3.2 | `SpendGuardProcessor implements Processor` from `@mastra/core/processors`; typecheck against the installed peer passes (this IS the hook-signature gate) |
| 3.3 | `readonly name = "spendguard-processor"` |
| 3.4 | No `default` export; no re-export of substrate symbols beyond `DecisionDenied` / `SidecarUnavailable` / `SpendGuardError` (reference-identical, TP-05) |
| 3.5 | camelCase everywhere on the public surface |
| 3.6 | Constructor `TypeError` validation for missing `client` / empty `tenantId` |
| 3.7 | Surface drift after design lock = P0 + re-spec escalation, not a quiet amendment |

## 4. Substrate-hash-reuse-only (P0 — blocker)

| Check | Pass condition |
|---|---|
| 4.1 | All id/key derivation goes through `@spendguard/sdk` `deriveIdempotencyKey` / `deriveUuidFromSignature` (TP-07 delegation proof) |
| 4.2 | Zero `@noble/hashes`, `node:crypto`, `createHash`, `createHmac`, `blake2` tokens in `src/` AND in `dist/index.js` (TP-36/TP-38); no hash library in package.json (TP-37) |
| 4.3 | Identity tuple matches design §6.3 exactly: `stepId="llm_call"`, `sessionId=runId`, `decisionId=llmCallId`, scope string `"mastra_llm_call_id"` |
| 4.4 | BLAKE2b cross-language byte-equivalence (D05 §13) rides the substrate — the adapter introduces NO transformation between inputs and the substrate call (golden vector TP-09) |

## 5. unitId day-1 (P0 — blocker)

| Check | Pass condition |
|---|---|
| 5.1 | `unitId?: string` present on `SpendGuardProcessorOptions` from the slice that creates the type (COV_D38_02) — not deferred |
| 5.2 | Set ⇒ threads verbatim to `projectedClaims[N].unit.unitId`; unset ⇒ field absent (substrate coerces to `""`) (TP-19) |
| 5.3 | Demo overlay sets `SPENDGUARD_UNIT_ID=66666666-6666-4666-8666-666666666666` and the ledger reserve rows exist (A5.7 + A5.3 — empty unit_id would be rejected) |
| 5.4 | JSDoc on the field mirrors the substrate's `UnitRef.unitId` ledger-UUID-vs-slug warning |

## 6. Mastra protocol correctness (P0)

| Check | Pass condition |
|---|---|
| 6.1 | Reserve fires at `processInputStep` — the before-LLM-step boundary — and on tool-call continuation steps (TP-12) |
| 6.2 | `processLLMRequest` is a no-op in v1 (design §11.3); any reserve logic there is drift |
| 6.3 | Exactly one commit per reservation even when both response and output hooks fire (TP-31) |
| 6.4 | Streaming: one reserve at step open, one commit after stream completion, zero per-chunk RPCs (TP-30) |
| 6.5 | The processor never mutates step messages (TP-21) |
| 6.6 | Every `[VERIFY-AT-IMPL]` V1–V8 touched by the slice has a recorded pin (answer + `@mastra/core` version) in the slice doc; an unpinned marker used by shipped code is P0 |
| 6.7 | Model-router-string agents are gated (TP-22) — this is the deliverable's core claim |

## 7. Reserve/commit semantics (P0)

| Check | Pass condition |
|---|---|
| 7.1 | `trigger="LLM_CALL_PRE"`; `route` defaults `"mastra-llm"`, consumer override propagates |
| 7.2 | `claimEstimator` called exactly once per reserve; claims forwarded verbatim; default projection per design §6.4 (chars/4, `defaultBudgetMicrosCap`, `scopeId = budgetId ?? tenantId`) |
| 7.3 | SUCCESS commit: `outcome="SUCCESS"`, `outcomeKind="SUCCESS"`, decision/reservation ids from the reserve outcome |
| 7.4 | Usage actuals on the wire fields when V4 exposes them; §6.6 LOCKED fallback (`estimatedAmountAtomic = projectedAmountAtomic`) when not (TP-24..TP-26) |
| 7.5 | FAILURE path per design §6.1: `outcome="PROVIDER_ERROR"`, `outcomeKind="FAILURE"`, `actualErrorMessage`; TTL-sweep backstop documented where no hook exists (TP-27) |
| 7.6 | Unknown-correlation commit hook = warn + no-op (TP-29) |

## 8. Phase-0 reconciliation (P0 for slice 0; P1 spot-check thereafter)

| Check | Pass condition |
|---|---|
| 8.1 | D06 design.md amendment is APPEND-ONLY and dated 2026-06-10 (A1.1); wording is factual, scopes D06 to explicit AI SDK instances, points to D38 |
| 8.2 | `@spendguard/vercel-ai` peer `"ai": ">=4.0.0 <5"`, version 0.2.0, CHANGELOG entry per design §9.2 — and NOT `>=5.0.0 <7` (the justified deviation is LOCKED; "fixing" it to >=5 is a P0 finding) |
| 8.3 | Zero changes under `sdk/typescript-vercel-ai/src/` in slice 0 |
| 8.4 | `deploy/demo/vercel_ai_mastra/**` + `verify_step_vercel_ai_mastra.sql` untouched by EVERY D38 slice (A5.8) |
| 8.5 | AI SDK v6 `LanguageModelV3` middleware work is NOT smuggled into any D38 slice (design §9.3 — D06 follow-on) |

## 9. Inflight correlation (P1)

| Check | Pass condition |
|---|---|
| 9.1 | Keyed per design §6.5 (V3 id, else per-runId FIFO); bounded 10k, FIFO eviction |
| 9.2 | `pop` deletes; concurrent runs isolated (TP-32..TP-35) |
| 9.3 | Entry carries `projectedAmountAtomic` for the §6.6 fallback |

## 10. Package hygiene (P1)

| Check | Pass condition |
|---|---|
| 10.1 | ESM-only, `sideEffects: false`, tsup externals = peers, no CJS artifact |
| 10.2 | Bundle ≤ 40 KB min / ≤ 12 KB gz; breach fails build |
| 10.3 | engines `>=22.13.0`; peer `@mastra/core >=1.0.0 <2`; `@spendguard/sdk` peer per shipped D06 convention |
| 10.4 | `pnpm-workspace.yaml` lists `sdk/typescript-mastra` |
| 10.5 | No `eval` / `new Function`; prompts never logged at INFO; user structures never mutated |

## 11. Demo correctness (P1)

| Check | Pass condition |
|---|---|
| 11.1 | Overlay name `deploy/demo/mastra_processor/`, mode `mastra_processor`, success line spelled exactly `[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)` |
| 11.2 | Runner image Node ≥ 22.13 (A5.6) |
| 11.3 | `verify_step_mastra_processor.sql` uses `COV_D38_GATE` prefix and copies the langchain_ts gate structure (counts ≥, INV-2 DO block, decision-rows block); Makefile target mirrors `demo-verify-langchain-ts` incl. canonical + outbox blocks |
| 11.4 | DENY step proves counter-unchanged (A5.2); V6 pin recorded (router string vs LOCKED explicit-instance fallback) |
| 11.5 | Demo run is REAL (demo-as-quality-gate memory rule): reviewer sign-off requires the demo actually executed at head, not "should pass" |

## 12. Documentation + positioning (P2)

| Check | Pass condition |
|---|---|
| 12.1 | Positioning text derives from design §2: factual contrast with `CostGuardProcessor` sourced from upstream docs, complementary framing, zero disparagement |
| 12.2 | Aux-LLM-calls limitation box + D06 workaround present (README + docs page) |
| 12.3 | JSDoc `@throws` on hook methods enumerating the typed errors |
| 12.4 | LICENSE_NOTICES lists `@mastra/core` Apache-2.0 with the `ee/` exclusion note |
| 12.5 | docs page code blocks `is:raw` (Astro rule) |

## 13. Slice anti-scope

| Slice | Anti-scope |
|---|---|
| COV_D38_00 | ONLY D06 docs amendment + vercel-ai package.json/CHANGELOG; no `sdk/typescript-mastra/` files |
| COV_D38_01 | Skeleton only; no processor logic |
| COV_D38_02 | No commit-path code; no demo; no docs page |
| COV_D38_03 | No demo; no docs page |
| COV_D38_04 | Tests + review fixes only; no new public surface |
| COV_D38_05 | Demo/example/Makefile/SQL only; no `src/` behavior changes |
| COV_D38_06 | Docs + publish workflow only; no `src/` changes |

## 14. Findings categorisation + escalation

| Category | Definition | Action |
|---|---|---|
| **P0 / Blocker** | §1–§8 violation: fail-open branch, surface drift, local hashing, unitId missing, design-vs-slice precedence inversion, unpinned VERIFY marker in shipped code, Phase-0 gate broken | Block; fix before re-run |
| **P1 / Major** | §9–§11 failure, missing test, wrong error class, demo gate failure | Block; fix before re-run |
| **P2 / minor** | §12, stylistic, JSDoc gaps | Residual; may merge with note |
| **Polish** | naming/wording | Residual |

Escalation: same finding unresolved two consecutive rounds → Staff+ panel arbitration. Any P0 open at R5 → automatic Staff+ arbitration. Findings must be **actually fixed**, not deferred (hardening-workflow directive 2026-05-31); deferrals require explicit gh-issue residuals using the template below.

```
Title: [D38 residual] <one-line summary>
Body:
- Slice: COV_D38_<NN>_<short>
- Round: R<n>
- Category: P<0|1|2>|Polish
- Spec ref: design.md §<n> / tests.md TP-<nn> / acceptance.md A<n.m>
- Repro: <minimal command sequence>
- Why deferred: <one line>
- Suggested follow-up: <name or "TBD post-D38">
```

## 15. Sign-off

Reviewer signs off only when: every P0+P1 in §1–§11 green; anti-scope (§13) honored; the slice's acceptance subset (acceptance.md §8) actually executed with output evidence; demo gates physically run for slices that own them; residuals filed. Otherwise the round loops.

## Reviewer prompt template

The orchestrator pastes the following EXACT text (with `<...>` placeholders filled) into the `superpowers:code-reviewer` subagent for each round R1–R5:

```
You are the adversarial code reviewer for slice <SLICE_ID> (round R<N>) of
deliverable D38 — the Mastra dedicated adapter `@spendguard/mastra` for
SpendGuard (pre-dispatch budget reservation via sidecar gRPC, post-call
commit/release, signed audit chain).

Read, IN THIS ORDER, before looking at any code:
1. docs/specs/coverage/D38_mastra/design.md — the LOCKED design. Precedence
   rule: if the slice doc, the code, or a comment disagrees with design.md,
   design.md WINS. Flag the disagreement; do not adopt the slice doc's
   version (D05/7 slice-author-bug rule, review-standards.md §1).
2. docs/specs/coverage/D38_mastra/review-standards.md — apply §2–§13 as your
   checklist. §2 (fail-closed), §3 (verbatim surface), §4 (substrate-hash-
   reuse-only), §5 (unitId day-1), §6 (Mastra protocol), §7 (reserve/commit),
   §8 (Phase-0) are P0; §9–§11 are P1; §12 is P2.
3. The slice doc: docs/specs/coverage/D38_mastra/slices/<SLICE_ID>.md
   (including its "VERIFY-AT-IMPL pins" section — every design.md §12 marker
   the slice touches must be pinned with the installed @mastra/core version).
4. The diff under review: <DIFF_REF — e.g. `git diff <base>..HEAD`>.

Then verify, in code, not by trusting comments:
- Every P0 check in review-standards §2–§8 that this slice's anti-scope
  (§13) makes applicable. Pay specific attention to: any catch around
  client.reserve() that lets the step proceed; any fail-open option or env
  knob; any local hashing (grep src AND dist for noble/hashes, node:crypto,
  createHash, createHmac, blake2); options surface drift from design.md §5
  verbatim block; unitId threading; DENY-before-inner-call evidence (TP-10).
- The slice's test additions per tests.md §4 mapping — confirm the tests
  exist, assert what they claim, and were run (demand command output).
- The slice's acceptance subset per acceptance.md §8 — re-run the runnable
  gates yourself where feasible; demand execution evidence for demo gates
  (demo-as-quality-gate: "should pass" is not evidence).
- Anti-scope: nothing outside the slice's declared file set changed;
  deploy/demo/vercel_ai_mastra/** and verify_step_vercel_ai_mastra.sql are
  byte-untouched.

Output format — a numbered findings list, most severe first. Each finding:
  <n>. [Blocker|Major|minor] <file>:<line> — <one-line summary>
     Evidence: <2–5 lines quoting the offending code or missing artifact>
     Spec ref: <design.md §N | review-standards.md §N.M | tests.md TP-NN | acceptance.md AN.M>
Blocker = any P0 violation. Major = P1. minor = P2/Polish.

Do NOT propose redesigns of LOCKED decisions (design.md §11) or of the
Phase-0 resolutions (design.md §9 — including the `ai` peer range
`>=4.0.0 <5`, which is a justified, locked deviation); if you believe a
LOCKED decision is wrong, file it as a finding labeled "LOCKED-DISPUTE"
with your reasoning — the Staff+ panel arbitrates, not this review loop.

End with exactly one verdict line:
  VERDICT: PASS            (zero Blocker + zero Major)
  VERDICT: FAIL (<b> blockers, <m> majors)
```
