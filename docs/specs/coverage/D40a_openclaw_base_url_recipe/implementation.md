# D40a - Implementation

Implementation plan for the OpenClaw base-URL recipe. This deliverable is docs/demo plus a narrow egress-proxy test override; it adds no SDK, proto, ledger, or OpenClaw adapter behavior.

## 1. File layout

```text
docs/site-v2/src/content/docs/docs/drop-in/openclaw.mdx
examples/openclaw-base-url/
  README.md
  openclaw.config.example.json
deploy/demo/openclaw_base_url/
  docker-compose.yaml
  openclaw.config.json
  runner.mjs
  README.md
deploy/demo/init/bundles/generate.sh
deploy/demo/verify_step_openclaw_base_url.sql
deploy/demo/Makefile
services/egress_proxy/src/forward.rs
README.md
CHANGELOG.md
docs/internal/slices/COV_D40A_01_openclaw_recipe_smoke.md
docs/internal/slices/COV_D40A_02_openclaw_docs_publish.md
```

## 2. Docs page skeleton

The page structure is locked:

1. What this covers: base-URL recipe only.
2. Prerequisites: OpenClaw, SpendGuard egress proxy, model credential.
3. Configure OpenClaw provider:
   - Base URL: `http://localhost:9000/v1`
   - API key: any non-empty local value or operator-managed key, depending on `OA-V5`
   - Model: demo default `gpt-4o-mini`
   - Request adapter: OpenAI-compatible chat completions, exact name pinned by `OA-V1`
4. Verify locally:
   - `make demo-down`
   - `make demo-up DEMO_MODE=openclaw_base_url`
   - `make -C deploy/demo demo-verify-openclaw-base-url`
5. Deployment notes:
   - local desktop
   - docker compose
   - hosted proxy
6. Gotchas:
   - trailing `/v1`
   - request adapter mismatch
   - inbound API key is not the upstream key
   - provider plugin work is D40b, not D40a

All code blocks in `.mdx` that contain braces must use the repo's Astro/Starlight escape pattern if required by the surrounding component.

## 3. Demo runner

`deploy/demo/openclaw_base_url/runner.mjs` performs three calls through OpenClaw:

1. ALLOW: small prompt, non-streaming.
2. DENY: normal OpenAI-compatible request with `max_tokens: 256` against the
   D40a overlay's demo hard-cap threshold, must not hit counting stub.
3. STREAM: streaming response, commits at stream close.

The runner prints the locked success line only after all assertions pass:

```text
[demo] openclaw_base_url ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

The runner must capture the counting stub call count before and after DENY and fail if it changes.

## 4. Demo compose

The overlay must source the same runtime budget bundle pattern as recent TS demos:

- `bundles-data` mounted where the runner can read runtime env.
- `SPENDGUARD_TENANT_ID`, `SPENDGUARD_BUDGET_ID`, `SPENDGUARD_WINDOW_INSTANCE_ID`, `SPENDGUARD_UNIT_ID`, and pricing tuple env vars passed to the runner or proxy path.
- A local counting stub is the upstream provider. No live provider key is required for the hard gate.
- Node runner image is `node:22` unless OpenClaw requires a higher floor pinned by `OA-V3`.

## 5. Verify SQL

`verify_step_openclaw_base_url.sql` mirrors the D38/D39 hard-gate style:

- prefix every assertion block with `COV_D40A_GATE`
- reserve count >= 2
- commit_estimated count >= 2
- denied decision count >= 1
- earliest reserve precedes earliest provider outcome
- canonical decision/outcome rows exist
- outbox closure count matches expected emitted events

Do not soften row-count assertions to warnings.

## 6. Slice to file map

| Slice | Files |
|---|---|
| `COV_D40A_01_openclaw_recipe_smoke` | docs page, example config, demo overlay, runner, verify SQL, Makefile branch. |
| `COV_D40A_02_openclaw_docs_publish` | README row, CHANGELOG entry, docs cross-links, memory writeback. |

## 7. Anti-scope

- No `sdk/` package.
- No OpenClaw plugin code.
- No frozen fixture edits.
- No upstream PR.
- No changes to unrelated demo modes.

## 8. 2026-06-12 implementation amendment

Per `design.md` §11, slice 1 may add the narrow
`SPENDGUARD_PROXY_OPENAI_BASE_URL` override to
`services/egress_proxy/src/forward.rs` so the D40a demo can route the proxy's
upstream call to the local counting stub without a live OpenAI key. The default
unset behavior must remain byte-for-byte equivalent to the existing routing
table targets and must be pinned by a unit test.

The `openclaw_base_url` runner validates a committed OpenClaw config fixture
against the pinned OpenClaw config surface and emits OpenAI-compatible calls
through the configured `baseUrl`. It must not claim that the full OpenClaw
gateway binary ran inside the demo stack.

The shared demo contract generator may accept
`DEMO_HARD_CAP_CLAIM_AMOUNT_ATOMIC_GT`; the default stays `1000000000`.
`deploy/demo/openclaw_base_url/docker-compose.yaml` sets it to `100` only for
this overlay, so ALLOW/STREAM stay below the cap while DENY's `max_tokens: 256`
is stopped before provider dispatch. The generator validates that the env var
is a non-negative integer before writing `contract.yaml`.
