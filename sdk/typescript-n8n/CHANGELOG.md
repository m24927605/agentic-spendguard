# Changelog

All notable changes to `n8n-nodes-spendguard` are documented in this
file. The format follows [Keep a Changelog](https://keepachangelog.com/);
the project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-06-08

### Added

- First public release.
- `SpendGuardChatModel` n8n community node wrapping `ai_languageModel`
  sub-nodes via `@spendguard/langchain` (D04).
- `SpendGuardApi` credential with `tenantId`, `socketPath`, `budgetId`,
  `windowInstanceId`, `runtimeKind` properties.
- Process-wide `SpendGuardClient` singleton cache keyed by
  `(tenantId, socketPath)` with FIFO eviction (max 16), concurrent-call
  dedup, and `beforeExit` close hook.
- Run identity resolution: `Execution ID + Node Name` / `Node Name` /
  `Custom Expression` modes.
- `mapToNodeApiError` translation of `DecisionDenied` / `DecisionStopped`
  / `DecisionSkipped` (403), `ApprovalRequired` (428),
  `SidecarUnavailable` (503), `HandshakeError` (502).
- Self-hosted n8n ≥ 1.50; CJS-only (n8n loader does not support ESM
  community nodes as of n8n 1.50).
- Apache-2.0 licence.
