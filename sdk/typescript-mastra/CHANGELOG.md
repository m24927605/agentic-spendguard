# `@spendguard/mastra` Changelog

All notable changes to the Mastra Processor adapter for the SpendGuard SDK.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0] — 2026-06-11

First public release. Closes coverage deliverable D38 — locked spec set:
[`docs/specs/coverage/D38_mastra/`](../../docs/specs/coverage/D38_mastra/).

### Added

- **`SpendGuardProcessor`** — a `@mastra/core` `Processor` implementation
  that reserves budget **pre-dispatch** at the before-LLM-step boundary
  (`processInputStep`, firing on tool-call continuation steps too), commits
  on the LLM response (`processLLMResponse`, with a `processOutputStep`
  backstop for streamed-step ordering), and settles provider failures with
  a FAILURE commit; the sidecar TTL sweep is the guaranteed settlement
  backstop when no hook fires. Mounts via the Agent's `inputProcessors`
  (+ `outputProcessors` for the backstop) — so it covers **model-router
  string Agents** (`model: "openai/gpt-4o"`), the flagship Mastra path
  that has no `wrapLanguageModel` injection point.
- **Fail-closed only.** Sidecar unreachable or DENY ⇒ the step aborts with
  a typed error (`DecisionDenied` family, `SidecarUnavailable`,
  `SpendGuardError`) and the provider call never fires. There is no
  fail-open option and no env escape hatch — a deliberate deviation from
  the `@spendguard/langchain` / `@spendguard/vercel-ai` degradation
  branches. Positioning (design §2): in the Mastra ecosystem the soft-warn
  niche is already served by Mastra's own `CostGuardProcessor` (per its
  docs: best-effort threshold, fail-open, async cost persistence);
  `SpendGuardProcessor` is the complementary **hard enforcement** layer —
  pre-dispatch reservation against a durable ledger with a signed audit
  chain, sharing one `budget_id` with every other SpendGuard integration.
- Options surface: `client`, `tenantId` (required, explicit), `budgetId`,
  `unitId` (day-1 ledger unit threading), `route`,
  `defaultBudgetMicrosCap`, `claimEstimator` (claims forward verbatim,
  including `windowInstanceId`), `runIdProvider`, and `pricing`
  (`PricingFreeze` repeated on the commit wire — production sidecars
  reject empty-tuple commits against bundle-stamped reservations; design
  §6.7 amendment #3).
- All id/hash derivation via `@spendguard/sdk`
  (`deriveIdempotencyKey`, `deriveUuidFromSignature`) — zero local hash
  code; byte-deterministic idempotency keys collapse same-step retries
  onto one reservation.
- Demo mode `mastra_processor` (`make demo-up DEMO_MODE=mastra_processor`)
  with HARD SQL verify gates: ALLOW + DENY + STREAM against a real
  `@mastra/core` Agent, proving the provider stub's hit counter does not
  move on the denied step.

### Changed

- **Supersedes the "covers Mastra" transitive-coverage claim of
  `@spendguard/vercel-ai` (D06).** Mastra owns its own agent loop since
  v0.14.0 and no longer calls `generateText` / `streamText` from `ai`.
  D06's Mastra coverage is re-scoped to *explicit AI SDK model instances*
  (see `@spendguard/vercel-ai` 0.2.0 and the dated 2026-06-10 amendment in
  `docs/specs/coverage/D06_vercel_ai_sdk/design.md` §9); `@spendguard/mastra`
  owns Mastra Agents — router strings and explicit instances — at the
  processor boundary.

### Known limitations

- Auxiliary LLM calls (Mastra memory title generation,
  `ModerationProcessor`'s classifier, scorers) are out of v1 scope —
  wrap those models explicitly via `@spendguard/vercel-ai`.
- `withMastra()` (plain-AI-SDK mounting, separate `@mastra/ai-sdk`
  package) is unsupported in v1.
- Streaming is bracketed whole-step (no per-chunk gating); Mastra
  `Workflow` step gating and tool-call PRE gating are v2 candidates.
- Consumer catch contract at the Agent boundary is message-match
  (Mastra 1.41.0 serializes processor errors inside its workflow engine;
  `instanceof` holds at the hook boundary —
  [gh #181](https://github.com/m24927605/agentic-spendguard/issues/181)).
