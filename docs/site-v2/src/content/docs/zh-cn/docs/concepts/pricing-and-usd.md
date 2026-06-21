---
title: "定价与以 USD 计价的预算"
---

Agentic SpendGuard 的 tenant 可以在多个 LLM provider 之间
共用同一份 USD 预算(OpenAI、Anthropic、Bedrock、Azure OpenAI、Gemini)。
整个流程如下:

1. **定价表** — canonical DB 里的 `pricing_table` 存的是
   `(provider, model, token_kind) → $/1M tokens`。compose-up 时从
   `deploy/demo/init/pricing/seed.yaml` 加载。
2. **定价冻结** — bundle 构建时挑选最新的 `pricing_version`,
   并把 snapshot hash 嵌入 `contract_bundle.metadata.json`。
   sidecar 启动时读取它;生产环境部署通过
   cosign 校验。
3. **Adapter 换算** — adapter 为每次 call 算出 µUSD:
   `(input_tokens × $/1M_input + output_tokens × $/1M_output) × 10^6`。
4. **Sidecar 预留** — adapter 提交 `BudgetClaim(amount_atomic=N µUSD)`,
   sidecar 在 `usd_micros` 这个 unit 下做预留(scale=6)。
5. **Sidecar 提交** — adapter 拿到 provider 的实际响应后,
   重新算一遍 µUSD,sidecar 通过 CommitEstimated 提交。

`multi_provider_usd` 这个 demo 模式端到端演示了这套流程:同一个 session 里
真实调用 OpenAI + Anthropic,花的是同一份共享 USD 预算。

负责换算的 helper 见 [pricing.py](https://github.com/m24927605/agentic-spendguard/blob/main/sdk/python/src/spendguard/pricing.py)。
