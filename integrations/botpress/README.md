# @spendguard/botpress-integration

A Botpress **LLM-provider integration** that wires Agentic SpendGuard's runtime
budget guardrail in front of every LLM completion a Botpress bot makes.

Botpress invokes this integration's **`generateContent`** action whenever a bot
needs an LLM completion. `generateContent`:

1. **Reserves** projected spend with the SpendGuard sidecar HTTP companion
   **before** any upstream call.
2. **Forwards** the prompt to the configured upstream provider (OpenAI /
   Anthropic / Bedrock).
3. **Commits** the real token usage after the call returns.

DENY / DEGRADE / config / provider / commit failures fail the call with a
Botpress `RuntimeError`, after releasing any held reservation. The stable
SpendGuard code (`BUDGET_DENIED` / `BUDGET_DEGRADED` / `BUDGET_CONFIG`) is
carried in the RuntimeError's `metadata.spendguardCode` bag (the RuntimeError
numeric `code` field is reserved by `@botpress/client` and is read-only).

Fail-closed by default; dev escape via `SPENDGUARD_BOTPRESS_FAIL_OPEN=1`.

> Why an action and not a network proxy? Botpress's first-party OpenAI
> integration hardcodes its endpoint and ignores `HTTPS_PROXY`, so a
> network-layer interposer cannot gate LLM spend. Implementing the
> `generateContent` LLM-provider action is the supported gate point.

## Build / develop

```bash
pnpm install                       # workspace-aware (lockfile at repo root)
pnpm --filter @spendguard/sdk run build   # build the SDK workspace dependency
cd integrations/botpress
pnpm exec bp build                 # offline codegen -> .botpress/ typings
pnpm run typecheck                 # tsc --noEmit (+ tests tsconfig)
pnpm run build                     # tsup ESM + .d.ts
pnpm run test                      # vitest unit suite
```

`bp build` is offline and needs no Botpress Cloud login. It reads
`integration.definition.ts` and generates the `.botpress/` typings the runtime
imports as `import * as bp from '.botpress'` (`.botpress/` is gitignored and
regenerated in CI).

## Configuration

The configuration schema in `integration.definition.ts` is validated at install
time; configure via the Botpress integration form:

| Field                          | Required | Description                                                                |
| ------------------------------ | -------- | -------------------------------------------------------------------------- |
| `sidecarUrl`                   | yes      | HTTP companion URL (e.g. `http://sidecar:8443`; remote hosts must be https). |
| `spendguardBudgetId`           | yes      | UUID of the SpendGuard budget to charge.                                    |
| `spendguardWindowInstanceId`   | yes      | UUID of the SpendGuard window instance.                                     |
| `upstreamProvider`             | yes      | One of `openai`, `anthropic`, `bedrock`.                                    |
| `tenantId`                     | yes      | Operator tenant identifier.                                                  |
| `tlsCertPath` / `tlsKeyPath` / `tlsRootCaPath` | no | mTLS material paths when the sidecar enforces client-cert auth. |

The upstream provider API key is read from the deployment environment
(`OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / `BEDROCK_API_KEY`).

Installing the integration triggers `register` -> `validateConfiguration`,
which issues a 1-token reserve+release probe against the sidecar, so a
successful install means the SpendGuard wiring is end-to-end live.

## Lifecycle invariants

| ID     | Invariant                                                                                       |
| ------ | ----------------------------------------------------------------------------------------------- |
| INV-1  | DENY / DEGRADE / config error never reaches the upstream provider.                              |
| INV-2  | The reserve RPC completes before the upstream forward.                                          |
| INV-3  | Fail-closed default — `SPENDGUARD_BOTPRESS_FAIL_OPEN=1` is the only escape.                      |
| INV-4  | `validateConfiguration` (run on `register`) exercises the full reserve+release path.            |
| INV-5  | Commit uses the provider's real token usage; a commit failure releases the reservation.         |
| INV-6  | No operator credentials in logs (URLs are scheme+host redacted).                                |
| INV-7  | Idempotency via the D05 idempotency key derived from the call identity.                          |
| INV-8  | A provider-forward error releases the reservation (no dangling reservation).                     |

## Scope

v1 ships non-streaming, single-choice `generateContent` + `listLanguageModels`.
The integration declares these as native actions rather than via the public
Botpress `llm` interface (`.extend(llm)`): the llm interface's schemas live in
`@botpress/common`, which is not published to npm, and `bp add llm` requires
Botpress Cloud auth. Native actions keep the build fully offline while
producing a correctly SDK-typed LLM-provider surface.

## Spec

See `docs/specs/coverage/D32_botpress/` in the
[agentic-spendguard](https://github.com/m24927605/agentic-spendguard)
repository.

## License

Apache-2.0.
