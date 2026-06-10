# COV_D38_03 — D38 Mastra adapter: commit + failure settlement paths

> **Deliverable**: D38 Mastra dedicated adapter (`@spendguard/mastra`)
> **Slice**: 3 of 7 (M)
> **Spec set**: [`docs/specs/coverage/D38_mastra/`](../specs/coverage/D38_mastra/)
> **Precedence**: `design.md` is LOCKED and trumps this doc (review-standards §1). Any disagreement here is a slice-author bug — follow design.md and flag the drift.

## Scope

Wire the post-dispatch half of the lifecycle: `processLLMResponse` SUCCESS commit with usage actuals when exposed, `processOutputStep` backstop commit (at most one commit per reservation) plus FAILURE settlement when an error signal exists, `src/usage.ts` extraction, and the §6.6 LOCKED estimated-amount fallback when usage is absent. Pins V4 (usage fields + last-hook ordering on streamed steps) and V7 (error/abort signal) against the installed `@mastra/core`. Streaming whole-step tests (one reserve at step open, one commit after stream completion, zero per-chunk RPCs) land here.

Commit-path errors are swallowed (logged at error level, TTL-sweep backstop) — this is the LOCKED §7.4 pre/post asymmetry, NOT fail-open; the swallow must never creep into the pre-dispatch path.

## Files touched

Exact set per implementation.md §8 (row COV_D38_03):

| File | Why |
|------|-----|
| `sdk/typescript-mastra/src/processor.ts` | `processLLMResponse` + `processOutputStep` bodies per design §6.1 / implementation.md §3.4 |
| `sdk/typescript-mastra/src/usage.ts` | NEW — `extractUsage` per implementation.md §3.5 (V4-pinned fields; camelCase + snake_case; `undefined`, not zeros, when absent) |
| `sdk/typescript-mastra/tests/processor.test.ts` | extend with TP-23..TP-31 (commit/failure/streaming) |
| `sdk/typescript-mastra/tests/usage.test.ts` | NEW — usage-shape tests (TP-24..TP-26 support) |
| `sdk/typescript-mastra/tests/_support/stubModel.ts` | recording/throwing/tool-calling stub models for the real-Agent loop tests (TP-23/27b/28/30/31) |
| `sdk/typescript-mastra/src/inflight.ts` | R2 — additive `InflightEntry.unit` (reserve-time unit; design §6.7 amendment 2026-06-10) |
| `sdk/typescript-mastra/tests/inflight.test.ts` | R2 — entry fixture carries the new `unit` field |

## LOCKED surface quoted verbatim

### Lifecycle mapping table — design.md §6.1 (LOCKED)

| Mastra hook | When it runs (per Mastra docs) | SpendGuard action |
|---|---|---|
| `processInputStep` | every step including tool-call continuations, before messages are sent to the LLM | RESERVE — `client.reserve(trigger="LLM_CALL_PRE")`; throw on any failure (fail-closed) |
| `processLLMRequest` | immediately before each provider call | v1: no-op (assert-only in tests: reserve must already be inflight) |
| `processLLMResponse` | after each provider response | COMMIT — `client.commitEstimated(outcome="SUCCESS", outcomeKind="SUCCESS")` with usage actuals when exposed |
| `processOutputStep` / output hooks | after the step's output is assembled | backstop COMMIT if the response hook did not fire for this reservation (streaming ordering — `[VERIFY-AT-IMPL: V4]`); FAILURE settlement if an error/abort signal is exposed (`[VERIFY-AT-IMPL: V7]`) |
| step failure (provider error) | error surfaced through whichever hook/callback Mastra exposes | FAILURE-COMMIT — `commitEstimated(outcome="PROVIDER_ERROR", outcomeKind="FAILURE", actualErrorMessage=err.message)`; if no error hook exists, the sidecar TTL sweep is the LOCKED settlement backstop |

### Commit estimation — design.md §6.6 (LOCKED fallback per constraint)

> - When the response/output hook exposes provider usage (Mastra 1.x normalizes nested AI SDK v6 usage to flat fields — `[VERIFY-AT-IMPL: V4]` pins the exact field names), commit with `estimatedAmountAtomic: "0"`, `actualInputTokensWire` / `actualOutputTokensWire` from usage — identical wire shape to the shipped D04 handler.
> - **When usage is NOT available at the hook** (LOCKED fallback): commit with `estimatedAmountAtomic = projectedAmountAtomic` carried in the inflight entry (the §6.4 default-estimator projection) and actuals omitted. The reservation settles at the estimate; the audit chain records that no provider actuals were observed.

