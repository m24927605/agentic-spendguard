---
title: "計價與以 USD 為單位的預算"
---

Agentic SpendGuard 的 tenant 可以拿同一筆 USD 預算,跨多家
LLM provider(OpenAI、Anthropic、Bedrock、Azure OpenAI、Gemini)一起花。
流程是這樣:

1. **計價表(Pricing table)** — canonical DB 裡的 `pricing_table` 存的是
   `(provider, model, token_kind) → $/1M tokens`。compose-up 的時候從
   `deploy/demo/init/pricing/seed.yaml` 載進來。
2. **計價凍結(Pricing freeze)** — build bundle 時會挑最新的 `pricing_version`,
   再把那份 snapshot hash 嵌進 `contract_bundle.metadata.json`。
   sidecar 啟動時讀它;production 部署則透過 cosign 驗章。
3. **Adapter 換算** — adapter 算出每次呼叫的 µUSD:
   `(input_tokens × $/1M_input + output_tokens × $/1M_output) × 10^6`。
4. **Sidecar reserve** — adapter 送出 `BudgetClaim(amount_atomic=N µUSD)`,
   sidecar 就用 `usd_micros` 這個 unit(scale=6)做預扣。
5. **Sidecar commit** — adapter 拿到 provider 真正的回應後,重算一次 µUSD,
   sidecar 再透過 CommitEstimated 把帳結掉。

`multi_provider_usd` 這個 demo 模式會把整段 end-to-end 跑給你看 —— 在同一個
session 裡,用真的 OpenAI + Anthropic 一起花同一筆共用 USD 預算。

做換算的 helper 可以看
[pricing.py](https://github.com/m24927605/agentic-spendguard/blob/main/sdk/python/src/spendguard/pricing.py)。
