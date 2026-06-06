// SpendGuard SDK — version constant.
//
// Hand-edited per release in v0.1.x. SLICE 10 (publish pipeline) wires a
// build-time generator that derives this from `package.json` at `tsup` time;
// in v0.1.x we hand-edit to avoid a build dependency on the generator.
//
// TODO(SLICE_10 publish pipeline): SLICE 10 reconciles `VERSION` here with
// `package.json#version` before the first npm publish. Until then the two
// can drift — `package.json` currently sits at `0.0.0` (PRE-RELEASE
// placeholder per the substrate's `"private": true` declaration); this file
// carries `0.1.0-pre` because it is the value the wire `sdkVersion` field
// reports and adapters surface to the sidecar at handshake. SLICE 10's
// publish pipeline (`COV_S05_10_d05_publish_pipeline`) is the single
// reconciliation point: at tag time it (a) bumps `package.json` to
// `0.1.0`, (b) bumps this constant to `0.1.0`, (c) asserts equality in
// CI. R1 minor m-4 flagged the drift; deferred to SLICE 10 per the slice
// plan boundary.

/**
 * The published `@spendguard/sdk` version. Mirror of the Python SDK
 * `spendguard.__version__` (which lives at `0.5.1` on PyPI as of 2026-05-31).
 *
 * SLICE 3 ships `0.1.0-pre` because the public surface is in flux through
 * SLICE 4-10; SLICE 10 bumps to `0.1.0` for first publish.
 */
export const VERSION = "0.1.0-pre" as const;