> **NOTE (append-only, 2026-06-10, orchestrator-ratified)**: the `estimatedAmountAtomic: "0"` literal in the quoted first bullet is a ratified erratum — the D04/HARDEN_D05_WI wire shape controls (SUCCESS-with-usage estimate = input+output token sum; the ledger rejects 0; usage-absent fallback unchanged). See design.md §6.7 amendment #2. The same amendment (#1) adds the reserve-time `unit` to the §6.5 inflight entry so commits tuple-match the reservation under a custom `claimEstimator` (R1 Major 1). TP-24 below predates the erratum; the implemented assertion is the token-sum estimate.

### Streaming posture — design.md §8 (LOCKED — D04/D24 parity)

> Mastra's agent loop streams. D38 brackets the WHOLE step at the before-LLM boundary:
>
> - Reserve fires once at `processInputStep` (before the first chunk leaves the provider) and covers the entire step.
> - Commit fires once after the step's stream completes, with usage from the response metadata when exposed (§6.6 fallback otherwise).
> - Per-chunk gating is explicitly out of scope (§4). Mid-stream abort → FAILURE settlement path (§6.1 last row).

### Pre/post asymmetry — design.md §7 LOCKED rule 4 + taxonomy rows

> 4. **The pre/post asymmetry is intentional**: fail-closed gates *dispatch* (no unguarded provider call), not *result delivery* (a post-call commit failure cannot un-spend; destroying the user's already-paid-for response would add harm without enforcement value). Reviewers must not flag the commit-path swallow as fail-open — it is the same race-guard semantics D06 §6 locked, backed by the TTL sweep.

| Condition | Error surfaced | Where | Step outcome |
|---|---|---|---|
| Provider error mid-step | original provider error rethrown by Mastra; adapter emits FAILURE commit | response/output/error hook (`[VERIFY-AT-IMPL: V7]`) | provider error propagates; reservation settles (or TTL sweep) |
| Commit RPC failure AFTER a successful provider call | logged at error level; **not** thrown into the consumer's result | commit path | step result delivered; reservation settles via sidecar TTL sweep + audit chain |
| Commit hook with no matching inflight entry | warn + no-op | commit path | idempotent re-delivery safe |

### `extractUsage` signature — implementation.md §3.5

> `extractUsage(args: unknown): { inputTokens: number; outputTokens: number; providerEventId?: string } | undefined`. Reads the V4-pinned flat usage fields; accepts both camelCase and snake_case shapes (D04/D06 `extractTokenUsage` discipline) and tolerates non-object bags. Returns `undefined` (NOT zeros) when usage is absent so the caller selects the §6.6 estimated-amount fallback.

Failure settlement decision — design.md §11.9:

> **Failure settlement = FAILURE commit where a hook exists; sidecar TTL sweep is the guaranteed backstop.** Explicit `client.release()` is reserved for a cancel-before-dispatch path if Mastra exposes one (`[VERIFY-AT-IMPL: V7]`); absence does not block v1.

## VERIFY-AT-IMPL pins owned by this slice (design.md §12)

| ID | Question (design §12 verbatim) | Pre-declared alternatives (design §12 verbatim) | PIN (record at impl) |
|---|---|---|---|
| V4 | Which usage fields (flat normalized) does `processLLMResponse` / `processOutputStep` expose, and which hook fires last on streamed steps? | usage actuals / LOCKED estimated-amount fallback (§6.6); backstop-commit ordering (§6.1) | **PINNED (COV_D38_03, `@mastra/core` 1.41.0)** — usage actuals selected. Flat fields: camelCase `inputTokens`/`outputTokens` (`LanguageModelUsage = LanguageModelV2Usage & {...}`; the loop's `normalizeUsage()` flattens AI SDK v6/V3 nested usage onto them, as §6.6 predicted; both fields `number \| undefined`). Exposure: `processOutputStep` carries flat `args.usage` directly; `processLLMResponse` carries NO flat usage field — usage rides the stripped `finish` chunk's `payload.output.usage` in `args.chunks` (provider response id at a `response-metadata` chunk's `payload.id`). Ordering on streamed steps: `processLLMResponse` (input-processor runner; installed .d.ts: "called after the LLM step completes (or a cached response is replayed)", with `fromCache: boolean` flagging replays) fires FIRST, `processOutputStep` (output-processor runner) fires LAST → `processOutputStep` is the §6.1 backstop; it only fires for `outputProcessors`-mounted instances. Key recovery at the commit hooks (V3 corollary): hooks expose no step messages, so the reserve hook stashes the §6.5 runId key in the per-request per-processor `state` bag (one `processorStates` Map is threaded through every runner the loop builds for a request); `runIdProvider` is the secondary source. Full pin block: `sdk/typescript-mastra/src/usage.ts` + `src/processor.ts` headers. |
| V7 | Does the Processor surface expose an error/abort signal usable for the FAILURE commit? | FAILURE commit at the signal / TTL-sweep-only settlement (§6.1) | **PINNED (COV_D38_03, `@mastra/core` 1.41.0)** — FAILURE commit at the signal. TWO signals, deduped by the FIFO inflight pop: (1) PRIMARY (empirically proven, TP-27b): model-execution errors arrive as an `error` CHUNK on `processLLMResponse`'s `args.chunks` (`{ type: "error", payload: { error } }`, `payload.error.message` preserved) — a throwing model yields chunks `["step-start","error"]` at the response hook, which emits the FAILURE commit before Mastra rethrows; (2) SECONDARY: the installed `processAPIError` hook (non-retryable API rejections; `runProcessAPIError` iterates input+output+error processors so the `inputProcessors` mount receives it; empirically NOT invoked for plain model throws). Mid-stream consumer abort invokes neither signal → sidecar TTL sweep is the LOCKED settlement backstop (§6.1 last row / §8). NO cancel-before-dispatch hook exists → NO `client.release()` path (§11.9). |

## Test/verification plan (tests.md §4: TP-23..TP-31)

| ID | One-liner |
|----|-----------|
| TP-23 | Happy path: reserve → response → exactly ONE `commitEstimated` with `outcome="SUCCESS"`, `outcomeKind="SUCCESS"`, ids from the reserve outcome |
| TP-24 | Usage exposed (V4 camelCase) → `actualInputTokensWire`/`actualOutputTokensWire` carry it; `estimatedAmountAtomic="0"` |
| TP-25 | Usage exposed snake_case → same as TP-24 |
| TP-26 | Usage ABSENT → commit carries `estimatedAmountAtomic === projectedAmountAtomic`; no actuals fields (§6.6 LOCKED fallback) |
| TP-27 | Provider error → FAILURE commit (`outcome="PROVIDER_ERROR"`, `outcomeKind="FAILURE"`, `actualErrorMessage`) when V7 signal exists; if V7 pinned "no error hook": NO success commit, inflight entry remains for TTL settlement |
| TP-28 | Commit RPC failure after success → consumer still gets the step result; error logged; no throw |
| TP-29 | Commit hook with no inflight entry → warn + no-op (no throw, no RPC) |
| TP-30 | Streaming step → exactly one reserve at open + one commit after completion; no per-chunk RPCs |
| TP-31 | At-most-one-commit: response AND output hooks both fire → exactly one commit RPC |

## Acceptance gates (acceptance.md §8 subset: A3.5)

```sh
# A3.5 — TP-11..TP-12, TP-17..TP-31 all green (reserve subset from COV_D38_02 + this slice's commit TPs)
pnpm -C sdk/typescript-mastra run test tests/processor.test.ts tests/usage.test.ts
```

## Anti-scope (review-standards §13 row COV_D38_03)

- NO demo overlay / example runner / Makefile / SQL — COV_D38_05. NO docs page / README / publish — COV_D38_06.
- NO public-surface changes — design §5 is verbatim-locked; no new options, no new exports.
- NO per-chunk stream gating (design §4 / §8 — whole-step bracket only).
- NO `client.release()` call unless V7 pins a cancel-before-dispatch path (design §11.9); do not invent one.
- NO weakening of the reserve path: the commit-path swallow is the ONLY swallow, post-dispatch only (review-standards §2.6).
- NO auxiliary-LLM coverage (memory titles, ModerationProcessor classifier, scorers) and NO AI SDK v6 V3 middleware (design §4, §9.3).
- `deploy/demo/vercel_ai_mastra/**` + `verify_step_vercel_ai_mastra.sql` byte-untouched (design §9.4).

## Residual notes

- backstop-commits-for-real test (output-mounted-only / cached-replay settlement via `processOutputStep`) deferred to COV_D38_04 (R1 minor 3).

## Backlinks

- [`design.md`](../specs/coverage/D38_mastra/design.md) — §6.1, §6.6, §7 (rule 4 + commit rows), §8, §11.8–§11.10, §12 (V4/V7), §13
- [`implementation.md`](../specs/coverage/D38_mastra/implementation.md) — §3.4 (processor commit skeleton), §3.5 (usage.ts), §4 (substrate call map), §8
- [`tests.md`](../specs/coverage/D38_mastra/tests.md) — §2 (TP-23..TP-31), §4
- [`acceptance.md`](../specs/coverage/D38_mastra/acceptance.md) — §3 (A3.5), §8
- [`review-standards.md`](../specs/coverage/D38_mastra/review-standards.md) — §2.6 (swallow scope), §6.3/§6.4 (one commit, streaming), §7 (reserve/commit semantics), §13
