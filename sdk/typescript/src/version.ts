// SpendGuard SDK — version constant.
//
// Hand-edited per release in v0.1.x. The publish pipeline
// (`.github/workflows/sdk-ts-publish.yml`, COV_S05_10) asserts equality
// between this constant and `package.json#version` before publishing
// (`scripts/version-check.sh`).
//
// Bump procedure (per CHANGELOG.md release):
//   1. Bump `package.json#version`.
//   2. Bump this constant to match.
//   3. Add a CHANGELOG.md entry.
//   4. Tag `ts-sdk-v<version>` and push; CI verifies parity and publishes.

/**
 * The published `@spendguard/sdk` version. Mirror of the Python SDK
 * `spendguard.__version__` (which lives at `0.5.1` on PyPI as of 2026-05-31).
 *
 * SLICE 10 (`COV_S05_10`) wires the publish pipeline; `0.1.0` is the first
 * publishable release.
 */
export const VERSION = "0.1.0" as const;
