# @spendguard/botpress-integration

Botpress 0.7 Integration SDK package that wires Agentic SpendGuard's
runtime safety layer into every Botpress bot's AI generation lifecycle.

The `beforeAiGeneration` hook reserves projected spend with the SpendGuard
sidecar HTTP companion **before** Botpress dispatches the upstream model
HTTP. The `afterAiGeneration` hook commits real usage end-of-call. DENY
and DEGRADE verdicts throw Botpress `RuntimeError` with a stable `code`
field (`BUDGET_DENIED`, `BUDGET_DEGRADED`, `BUDGET_CONFIG`).

Fail-closed by default; dev escape via `SPENDGUARD_BOTPRESS_FAIL_OPEN=1`.

## Install

```bash
pnpm add @spendguard/botpress-integration @spendguard/sdk @botpress/sdk
botpress integrations push
```

## Configuration

The integration's Zod schema is the source of truth at install time;
configure via the Botpress Studio integration form:

| Field                          | Required | Description                                                                |
| ------------------------------ | -------- | -------------------------------------------------------------------------- |
| `sidecarUrl`                   | yes      | HTTP companion URL (e.g. `http://sidecar:8443`).                            |
| `spendguardBudgetId`           | yes      | UUID of the SpendGuard budget to charge.                                    |
| `spendguardWindowInstanceId`   | yes      | UUID of the SpendGuard window instance.                                     |
| `upstreamProvider`             | yes      | One of `openai`, `anthropic`, `bedrock`.                                    |
| `tenantId`                     | yes      | Operator tenant identifier.                                                  |
| `tlsCertPath` / `tlsKeyPath` / `tlsRootCaPath` | no | mTLS material paths when the sidecar enforces client-cert auth. |

Saving the configuration form triggers `validateConfiguration`, which
issues a 1-token reserve+release probe against the sidecar. A successful
form save means the SpendGuard wiring is end-to-end live.

## Lifecycle invariants

| ID     | Invariant                                                                                                       |
| ------ | --------------------------------------------------------------------------------------------------------------- |
| INV-1  | DENY never hits the upstream provider.                                                                          |
| INV-2  | Pre-call reservation precedes upstream HTTP.                                                                    |
| INV-3  | Fail-closed default — `SPENDGUARD_BOTPRESS_FAIL_OPEN=1` is the only escape.                                     |
| INV-4  | `validateConfiguration` exercises the full reserve+release path.                                                |
| INV-5  | End-of-hook commit uses real `event.payload.usage` when present; estimator fallback otherwise (WARN logged).    |
| INV-6  | No mutation of operator credentials in logs (no `sidecarUrl` / `tlsKeyPath` substring).                         |
| INV-7  | Idempotency across retries — same `(botId, conversationId, runId, retry)` dedupes sidecar-side.                 |
| INV-8  | Integration does NOT mutate Botpress core; the dist bundle is mounted read-only.                                |
| INV-9  | No SDK / D09 HTTP companion drift; D32 is purely additive.                                                       |
| INV-10 | Hook re-entrancy safety — concurrent calls get distinct handles.                                                |

## Spec

See `docs/specs/coverage/D32_botpress/` in the [agentic-spendguard](https://github.com/m24927605/agentic-spendguard) repository for the design, implementation, review standards, tests and acceptance documents.

## License

Apache-2.0.
