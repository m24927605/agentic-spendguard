# Changelog — @spendguard/flowise-nodes

All notable changes to this package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — D35 SLICE 1–6

### Added

- `SpendGuardChatModelWrapper` Flowise `INode` (canvas `Spend Guard →
  SpendGuard ChatModel Wrapper`).
- Module-level `SpendGuardClient` cache keyed by `(tenantId,
  sidecarUds)` — first chat invocation pays the gRPC handshake, every
  subsequent call inside the same Flowise process reuses the connection.
- No-code `claimEstimatorJson` input: conservative `$1` USD-micros
  default per call; JSON override for per-route tuning.
- Three install paths documented in the README (npm into Flowise
  source, `~/.flowise/nodes/` drop-in, custom Dockerfile layer).
- Unit suite — claimEstimator (CE-01..CE-07), clientCache (C-01..C-06),
  wrapper (W-01..W-16), Flowise manifest lock (M-01..M-08), fixture
  shape.
- `tests/_fixtures/chatflow_minimal.json` + `chatflow_deny.json` —
  pre-baked Flowise 2.x chatflows.
- E2E scaffolding under `tests/e2e/` (gated behind `D35_E2E=1`).
- Demo `DEMO_MODE=flowise_real`: counting-stub + Node runner exercising
  the integration's reserve / commit / release lifecycle through the
  sidecar HTTP companion; SQL gates assert INV-1 + INV-5.

### Locked design decisions

- One wrapper node, not per-provider nodes.
- D04's `SpendGuardCallbackHandler` does all the work — D35 is glue +
  node manifest only.
- Mutates the inner chat model's `callbacks` array in place; returns
  the SAME reference so downstream nodes see a normal `BaseChatModel`.
- ESM-only build, Node 20 target floor; peer-deps `@spendguard/sdk`,
  `@spendguard/langchain`, `flowise-components`.
- Self-hosted Flowise only; Flowise Cloud is out of scope.
