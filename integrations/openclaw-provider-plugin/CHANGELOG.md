# Changelog

## 0.1.0-pre

- Ship D40b OpenClaw provider plugin adapter:
  - `createSpendGuardOpenClawProvider(upstream, options)` public factory.
  - Pre-dispatch reserve at OpenClaw's `wrapStreamFn(ctx)` provider hook.
  - Fail-closed DENY and sidecar-unavailable behavior; no fail-open option.
  - SUCCESS, PROVIDER_ERROR, CLIENT_TIMEOUT, RUN_ABORTED, and async-stream
    settlement paths.
  - Day-1 `unitId`, `windowInstanceId`, and pricing freeze threading.
  - Live `DEMO_MODE=openclaw_provider_plugin` gate proving ALLOW + DENY +
    STREAM + PROVIDER_ERROR against a local counting stub.
- Add package skeleton for `COV_D40B_01_plugin_package_init`.
- Pin OpenClaw provider registration surface to `openclaw@2026.6.2`.
