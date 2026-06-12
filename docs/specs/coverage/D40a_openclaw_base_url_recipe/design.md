# D40a - OpenClaw base-URL recipe

**Status:** Spec - LOCKED 2026-06-12.
**Parent strategy:** [`framework-coverage-addendum-2026-06-10.md`](../../../strategy/framework-coverage-addendum-2026-06-10.md) §2.
**Pattern:** Pattern 2 - OpenAI-compatible base URL redirect.
**Owner sub-agent:** Technical Writer.
**Sibling deliverable:** [`D40b_openclaw_provider_plugin`](../D40b_openclaw_provider_plugin/design.md) owns the in-process plugin adapter. D40a is the durable floor and does not wait for D40b.

> **LOCKED design.md trumps slice docs.** Where a slice doc disagrees with this file, this file wins. Slice docs must quote the relevant sections verbatim and flag drift as a slice-author bug.

## 1. Problem

OpenClaw is a high-traffic local-first gateway and agent runtime candidate recorded in the addendum as D40 rank 1. Its durable SpendGuard entry point is not the in-process plugin surface; it is the custom provider base URL path. OpenClaw already supports provider configuration that can route OpenAI-compatible traffic to a configured base URL. SpendGuard already exposes an OpenAI-compatible egress proxy, so D40a can cover the common path without depending on OpenClaw plugin API stability.

The user-facing problem is configuration accuracy: operators need a short recipe that proves OpenClaw traffic goes through SpendGuard, not a generic "set OPENAI_BASE_URL" page. The recipe must show the OpenClaw-specific config keys, the SpendGuard proxy URL, the model adapter choice, and a live smoke that reserves, commits, and denies without modifying OpenClaw source.

## 2. Goals

1. Ship an OpenClaw drop-in recipe page under the docs site.
2. Add `examples/openclaw-base-url/` with a minimal OpenClaw config template and README.
3. Add demo mode `openclaw_base_url` that runs OpenClaw, SpendGuard egress proxy, and a counting upstream stub.
4. Prove three flows in the demo: ALLOW, DENY-before-provider, and STREAM.
5. Keep the demo fully local and reproducible: no live OpenAI key and no hosted SpendGuard dependency.
6. Promote the root README adapter/drop-in table row for OpenClaw only after the live demo gate passes.

## 3. Non-goals

- No OpenClaw provider plugin or hook implementation. That is D40b.
- No fork of OpenClaw.
- No hosted OpenRouter budget setup. The doc may mention it as a competitor workaround, but the verified path is SpendGuard egress proxy.
- No reverse engineering of proprietary provider internals.
- No k8s sidecar topology. D40a is local-first.
- No upstream OpenClaw PR.

## 4. Integration shape - LOCKED

OpenClaw is configured to send OpenAI-compatible chat traffic to:

```text
http://egress-proxy:9000/v1
```

or, outside compose:

```text
http://localhost:9000/v1
```

The trailing `/v1` is mandatory. The SpendGuard egress proxy forwards to the real upstream or, in the demo, to the counting stub. OpenClaw's inbound API key can be any non-empty string if the local proxy ignores inbound authorization and uses its own upstream credentials.

The recipe MUST state:

> D40a is a configuration recipe. SpendGuard enforcement happens in the egress proxy and sidecar before the provider call. OpenClaw is not modified, and no OpenClaw plugin is installed.

## 5. Demo topology - LOCKED

```text
openclaw-runner
  -> OpenClaw configured provider base URL
  -> spendguard egress-proxy
  -> sidecar over UDS/gRPC
  -> ledger + audit chain
  -> counting-openai-stub
```

The demo mode name is `openclaw_base_url`.

The locked success line is:

