# COV_D38_04 — D38 Mastra adapter: fail-closed matrix + estimator/threading tests + coverage floor

> **Deliverable**: D38 Mastra dedicated adapter (`@spendguard/mastra`)
> **Slice**: 4 of 7 (M — tests only; no new behavior)
> **Spec set**: [`docs/specs/coverage/D38_mastra/`](../../specs/coverage/D38_mastra/)
> **Precedence**: `design.md` is LOCKED and trumps this doc (review-standards §1). Any disagreement here is a slice-author bug — follow design.md and flag the drift.

## Scope

Harden the test suite to the spec floor: complete the full §7 fail-closed matrix in `tests/failClosed.test.ts` (created with the reserve subset in COV_D38_02 — see the NOTE in that slice doc), add the hash-reuse P0 suite (`tests/hashReuse.test.ts`: source grep, package.json dep probe, built-bundle token scan), exhaustive claimEstimator / route / budgetId / unitId threading tests, and top up the mock-sidecar suite until every tests.md §1 coverage floor is met. TA-01 (full vitest + coverage) and TA-02 (typecheck incl. `implements Processor` conformance) go green here.

Per implementation.md §8: "NO src behavior changes beyond review fixes" — any production-code edit in this slice must be a fix for a review finding, called out explicitly in the diff.

## Files touched

Exact set per implementation.md §8 (row COV_D38_04):

| File | Why |
|------|-----|
| `sdk/typescript-mastra/tests/failClosed.test.ts` | extend to the FULL §7 matrix (every error class × reserve path; TP-04 fail-open-knob probe lives in lockedSurface but is re-verified here at the matrix level) |
| `sdk/typescript-mastra/tests/hashReuse.test.ts` | NEW — TP-36..TP-38 |
| `sdk/typescript-mastra/tests/*` coverage top-up | reach tests.md §1 floors (`processor.ts` ≥ 90 %/85 %; `identity.ts`/`inflight.ts`/`flatten.ts`/`usage.ts` 100 %/90 %; package ≥ 90 %/85 %) |
| (`sdk/typescript-mastra/src/**` — review fixes ONLY) | no behavior changes beyond review-finding fixes |

## LOCKED surface quoted verbatim

### Error taxonomy + fail-closed semantics — design.md §7 (LOCKED, full matrix this slice test-pins)

| Condition | Error surfaced | Where | Step outcome |
|---|---|---|---|
| Invalid options (`client` missing, `tenantId` empty) | `TypeError` | constructor | construction fails |
| Reserve → DENY / STOP / STOP_RUN_PROJECTION | `DecisionDenied` / `DecisionStopped` | `processInputStep` | **step aborts; provider never called** |
| Reserve → REQUIRE_APPROVAL | `ApprovalRequired` (subclass of `DecisionDenied`) | `processInputStep` | step aborts; resume pattern documented, no helper in v1 |
| Sidecar unreachable / timeout / handshake missing | `SidecarUnavailable` / `HandshakeError` | `processInputStep` | **step aborts (FAIL-CLOSED)** |
| Any other substrate error on reserve | `SpendGuardError` | `processInputStep` | **step aborts (FAIL-CLOSED)** |
| Provider error mid-step | original provider error rethrown by Mastra; adapter emits FAILURE commit | response/output/error hook (`[VERIFY-AT-IMPL: V7]`) | provider error propagates; reservation settles (or TTL sweep) |
| Commit RPC failure AFTER a successful provider call | logged at error level; **not** thrown into the consumer's result | commit path | step result delivered; reservation settles via sidecar TTL sweep + audit chain |
| Commit hook with no matching inflight entry | warn + no-op | commit path | idempotent re-delivery safe |

LOCKED rules (design §7 verbatim):

