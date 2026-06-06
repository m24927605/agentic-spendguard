# Envoy AI Gateway v0.6 reference fixtures (vendored)

## Provenance

| Field | Value |
|---|---|
| Upstream repo | https://github.com/envoyproxy/ai-gateway |
| Tag | `v0.6.0` (published 2026-05-05) |
| Tag commit SHA | `a82fcf515901f38d6d8431ecbdbd1218735d2be1` |
| Refresh date | 2026-06-07 |
| License | Apache-2.0 (preserved per upstream `SPDX-License-Identifier` headers) |

## Files

- `token_counting.yaml` — verbatim copy of upstream
  `examples/basic/basic.yaml`. Represents the "happy-path" v0.6
  AIGatewayRoute that the ExtProc service must conform to for the
  token-counting + decision flow. We rename it locally to match the
  SLICE 5 spec naming convention (D01 implementation §8) so the test
  loader signatures stay aligned with `docs/specs/coverage/D01_envoy_extproc/`.
- `budget.yaml` — verbatim copy of upstream
  `examples/token_ratelimit/token_ratelimit.yaml`. Represents the v0.6
  reference for token-budget enforcement (per-tenant input/output/total
  token limits via `llmRequestCosts`). Renamed to match SLICE 5 spec
  naming. **Deviation #1**: upstream v0.6 does not publish a literal
  `budget.yaml` — the closest equivalent is `token_ratelimit.yaml`,
  which exercises the same wire-shape boundary that SpendGuard's budget
  enforcement does (per-tenant token cost surfaced in the response
  metadata namespace `io.envoy.ai_gateway`). The conformance loader
  treats the two as logical synonyms.

## Refresh procedure

Manual refresh (preferred for v0.6 → v0.7):

```bash
./services/envoy_extproc/scripts/refresh_fixtures.sh
```

The script fetches the latest tag, downloads the upstream YAML files,
diffs them against the committed fixtures, and prompts before
overwriting. See `scripts/refresh_fixtures.sh` for the SLICE 5 cadence.

## What the conformance tests do

The vendored YAML files are NOT loaded directly into the test runtime
(they describe Kubernetes manifests, not ExtProc gRPC frames). Instead,
`tests/conformance.rs` constructs `ProcessingRequest` frames that mirror
what Envoy AI Gateway v0.6 emits when it processes a request matching
each manifest's route. The YAML files exist in-tree so reviewers can
verify the test frame shapes against the upstream contract by hand
(per review-standards §6.1: "Conformance fixture source / version
pinned").

## When to refresh

- Envoy AI Gateway publishes a new minor (e.g. v0.7).
- A v0.6 patch release lands a backward-incompatible change to the
  `x-ai-eg-model` header or `io.envoy.ai_gateway` metadata namespace.
- Quarterly hygiene check (suggested cadence: end of each calendar
  quarter).

If the upstream tag changes, the test harness MUST be re-validated
against the new wire shape, NOT silently regenerated (review-standards
§6.2).
