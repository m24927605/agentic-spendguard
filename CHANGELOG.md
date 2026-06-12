# SpendGuard Changelog

All notable product-level changes are recorded here. SDK-only releases continue to use `sdk/python/CHANGELOG.md`.

Version tags follow `vYYYY.MM.DD-ga.N` for GA release candidates and GA releases.

## Unreleased

- **OpenClaw provider plugin adapter (D40b)** — new
  `@spendguard/openclaw-provider-plugin` package for trusted OpenClaw
  deployments. The adapter wraps OpenClaw's pinned `wrapStreamFn(ctx)` provider
  hook, reserves before provider dispatch, fails closed on DENY or sidecar
  outage, and settles SUCCESS / PROVIDER_ERROR / CLIENT_TIMEOUT / RUN_ABORTED
  using reserve-time unit/pricing tuples. New
  `DEMO_MODE=openclaw_provider_plugin` hard gate validates the committed
  plugin config fixture, runs ALLOW + DENY + STREAM + PROVIDER_ERROR against
  a local counting upstream, proves DENY leaves the provider counter
  unchanged, and asserts ledger / canonical / outbox rows. This is an
  in-process enforcement hook, not a sandbox; D40a base-URL routing remains
  the durable fallback when plugin install is not acceptable.
- **OpenClaw base-URL recipe (D40a)** — new OpenClaw drop-in recipe and
  `examples/openclaw-base-url/` config template for routing
  `api: "openai-completions"` custom-provider traffic through the SpendGuard
  egress proxy at `http://localhost:9000/v1`. New
  `DEMO_MODE=openclaw_base_url` hard gate validates the pinned OpenClaw config
  fixture, runs ALLOW + DENY + STREAM through a local counting upstream, proves
  DENY leaves the provider counter unchanged, and asserts exact ledger /
  canonical / outbox rows. This is base-URL recipe coverage only; the OpenClaw
  provider plugin remains D40b.
- **AG-UI spend-event family (D39, display-only)** — new `@spendguard/ag-ui`
  npm package + `spendguard.integrations.ag_ui` Python module: pure builders
  for the five `spendguard.*` AG-UI `CUSTOM` events
  (`spendguard.budget.snapshot`, `spendguard.reservation.created`,
  `spendguard.reservation.committed`, `spendguard.reservation.released`,
  `spendguard.decision.denied`) with a locked canonical-JSON serializer
  (byte-identical TS↔Python via the frozen `ag_ui_v1.json` corpus) and an
  SSE encode helper. Display-only: the events report decisions the
  SpendGuard adapters + sidecar already made and can neither grant nor deny
  spend. New `DEMO_MODE=ag_ui_events` + `make demo-verify-ag-ui-events`
  prove the family end-to-end against a real sidecar run (exact 4-frame SSE
  gate + SSE↔ledger reservation join + ledger SQL gates).
- GA readiness phase started after HARDEN_08.
- Release bundle tooling added in GA_01.
- Operator upgrade warning: before rolling a SLICE_02+ sidecar, grep
  contract bundles for `condition:`. v1alpha2 bundles with CEL
  `condition:` fail to load with `bundle_validation_failed`; amount-only
  predicates may be rewritten to declarative `when:`, while projection
  predicates must be removed or kept disabled because RUN_* activation is
  owned by `run_cost_projector`.

## v2026.05.31-ga.0 - 2026-05-31

### Summary

- Predictor upgrade SLICE_01 through SLICE_15 completed.
- HARDEN_01 through HARDEN_08 completed and merged to main.
- Legacy egress heuristic is replaced by predictor-backed budget projection and audit mirror columns.
- Python SDK 0.5.0 is the predictor-upgrade SDK line.

### Operator Highlights

- Production blockers #90, #137, #143, #145, #150, #160, #168, #169, and #171 are closed.
- Demo modes verified during hardening include `default`, `m1_benchmark_runaway_loop`, `multi_provider_usd`, `agent_real_anthropic`, and `plugin_c_synthetic`.
- Per-tenant SVID plugin identity is enforced for Strategy C production readiness.

### Migration Notes

- Ledger, canonical ingest, and control-plane migrations must be applied in documented order.
- Immutable audit data must be treated as forward-fix only; do not plan destructive rollback for canonical audit history.

### Helm / Config Notes

- Production values must reference Kubernetes Secrets for database URLs and signing material.
- Strategy C production deployments require per-tenant SVID bindings unless an explicit legacy global-cert opt-in is used.
- Demo profile remains separate from production validation values.

### Security Notes

- Database URLs are expected to come from Kubernetes Secret references in production Helm.
- Container security baseline remains required: non-root user, read-only root filesystem, no privilege escalation, and dropped capabilities.
- Supply-chain signing, SBOM, and vulnerability scan gates are owned by GA_09.

### Rollback / Forward-Fix Notes

- Release rollback must follow migration classification from GA_04 once available.
- Canonical audit history is append-only; use forward-fix for audit-chain data corrections.
- If Strategy C plugin onboarding fails, disable the affected binding or fall back according to the documented Strategy B path rather than weakening tenant SVID validation.
