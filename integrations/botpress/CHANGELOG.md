# Changelog

All notable changes to `@spendguard/botpress-integration` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — Initial release

### Architecture

SpendGuard is a Botpress **LLM-provider integration**. It implements the
`generateContent` action (the point Botpress calls for an LLM completion) and
`listLanguageModels`, compiled against the real `@botpress/sdk` (6.11.x). An
earlier draft targeted imagined `beforeAiGeneration` / `afterAiGeneration` bot
hooks and a non-existent `new Integration<Configuration>({...})` constructor;
those never compiled against the real SDK and have been removed.

### Added

- `integration.definition.ts` — a real `IntegrationDefinition` declaring the
  `generateContent` + `listLanguageModels` actions and the `modelRef` entity,
  plus the configuration schema. Native actions (not `.extend(llm)`) keep
  `bp build` codegen fully offline / auth-free; the public `llm` interface
  depends on the unpublished `@botpress/common` and an authenticated
  `bp add llm`.
- `src/index.ts` — a `new bp.Integration({...})` (typed against the generated
  `.botpress` codegen) implementing `register` -> `validateConfiguration`,
  `generateContent`, and `listLanguageModels`.
- `generateContent` flow: reserve budget with the SpendGuard sidecar, forward
  to the configured upstream provider (OpenAI / Anthropic / Bedrock), commit
  real usage — fail-closed throughout.
- `SpendGuardReservation` reserve / commitSuccess / releaseFailure delegate
  against the D09 SLICE 1 sidecar HTTP companion (reserve `/v1/decision`,
  commit/release `/v1/trace`), composition-only.
- Provider forward (`src/provider/forward.ts`) — injectable `ForwardFn`,
  OpenAI-compatible + Anthropic wire shapes, default reads the provider API key
  from the environment.
- Configuration schema with the v1 fields (`sidecarUrl`, `spendguardBudgetId`,
  `spendguardWindowInstanceId`, `upstreamProvider`, `tenantId`, optional mTLS
  paths), transport-security + all-or-none mTLS refinements.
- Error translation to Botpress `RuntimeError`. The SpendGuard code
  (`BUDGET_DENIED` / `BUDGET_DEGRADED` / `BUDGET_CONFIG`) is carried in
  `metadata.spendguardCode` because the RuntimeError numeric `code` (from
  `@botpress/client`) is read-only.
- 36 unit tests against the sidecar HTTP companion mock + an injected provider
  forward, covering the reserve -> forward -> commit ordering and the
  fail-closed matrix.

### Anti-scope (deferred)

- Streaming / token-by-token mid-stream cap (v1 is non-streaming, one choice).
- Tool-call / multimodal `generateContent` inputs.
- The public Botpress `llm` interface via `.extend(llm)` (needs `bp add llm`
  + Botpress Cloud auth + the unpublished `@botpress/common`).
- A real Botpress-server end-to-end CI tier (the prior hook-based Docker tier
  targeted a non-existent architecture and has been removed).
- `@botpress/sdk` 6.12+ (`RuntimeError` / `Integration` API changed); pinned
  `>=6.11.0 <6.12.0`.
