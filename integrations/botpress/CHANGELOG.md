# Changelog

All notable changes to `@spendguard/botpress-integration` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this package adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — Initial release

### Added

- `Integration`-shaped default export wiring the `beforeAiGeneration` /
  `afterAiGeneration` hooks + `validateConfiguration` register
  lifecycle.
- `SpendGuardReservation` reserve / commitSuccess / releaseFailure
  delegate (composition-only, no inheritance from `@botpress/sdk` types).
- Zod configuration schema with the LOCKED v1 fields (`sidecarUrl`,
  `spendguardBudgetId`, `spendguardWindowInstanceId`, `upstreamProvider`,
  `tenantId`, optional mTLS paths).
- Error translation: `DecisionDenied` → `RuntimeError("BUDGET_DENIED")`,
  `SidecarUnavailable` → `RuntimeError("BUDGET_DEGRADED")`,
  `SpendGuardConfigError` → `RuntimeError("BUDGET_CONFIG")`.
- `event.payload.usage` extraction covering OpenAI / Anthropic /
  Bedrock-via-Botpress shapes; estimator-snapshot fallback + WARN when
  usage is missing.
- 37 unit tests + 4 integration tests against the D09 SLICE 1 sidecar
  HTTP companion mock.
- Demo: `DEMO_MODE=botpress_real` covering ALLOW + DENY + STREAM at the
  ledger / canonical events layer.
- Docs page at `docs/integrations/botpress`.

### Anti-scope (deferred)

- Workflow-node gating beyond AI hook (RAG, tool-call, knowledge-base).
- Botpress channel plugins (WhatsApp / Slack / Web Chat).
- Token-by-token mid-stream cap.
- `@botpress/sdk` 0.8.x compatibility (pinned `^0.7.0`).
- Botpress v11 (different hook surface; integration does not load).
- Strategy C customer plugin contract (v1.1).
