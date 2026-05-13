# Pricing & USD-denominated budgets

A Agentic SpendGuard tenant can spend a single USD budget across multiple
LLM providers (OpenAI, Anthropic, Bedrock, Azure OpenAI, Gemini).
The flow:

1. **Pricing table** — `pricing_table` in canonical DB stores
   `(provider, model, token_kind) → $/1M tokens`. Loaded from
   `deploy/demo/init/pricing/seed.yaml` at compose-up time.
2. **Pricing freeze** — bundle build picks the latest `pricing_version`
   and embeds the snapshot hash in `contract_bundle.metadata.json`.
   Sidecar reads it at startup; production deploys validate via
   cosign.
3. **Adapter conversion** — adapter computes µUSD per call:
   `(input_tokens × $/1M_input + output_tokens × $/1M_output) × 10^6`.
4. **Sidecar reserve** — adapter submits `BudgetClaim(amount_atomic=N µUSD)`.
   Sidecar reserves under the `usd_micros` unit (scale=6).
5. **Sidecar commit** — adapter sees the actual provider response,
   recomputes µUSD, sidecar commits via CommitEstimated.

The `multi_provider_usd` demo mode shows this end-to-end with real
OpenAI + Anthropic in the same session against a shared USD budget.

See [pricing.py](https://github.com/m24927605/agentic-flow-cost-evaluation/blob/main/sdk/python/src/spendguard/pricing.py)
for the helper that does the conversion.