```text
[demo] openclaw_base_url ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

The DENY step must prove the upstream counting stub counter is unchanged across the denied call. A reviewer cannot sign off from SQL counts alone; the zero-provider-call proof is part of the gate.

## 6. File surfaces

| Path | Purpose |
|---|---|
| `docs/site-v2/src/content/docs/docs/drop-in/openclaw.mdx` | User-facing recipe page. |
| `examples/openclaw-base-url/` | Config template and local README. |
| `deploy/demo/openclaw_base_url/` | Compose overlay, runner, and local fixture config. |
| `deploy/demo/verify_step_openclaw_base_url.sql` | HARD SQL gate. |
| `deploy/demo/Makefile` | `DEMO_MODE=openclaw_base_url` branch and verify target. |
| `README.md` | Adapter/drop-in table row after live gate passes. |
| `CHANGELOG.md` | D40a entry after live gate passes. |

## 7. Safety and honesty locks

1. The doc must not claim plugin-level coverage. Use "base-URL recipe" and "egress proxy" wording.
2. The doc must not claim all OpenClaw providers are covered. It covers OpenAI-compatible request-adapter paths that can point at the SpendGuard proxy.
3. If OpenClaw config cannot be updated non-interactively at implementation time, the demo must use a committed fixture config and document the manual UI path separately. Do not fake a GUI operation.
4. Any exact OpenClaw config key names must be pinned at implementation time and recorded in the slice doc.
5. Existing `sdk/fixtures/cross-language/` frozen corpora are read-only for D40a.

## 8. VERIFY-AT-IMPL marker register

| Marker | Question to pin during implementation | Owning slice |
|---|---|---|
| `OA-V1` | Exact OpenClaw provider config keys for base URL, API key, model, and request adapter. | `COV_D40A_01_openclaw_recipe_smoke` |
| `OA-V2` | Whether OpenClaw can be configured fully by file/env in compose, or needs a runner-side bootstrap step. | `COV_D40A_01_openclaw_recipe_smoke` |
| `OA-V3` | Pinned OpenClaw package/image/version used by the demo. | `COV_D40A_01_openclaw_recipe_smoke` |
| `OA-V4` | Streaming response shape that the egress proxy sees from OpenClaw's OpenAI-compatible call. | `COV_D40A_01_openclaw_recipe_smoke` |
| `OA-V5` | Whether OpenClaw strips or rewrites inbound authorization before forwarding. | `COV_D40A_01_openclaw_recipe_smoke` |

No marker may remain unresolved when D40a closes.

## 9. Slice plan

| Slice | Title | Scope |
|---|---|---|
| `COV_D40A_01_openclaw_recipe_smoke` | Recipe + local demo smoke | Docs page, example config, demo mode, verify SQL. |
| `COV_D40A_02_openclaw_docs_publish` | Publish polish + closeout | README row, CHANGELOG, final docs links, memory entry. |

## 10. Definition of done

D40a is shipped when both slices are on main, the demo has physically run, `verify_step_openclaw_base_url.sql` is green, OpenClaw is present in the README table as base-URL recipe coverage, and `project_coverage_d40a_shipped.md` exists in memory.

## 11. Dated amendments

### 2026-06-12 - Slice 1 implementation pin for local deterministic demo

Primary OpenClaw sources at implementation time pin the durable config surface:
`models.providers.<provider>.baseUrl`, `apiKey`, `api: "openai-completions"`,
`models.providers.<provider>.models[]`, and `agents.defaults.model.primary`.
OpenClaw also publishes npm package `openclaw` version `2026.6.2`, Docker
runtime files, and `OPENCLAW_CONFIG_PATH` / `OPENCLAW_CONFIG_DIR` /
`OPENCLAW_STATE_DIR` environment variables.

This amendment is orchestrator-ratified for Slice 1 and supersedes §2 goal 3
and §5 topology where they imply the full OpenClaw gateway binary runs inside
the demo stack. The D40a hard gate remains local and keyless. To avoid a live
provider key, the demo uses a committed OpenClaw config fixture plus an
`openclaw-runner` that validates the fixture against the pinned OpenClaw config
surface and then emits OpenAI-compatible calls matching that provider shape.
This is not plugin coverage, not full gateway runtime proof, and not a claim
that the full OpenClaw gateway binary was embedded in the SpendGuard demo
stack. Full in-process OpenClaw plugin/runtime coverage remains D40b.

The egress proxy may use `SPENDGUARD_PROXY_OPENAI_BASE_URL` in demo/test
environments to send OpenAI-compatible upstream calls to a local counting stub.
When unset, the proxy continues to use the existing routing table targets such
as `https://api.openai.com/v1/chat/completions`; default production behavior is
unchanged.

The demo contract generator may read
`DEMO_HARD_CAP_CLAIM_AMOUNT_ATOMIC_GT`; its default remains
`1000000000`, preserving all existing demo modes. The D40a compose overlay
sets this value to `100` so the DENY step can use a normal
OpenAI-compatible request with `max_tokens: 256` to trigger the hard-cap rule.
This avoids relying on `X-SpendGuard-Estimated-Tokens` as a reserve amount:
the proxy treats that header as tokenizer input override only, and the
reservation amount still flows through the Strategy A context-window policy.
