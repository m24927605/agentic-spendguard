# SpendGuard × Mastra Processor demo runner (COV_D38_05)

Runnable Node example for the **dedicated Mastra adapter**
[`@spendguard/mastra`](../../sdk/typescript-mastra/): a real `@mastra/core`
`Agent` with the `SpendGuardProcessor` mounted via `inputProcessors`
(reserve + SUCCESS commit) and `outputProcessors` (backstop commit), driven
against the SpendGuard sidecar UDS and the in-network counting-stub
(mock OpenAI `/v1/chat/completions` provider).

## The 3 steps (design.md §10, LOCKED)

| Step | What happens | Proof |
|---|---|---|
| 1 ALLOW | `agent.generate(...)` small prompt within budget | counting-stub `/_count` +1; exactly 1 reserve + 1 SUCCESS commit |
| 2 DENY | second `SpendGuardProcessor` whose `claimEstimator` projects `2_000_000_000` atomic (> the seeded 1B hard cap) | step aborts pre-call; `/_count` **UNCHANGED** (zero provider HTTP — live fail-closed); no commit |
| 3 STREAM | `agent.stream(...)` drained | one reserve at step open + one commit after stream end; `/_count` +1 |

Success line (LOCKED spelling — the CI grep depends on it):

```
[demo] mastra_processor ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

## V6 pin — why the model is an explicit instance, not a router string

`[VERIFY-AT-IMPL: V6]` pinned **NO** against `@mastra/core` 1.41.0: the
model-router string `"openai/gpt-4o-mini"` resolves to the vendored
`createOpenAI(...).responses(modelId)` — `OPENAI_BASE_URL` *is* honored for
the base URL, but the call speaks the OpenAI **Responses API**
(`POST {base}/responses`), which the LOCKED-verbatim counting-stub
(chat/completions only) cannot serve. Per design §10/§12 the runner uses the
LOCKED explicit-instance fallback (a counting-stub-backed `LanguageModelV2`);
the router-string Processor mount is separately proven by TP-22 in
`sdk/typescript-mastra/tests/mastraIntegration.test.ts`.

## Running

The supported path is the demo overlay (brings up postgres + sidecar +
ledger + counting-stub + this runner):

```sh
make demo-down                      # ALWAYS: stale volumes → IDEMPOTENCY_CONFLICT
make demo-up DEMO_MODE=mastra_processor
make -C deploy/demo demo-verify-mastra-processor
make demo-down
```

Direct invocation (inside the runner container, or any host that can reach
the sidecar UDS + counting stub):

```sh
npm install
node index.mjs            # all 3 steps
npm run start:allow       # single step variants
npm run start:deny
npm run start:stream
```

### Environment

| Var | Demo value |
|---|---|
| `SPENDGUARD_SIDECAR_UDS` | `/var/run/spendguard/adapter.sock` |
| `SPENDGUARD_TENANT_ID` | `00000000-0000-4000-8000-000000000001` |
| `SPENDGUARD_BUDGET_ID` | `44444444-4444-4444-8444-444444444444` |
| `SPENDGUARD_WINDOW_INSTANCE_ID` | `55555555-5555-4555-8555-555555555555` |
| `SPENDGUARD_UNIT_ID` | `66666666-6666-4666-8666-666666666666` |
| `SPENDGUARD_COUNTING_STUB_URL` | `http://counting-stub:8765` |
| `OPENAI_BASE_URL` | `http://counting-stub:8765/v1` |

`unitId` rides `SpendGuardProcessorOptions.unitId` (HARDEN_D05_UR day-1
threading); `windowInstanceId` rides the demo `claimEstimator`'s claims
(HARDEN_D05_WI — estimator claims forward verbatim onto the reserve wire).

Node `>=22.13.0` required (`@mastra/core` floor — the demo runner image is
`node:22.13-bookworm-slim`, NOT the node:20.10 base the sibling overlays use).