> 1. **No fail-open branch anywhere.** Unlike the shipped D04/D06 adapters (which log-and-proceed on `SidecarUnavailable` per their "operational degradation" stance), `SpendGuardProcessor` propagates EVERY reserve-path error. This is a deliberate, positioning-bearing deviation (§2): in the Mastra ecosystem the fail-open niche is already occupied by `CostGuardProcessor`; D38's reason to exist is the hard gate. Any `catch`-and-continue around `client.reserve()` is a P0 finding.
> 2. **No env escape hatch.** The adapter reads NO environment variable that weakens enforcement. (`SPENDGUARD_DISABLE` exists on the substrate client for tests — the adapter neither reads nor documents it as a production path.)
> 3. **Abort mechanism**: the adapter throws the substrate typed error from `processInputStep`. `[VERIFY-AT-IMPL: V2]`: slice `COV_D38_02` MUST verify against the installed `@mastra/core` that a throw from `processInputStep` halts the step before the provider call — and if Mastra's processor runner instead requires its `abort()` mechanism to halt (TripWire-style), the adapter calls that mechanism **with the typed error preserved on the `cause` chain**. Either way the observable contract is fixed and test-pinned: **DENY ⇒ zero provider HTTP calls** (tests.md TP-10/TA-04) and the consumer can reach the typed error via the thrown error or its `cause` chain.
> 4. **The pre/post asymmetry is intentional**: fail-closed gates *dispatch* (no unguarded provider call), not *result delivery* (a post-call commit failure cannot un-spend; destroying the user's already-paid-for response would add harm without enforcement value). Reviewers must not flag the commit-path swallow as fail-open — it is the same race-guard semantics D06 §6 locked, backed by the TTL sweep.

### Hash-reuse P0 — design.md §6.3 (last property) + implementation.md §7

> - ALL derivation goes through `@spendguard/sdk`. The adapter contains zero `node:crypto` / `@noble/hashes` imports (P0, review-standards §4).

> - `dist/index.js` must not contain `blake2`, `createHash`, `createHmac`, or an inlined copy of substrate code (hashReuse + tree-shake tests).

### Coverage floors — tests.md §1 (verbatim)

| Module | Floor |
|---|---|
| `processor.ts` | ≥ 90 % stmt, ≥ 85 % branch |
| `identity.ts`, `inflight.ts`, `flatten.ts`, `usage.ts` | 100 % stmt, ≥ 90 % branch |
| Package overall | ≥ 90 % stmt, ≥ 85 % branch |

## VERIFY-AT-IMPL pins owned by this slice (design.md §12)

None. V1/V2/V3/V5 were pinned in COV_D38_02 and V4/V7 in COV_D38_03 — this slice's tests EXERCISE those pins (e.g. TP-10's V2-dependent assertion shape) but pin nothing new. If a test here contradicts a recorded pin, that is a finding against the pin's slice, not a license to re-pin.

## Test/verification plan (tests.md §4: TP-36..TP-38, full TP suite to coverage floor, TA-01/TA-02)

| ID | One-liner |
|----|-----------|
| TP-36 | `grep -RE "@noble/hashes|node:crypto|createHash|createHmac|blake2" sdk/typescript-mastra/src/` → zero matches |
| TP-37 | `package.json` has no `@noble/hashes` in any dependency block |
| TP-38 | Built `dist/index.js` contains none of the TP-36 tokens, no inlined BLAKE2 table constants; substrate externalized |
| full TP suite | TP-01..TP-38 all green at the tests.md §1 coverage floors |
| TA-01 | `pnpm -C sdk/typescript-mastra run test` exit 0 with coverage floors met |
| TA-02 | `pnpm -C sdk/typescript-mastra run typecheck` exit 0 (incl. `implements Processor` conformance) |

## Acceptance gates (acceptance.md §8 subset: A3.1, A3.7; coverage floors)

```sh
# A3.1 — full suite + coverage floors (tests.md §1)
pnpm -C sdk/typescript-mastra run test

# A3.7 — hash-reuse P0 (src AND dist; needs a fresh build for the dist scan)
pnpm -C sdk/typescript-mastra run build
pnpm -C sdk/typescript-mastra run test tests/hashReuse.test.ts

# TA-02
pnpm -C sdk/typescript-mastra run typecheck
```

## Anti-scope (review-standards §13 row COV_D38_04)

- Tests + review fixes ONLY — NO new public surface, NO new options, NO new exports, NO behavior changes in `src/`.
- NO demo overlay / example runner / Makefile / SQL — COV_D38_05. NO docs/README/publish — COV_D38_06.
- NO re-pinning of V1–V7 (owned by COV_D38_02/03); NO V8 work (COV_D38_06).
- NO weakening of any LOCKED decision to make a test pass — if a LOCKED shape seems untestable, escalate (LOCKED-DISPUTE), don't adapt the shape.
- NO per-chunk stream gating, auxiliary-LLM coverage, or AI SDK v6 V3 middleware (design §4, §9.3).
- `deploy/demo/vercel_ai_mastra/**` + `verify_step_vercel_ai_mastra.sql` byte-untouched (design §9.4).

## Backlinks

- [`design.md`](../../specs/coverage/D38_mastra/design.md) — §7 (full taxonomy, LOCKED rules), §6.3/§6.4 (threading + projection under test), §11.4/§11.6, §13
- [`implementation.md`](../../specs/coverage/D38_mastra/implementation.md) — §1 (test layout), §7 (bundle hygiene), §8
- [`tests.md`](../../specs/coverage/D38_mastra/tests.md) — §1 (coverage floors), §2 (TP-36..TP-38), §3 (TA-01/TA-02), §4
- [`acceptance.md`](../../specs/coverage/D38_mastra/acceptance.md) — §3 (A3.1, A3.7), §8
- [`review-standards.md`](../../specs/coverage/D38_mastra/review-standards.md) — §2 (fail-closed P0), §4 (hash-reuse P0), §5 (unitId threading), §13
