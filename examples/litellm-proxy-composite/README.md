# LiteLLM proxy + SpendGuard — runnable example

> **Status: first-party reference example.** Drops a SpendGuard
> `CustomLogger` callback into a stock LiteLLM proxy so every
> `POST /v1/chat/completions` reserves against a budget before the
> upstream provider is hit. No changes to your app code beyond
> pointing at `http://localhost:4000`.

This is the parallel of `deploy/demo/demo/run_demo.py::run_litellm_real_mode`
extracted into a standalone, copy-pasteable directory so contributors
can try the integration without spinning up the full demo Docker stack.

## What this proves

Two hard invariants:

1. **SpendGuard DENY ⇒ the upstream provider is NEVER invoked.** If
   the budget is exhausted (or the operator's resolver returns None),
   the LiteLLM proxy returns HTTP 403 / 503 / 500 (per error class)
   and the request never leaves the proxy.
2. **End-of-stream reconciliation commits real usage**, not the
   estimator worst-case. The post-call hook reads
   `response.usage.completion_tokens` from the provider response and
   emits an `INVOICE_COMMITTED` event with the real amount.

## Quickstart

```bash
cd examples/litellm-proxy-composite
docker compose up --build           # ~10 min cold (Rust builds)
# In another terminal:
python app.py                        # exercises ALLOW + DENY + STREAM
```

Expected output:

```
[app] (1) ALLOW: HTTP 200 completion_tokens=7
[app] (2) DENY: HTTP 403 reasons=['BUDGET_EXHAUSTED', ...]
[app] (3) STREAM: HTTP 200 (counter +1)
[app] ALL 3 steps PASS
```

## Topology

```
┌─────────────────────────────────────────────────────────────┐
│  app.py (your code — direct httpx to LiteLLM proxy)        │
│         POST http://localhost:4000/v1/chat/completions      │
└─────────────────────────────┬──────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  litellm-proxy container                                    │
│    proxy_config.yaml registers:                             │
│      litellm_settings.callbacks =                           │
│        [spendguard_litellm_proxy_callback.handler_instance] │
│                                                             │
│    On every request:                                        │
│      1. async_pre_call_hook → resolves binding,             │
│         estimates, reserves via SpendGuard sidecar.         │
│      2. Provider call (counting stub for the example).      │
│      3. async_log_success_event → commits real usage.       │
│      4. async_log_failure_event → releases on error.        │
└─────────────────────────────┬──────────────────────────────┘
                              │ Unix domain socket
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  spendguard-sidecar container (Rust)                        │
│    • contract DSL evaluator (hard-cap, approval, deny rules)│
│    • per-pod fencing lease                                  │
└─────────────────────────────┬──────────────────────────────┘
                              │ mTLS gRPC
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Postgres + ledger service                                  │
│    • Stripe-style auth → capture                            │
│    • signed append-only audit chain                         │
└─────────────────────────────────────────────────────────────┘
```

## What you need to fork

To deploy this against your own production LiteLLM proxy, you fork
**ONE file**:

- `spendguard_litellm_proxy_callback.py` — the operator template.
  ~120 LOC. Strip any `spendguard_test_fail_mode` /
  `spendguard_estimate_override` branches that the in-tree demo
  callback uses (they exist only to drive the deny-mode demo). The
  template here is already stripped — use as-is.

You write:

- Your real `_resolve(ctx)` — return the `BudgetBinding` for the
  caller's team. Inspect `ctx.user_api_key_dict.team_id` (LiteLLM's
  virtual-key auth populates this).
- Your real `_estimate(ctx)` — worst-case pre-call cost from your
  pricing table.
- Your real `_reconcile(ctx, response_obj)` — exact cost from
  `response.usage`.

See [`PROXY_RECIPE.md`](../../docs/specs/litellm-integration/PROXY_RECIPE.md)
§2 for the full operator-facing template.

## File listing

| File | Purpose | LOC |
|---|---|---|
| `README.md` | this file | 180 |
| `docker-compose.yml` | 3-service compose | 100 |
| `proxy_config.yaml` | LiteLLM proxy config | 30 |
| `spendguard_litellm_proxy_callback.py` | operator callback (stripped) | 120 |
| `app.py` | httpx caller (your app) | 130 |
| `requirements.txt` | app deps | 5 |

## Env vars

All set via `docker-compose.yml`. Override at compose time if needed:

| Var | Default | Purpose |
|---|---|---|
| `SPENDGUARD_SIDECAR_UDS` | `/var/run/spendguard/adapter.sock` | UDS path inside containers |
| `SPENDGUARD_TENANT_ID` | demo UUID | tenant scope |
| `SPENDGUARD_BUDGET_ID` | demo UUID | budget the resolver routes to |
| `SPENDGUARD_PRICING_VERSION` | `v1` | pricing-table version pin |
| `LITELLM_MASTER_KEY` | `sk-demo-key` | proxy master key (rotate for prod) |
| `OPENAI_API_KEY` | not required | counting stub doesn't validate |

## Limitations

- Single-tenant: the example wires one team/budget. Multi-tenant
  dispatch via `team_id` is documented in PROXY_RECIPE.md §2.2
  but not exercised here.
- Counting stub upstream: the example uses an in-container HTTP stub
  instead of real OpenAI/Anthropic. To swap in real providers, edit
  `proxy_config.yaml`'s `model_list[*].api_base` and set
  `OPENAI_API_KEY` in the environment.
- No streaming reconcile in the demo's stream step: the counting stub
  returns a non-streaming response; the SDK's
  `_async_log_success_streaming` is exercised by the full demo
  (`make demo-up DEMO_MODE=litellm_real`) instead.

## Tear-down

```bash
docker compose down -v               # removes volumes (Postgres ledger reset)
```

## Related

- [PROXY_RECIPE.md](../../docs/specs/litellm-integration/PROXY_RECIPE.md) — operator deployment recipe
- [docs/site/docs/integrations/litellm.md](../../docs/site/docs/integrations/litellm.md) — public docs
- Other examples: [`openai-agents-composite/`](../openai-agents-composite/)
