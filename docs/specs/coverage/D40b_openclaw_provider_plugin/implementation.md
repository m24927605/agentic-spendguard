# D40b - Implementation

## 1. File layout

```text
integrations/openclaw-provider-plugin/
  package.json
  tsconfig.json
  tsup.config.ts
  README.md
  CHANGELOG.md
  LICENSE_NOTICES.md
  src/
    index.ts
    provider.ts
    options.ts
    identity.ts
    flatten.ts
    usage.ts
    errors.ts
  tests/
    provider.test.ts
    failclosed.test.ts
    identity.test.ts
    streaming.test.ts
    hashReuse.test.ts
deploy/demo/openclaw_provider_plugin/
  docker-compose.yaml
  openclaw.config.json
  runner.mjs
deploy/demo/verify_step_openclaw_provider_plugin.sql
docs/site-v2/src/content/docs/docs/integrations/openclaw-provider-plugin.mdx
```

If the pinned OpenClaw API requires a different tree name, record a dated amendment in `design.md` before moving files.

## 2. Package constraints

- ESM-only.
- Node floor follows the pinned OpenClaw runtime, not a guessed repo-wide floor.
- Peer dependency on the pinned OpenClaw package range from `OB-V1`.
- `@spendguard/sdk` is a peer/workspace dependency.
- No local hash library.
- Bundle budget: 50 KB minified, excluding peers.

## 3. Core modules

| Module | Responsibility |
|---|---|
| `options.ts` | Locked options surface and validation. |
| `identity.ts` | Delegates to `@spendguard/sdk` ID helpers. |
| `flatten.ts` | Deterministic prompt/message text extraction from OpenClaw request shapes. |
| `provider.ts` | Wrapper factory, reserve, upstream dispatch, commit/release settlement. |
| `usage.ts` | Usage extraction for non-streaming and streaming responses. |
| `errors.ts` | Typed adapter errors and substrate error re-exports. |

## 4. Provider wrapper skeleton

```ts
export function createSpendGuardOpenClawProvider(upstream, options) {
  const opts = validateOptions(options);
  return {
    ...upstream,
    async generate(request, context) {
      const prepared = prepareOpenClawCall(request, context, opts);
      const decision = await opts.client.reserve(prepared.reserveRequest);
      const pending = pendingFromDecision(prepared, decision);
      try {
        const response = await upstream.generate(request, context);
        await commitSuccess(opts.client, pending, response);
        return response;
      } catch (err) {
        await commitFailure(opts.client, pending, err);
        throw err;
      }
    },
    async stream(request, context) {
      // Same reserve first. The stream wrapper commits once at terminal event.
    },
  };
}
```

The implementation must use exact OpenClaw method names pinned by `OB-V1`. The skeleton is a control-flow contract, not a claim about upstream type names.

## 5. Demo mode

`DEMO_MODE=openclaw_provider_plugin` boots:

- Postgres/ledger/canonical ingest/sidecar.
- OpenClaw with the plugin installed/configured.
- Counting OpenAI-compatible stub.
- Runner issuing ALLOW, DENY, STREAM, and provider-error calls.

Locked success line:

```text
[demo] openclaw_provider_plugin ALL 4 steps PASS (ALLOW + DENY + STREAM + PROVIDER_ERROR)
```

## 6. Slice to file map

| Slice | Files |
|---|---|
| `COV_D40B_01_plugin_package_init` | package skeleton, OpenClaw version pin, `OB-V1`/`OB-V2`. |
| `COV_D40B_02_provider_wrapper_reserve` | `options.ts`, `identity.ts`, `flatten.ts`, reserve path tests. |
| `COV_D40B_03_commit_failure_streaming` | `usage.ts`, commit/failure/stream settlement. |
| `COV_D40B_04_failclosed_tests` | full test matrix, hash-reuse tests, bundle gates. |
| `COV_D40B_05_openclaw_plugin_demo` | demo overlay, runner, verify SQL, Makefile. |
| `COV_D40B_06_docs_publish` | docs page, README, CHANGELOG, memory. |
