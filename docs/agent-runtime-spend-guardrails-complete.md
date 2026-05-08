# Agent Runtime Spend Guardrails — 從市場研究到產品策略

> **完整版整合文件**  
> 日期：2026-05-06  
> 範圍：市場研究（80+ 產品、80+ 學術論文、10+ 開源專案）+ 產品策略（兩輪 codex 反饋後的最終版）

---

## 文件導讀

本文件分四部分：

| Part | 內容 | 何時讀 |
|---|---|---|
| **Part I** | 市場研究 — 事實基礎、能力矩陣、缺口分析 | 想了解業界現況 |
| **Part II** | 產品策略 — 收窄定位、T→L→C→D→E→P 閉環、Phase 路線圖 | 想了解該做什麼 |
| **Part III** | 策略演進記錄 — v1.0 → v1.1 → v1.2 的判斷歷程與教訓 | 想了解為什麼這麼做 |
| **Part IV** | 附錄 — 參考論文、產品 URLs、開源 repos | 需要 cite 證據 |

### 30 秒摘要

**研究問題**：是否存在真正完整的閉環 LLM Token & Cost Intelligence Platform？  
**答案**：不存在。市場分裂為 observability / gateway / GPU-infra 三孤島。  
**最接近的 5 個產品**：CAST AI、TensorZero、Galileo Agent Control、Portkey、Pydantic-AI — 各自完成 1-2 個閉環階段。  
**最大技術缺口**：output token prediction（學術已解但無人商品化）、mid-stream model switching（無人實作）、eval-gated auto-apply、cross-tenant learning。  
**產品結論**：做 **Agent Runtime Spend Guardrails — 三支柱閉環（Predict + Control + Optimize）**。**主動排除 Continuous Learning**（其難度屬合規上限，創造 ceiling 而非 moat；見 §22.4）。  
**Wedge**：跨 runtime 的 step-boundary policy decision engine。  
**Moat**：per-tenant **passive** data accumulation + Contract DSL switching cost + audit trail accumulation 三重複合。  
**不做**：dashboard / 更準的 attribution / 更便宜的 router / mid-stream switching / **任何 active continuous learning（含 cross-tenant federated）**。

---

# Part I：市場研究

## 1. 執行摘要

### 1.1 對核心問題的明確回答

> **是否存在真正完整的閉環 LLM Token & Cost Intelligence Platform？**

**答案：不存在。** 截至 2026 年 5 月，沒有任何商業產品、開源專案或學術系統實現完整閉環。市場分裂成三個彼此不對話的孤島：

1. **Observability / Allocation 派**（Langfuse、LangSmith、Helicone、Datadog、CloudZero、Vantage 等）— 只看不管
2. **Gateway / Routing 派**（Portkey、LiteLLM、TensorZero、Azure Foundry、Martian、Not Diamond 等）— 管但不預測
3. **GPU Infra 派**（NVIDIA Run:AI、CAST AI、Densify/Kubex、Anyscale）— 管硬體不管 token

### 1.2 最接近閉環的五個產品

| # | 產品 | 完整度 | 缺口 |
|---|---|---|---|
| 1 | **CAST AI** | GPU + LLM 雙層 auto-apply | 缺 eval-gated 品質保證、缺 cross-customer learning |
| 2 | **TensorZero** | in-gateway online bandit + GEPA pipeline | bandit 限 variant-level 而非 prompt-conditional；無 mid-stream 切換 |
| 3 | **Galileo Agent Control**（2026/3 開源） | runtime policy plane | 目標是治理非成本；無 output prediction |
| 4 | **Portkey + 自建 eval** | 最強 hard runtime budget enforcement | 無 prediction、無 recommend、無 learning |
| 5 | **Pydantic-AI + genai-prices + Logfire** | 唯一 OSS 第一級 `UsageLimits` 強制 | 無 prediction phase |

### 1.3 關鍵發現

- **Output Token Prediction 已被學術解決但無人商品化**：EGTP/PLP（ICLR 2026, arXiv:2602.11812）達 −29% MAE，但 vLLM、SGLang、TGI、TensorRT-LLM 全部未整合
- **`vllm-ltr`** 已存在 18 個月仍未 merge upstream；Q2 2026 vLLM roadmap 不包含 length predictor
- **Mid-stream 模型切換 = 0 個產品實作**（業界共識）
- **沒有產品做 reasoning token 預測**，但 reasoning models 是當下最大成本黑洞
- **Helicone 2026/3 被 Mintlify 收購進入 maintenance**，OSS 領導者退場、賽道空虛
- **FOCUS v1.2（2025/5）已支援 token-based 非貨幣 pricing units**，標準化基礎已就緒
- **State of FinOps 2026**：98% 從業者已將 AI spend 納入管理範疇；40% 大型公司每年 AI 支出 $10M+

---

## 2. 研究範圍與分類框架

### 2.1 嚴格區分的六種類型

研究堅持區分以下類型，避免被 marketing 詞彙混淆：

| 類型 | 定義 | 真實能力範圍 |
|---|---|---|
| **Observability** | 事後查看 token/cost/log | 純 dashboard，無控制 |
| **Gateway** | routing / retry / cache | 前置決策 + failover |
| **Optimization Engine** | 主動降低 token/cost | 自動套用優化 |
| **Prediction Engine** | request 前預測 token | output length forecast |
| **Runtime Adaptive System** | inference 中動態調整 | mid-stream switch / cap |
| **Recommendation System** | 自動提出優化建議 | 生成 specific, actionable advice |

**閉環平台必須同時具備 Prediction + Runtime Adaptive + Recommendation + Continuous Learning 四項。**

### 2.2 評估框架（4+1 維度）

#### A. Pre-Inference Prediction（事前預測）
- Input token prediction（簡單 — tokenizer 即可）
- **Output token prediction（困難 — 真關鍵）**
- Reasoning token prediction（最困難 — o1/R1 時代）
- Cost preview before request

#### B. Runtime Adaptive Control（事中控制）
- Adaptive max_tokens、Dynamic truncation
- **Mid-stream model switching**（業界 0 實作）
- Budget-aware enforcement
- CoT / reasoning depth control

#### C. Post-Inference Optimization（事後優化建議）
- Specific actionable recommendations（auto-applied vs human-reviewed 區分）
- Prompt compression、context reduction、model routing
- Token waste detection、ROI / cost-quality tradeoff

#### D. Continuous Learning（持續學習）— 已**主動移出產品範圍**（見 §11.5 與 §22.4）
- Online learning from production data — 不做
- Cross-customer federated learning — 不做
- Bandit / RL routing optimization — 不做（除 TensorZero 已有 variant-level，留作競品觀察）
- **保留：被動 per-tenant data accumulation 作為自然 by-product**（不是 active learning）

#### E. Closed Loop（綜合判定）❌ 完全沒有 / 🟡 partial / 🟢 partial 但最接近 / ✅ 完整閉環

> **本研究的「閉環」定義為四件事同時具備。但本產品策略主動把 Continuous Learning 移出 MVP 範圍** — 因為其難度屬合規 / 隱私 / GDPR / consent 範疇，創造的是**法律風險上限**而非 moat（見 §22.4）。本產品的閉環簡化為三件事：**Predict + Control + Optimize**。

### 2.3 研究方法

5 個並行 AI agent 各自獨立調查：
1. LLM Observability 平台（13 產品）
2. AI Gateway 與 Routing 系統（17 產品）
3. Token / Output Length Prediction 學術論文（30+ 篇）
4. 開源 Inference Optimization 工具（33 專案）
5. AI FinOps 與 Cost Intelligence 工具（25+ 產品）

每個 agent 接收獨立、自包含 prompt；強調區分 marketing claims 與 shipped reality；引用 URL 證據。

---

## 3. 市場地圖：五層架構

### Layer 1：Observability（事後查看）

| 產品 | 類型 | 開源 | 焦點 | 閉環 |
|---|---|---|---|---|
| Langfuse | OSS | ✅ MIT | tracing + cost dashboard | ❌ |
| Helicone | proxy + obs（**2026/3 maintenance**） | ✅ Apache | 被 Mintlify 收購 | 🟡 |
| LangSmith | tracing + eval | ❌ | LangChain 生態 | ❌ |
| Arize Phoenix / AX | obs + eval | ✅ / ❌ | Prompt Learning SDK（batch） | 🟡 |
| Braintrust | eval + obs | ❌ | **Loop**（AI 助理建議 prompt） | 🟡 |
| Maxim AI / Bifrost | hybrid + Go gateway | ✅ Bifrost | virtual-key budget 強制 | 🟡 |
| Keywords AI / Respan | hybrid | ❌ | 30+ 整合 | ❌ |
| **Galileo + Agent Control** | eval + runtime policy（**2026/3 開源**） | ✅ | **policy plane 控制 agent step** | 🟡 |
| OpenLIT | OTEL-native | ✅ Apache | 評估 + Rule Engine | ❌ |
| Datadog LLM Obs | APM | ❌ | 自身觀測成本 +40-200% | ❌ |
| New Relic / Dynatrace / Splunk | APM | ❌ | 純 dashboard | ❌ |
| WhyLabs / Fiddler / Arthur | safety/drift | mixed | 不關注 cost | ❌ |
| Honeycomb | tracing | ❌ | 通用 | ❌ |
| Lightstep | **2026/3 EOL** | — | — | — |

### Layer 2：Gateway / Router

| 產品 | 預測 | Runtime control | 閉環 |
|---|---|---|---|
| **Portkey** | input cost only | ✅ workspace + API key 美元/token 硬阻擋 | 🟡（最強控制） |
| **LiteLLM** | input only | ✅ 多層 budget 但僅 cumulative | ❌ |
| **TensorZero** | input only | ✅ DICL + bandit variant select | 🟡（最接近閉環） |
| **Kong AI** | input only | embedding-similarity routing | 🟡 |
| **Cloudflare AI Gateway** | input only | rule flow，反應式 budget | ❌ |
| **AWS Bedrock IPR** | per-(query,model) quality | within-family classifier only | 🟡 |
| **Azure Foundry Model Router** | per-prompt quality（trained classifier） | 22 模型跨家族 | 🟡 |
| Vertex AI | — | 無對應 router | ❌ |
| Martian | Model Mapping quality | batch 訓練 | 🟡 |
| Not Diamond | quality predictor | batch retrain | 🟡 |
| RouteLLM | 4 種 router 類型 | 靜態訓練；BaRP research | 🟡(research) |
| Unify | quality + 10-min live bench | trained scorer | 🟡 |
| OpenRouter Auto | （Not Diamond underneath） | 不可重現的路由 | 🟡 |
| Requesty / Vellum / Humanloop | 規則或 operator-defined | failover / rules | ❌ |
| Narev | ❌ | 推薦器，非 router | ❌ |

### Layer 3：AI FinOps / Cost Allocation

| 產品 | 領域 | Auto-apply | 閉環 |
|---|---|---|---|
| CloudZero AI / Vantage / Finout / Apptio | allocation | ❌ | ❌ |
| nOps for AI | K8s + LLM | ❌ | ❌ |
| **CAST AI** | K8s GPU + AI Enabler LLM 路由 | **✅ 雙層 auto-apply** | 🟢(closest) |
| Spot.io / Flexera | infra | ✅ infra | ✅(infra only) |
| Densify / Kubex | GPU MIG + MCP server | optional | 🟡 |
| Kubecost | K8s | ❌ | ❌ |
| Narev AI | "FinOps pure-play" | 實為 FOCUS-1.2 converter + $20/mo SaaS | ❌ |

### Layer 4：GPU / Inference Infra

| 產品 | 焦點 | 閉環 |
|---|---|---|
| NVIDIA Run:AI | GPU 分割、bin-pack、MIG | ✅(GPU only) |
| OctoAI | **2024/10 已關閉**，併入 NVIDIA NIM | — |
| Anyscale | Ray Serve LLM + 開源 router | 🟡 |
| Modal Labs | per-second compute + budget API | self-control |
| Fireworks/Together/Replicate | provider 端優化 | 透過便宜 supplier |

### Layer 5：Inference Engine

| 引擎 | Output Length 預測 | Length-aware Scheduling |
|---|---|---|
| vLLM | ❌ mainline；vllm-ltr 為 fork | priority only（無預測） |
| SGLang | ❌ | priority PoC |
| TensorRT-LLM | ❌ | KV-eviction 政策 |
| TGI | ❌（**2025/12 maintenance**） | — |
| Triton/Dynamo | ❌ | dynamic batch |
| Ray Serve LLM | ❌ | custom routing |
| DeepSpeed-MII | ❌ | SplitFuse |
| MLC-LLM | ❌ | MicroServing 可程式化 |
| **llama.cpp** | ❌ | **`--reasoning-budget`**（唯一 OSS server-level mid-request 強制） |
| Mooncake (Kimi) | ❌（但有 SLO-violation predictor） | KVCache-centric |

---

## 4. 能力矩陣

### 4.1 Top 20 產品 × 4 能力

| 產品 | Pre-Predict | Runtime Control | Post-Optimization | Continuous Learning | Closed Loop |
|---|---|---|---|---|---|
| **CAST AI** | 🟡 workload | 🟡 soft via routing | ✅ GPU+LLM auto | 🟡 partial | **🟢 closest** |
| **TensorZero** | input only | DICL adapt | offline GEPA | **✅ bandit (variant)** | 🟡 |
| **Galileo Agent Control** | ❌ | ✅ policy plane | ✅ Insights | 🟡 auto-tuned metrics | 🟡 |
| **Portkey** | input only | **✅ hard budget cap** | 🟡 routing | ❌ | 🟡 |
| **Pydantic-AI** | ❌ | ✅ `UsageLimits` raise | ❌ | ❌ | 🟡 |
| **Azure Foundry Model Router** | ✅ trained classifier | ✅ 3 modes | ❌ 不透明 | opaque | 🟡 |
| **Maxim/Bifrost** | input only | ✅ virtual-key block + cost-tier fallback | 🟡 | 🟡 | 🟡 |
| **Helicone** (maint) | ❌ | ✅ multi-dim limits | 🟡 Auto-Improve | ❌ | 🟡 |
| **AWS Bedrock IPR** | per-(q,m) quality | within-family | ❌ | opaque | 🟡 |
| **Not Diamond** | quality | pre-request | ❌ | batch retrain | 🟡 |
| **Martian** | quality | pre-request | ❌ | batch | 🟡 |
| **RouteLLM** | quality (4 types) | static | research | ❌ | 🟡(research) |
| **Unify** | quality + live | pre-request | ❌ | live signals | 🟡 |
| **Braintrust** | ❌ | proxy fallback | ✅ **Loop** | 🟡 | 🟡 |
| **Run:AI** | workload | GPU only | ✅ fractioning | ✅ | 🟢(GPU only) |
| **Mooncake** | SLO-violation | KV-aware | ❌ | retrain | 🟡(SLO only) |
| **Narev / LiteLLM / CloudZero / Vantage / Datadog / Langfuse** | ❌ | ❌ 或 budget only | ❌ | ❌ | ❌ |

### 4.2 16 個能力維度評分（0-10）

| 能力 | 最高分產品 | 分數 | 說明 |
|---|---|---|---|
| Input token 計數 | tiktoken / HF tokenizers | 10 | 已解決 |
| **Output token 預測** | EGTP/PLP (research only) | 7 | **無商品化** |
| Reasoning token 預測 | TALE / SelfBudgeter (research) | 4 | 自我估算誤差 ±30-50% |
| Cost forecast (aggregate) | CloudZero / Vantage | 8 | 歷史外推 |
| Pre-request cost preview | LlamaIndex (input only) | 5 | 無 output |
| Hard runtime budget | Portkey | 9 | workspace+key 美元/token |
| Mid-stream model switch | **無人** | 0 | 全業界缺口 |
| Adaptive max_tokens | llama.cpp `--reasoning-budget` | 4 | 僅 reasoning |
| Cascading inference | RouteLLM / FrugalGPT | 7 | 學術成熟 |
| Semantic caching | Helicone / Portkey / LiteLLM | 8 | 已普及 |
| Prompt 自動壓縮建議 | LLMLingua-2 | 7 | 手動觸發 |
| Eval-gated auto-apply | **無人** | 1 | shadow + statistical gate 缺口 |
| Cross-customer learning | **無人** | 0 | 隱私 + 信任問題 |
| Online routing learning | TensorZero (variant level) | 5 | 非 prompt-conditional |
| Per-agent-step budget | **無人** | 0 | agent 時代新缺口 |
| GPU + Token 統一 | **CAST AI** | 6 | 僅它能做雙層 |

---

## 5. Layer 詳細分析（重點摘錄）

### 5.1 Observability 層

**綜合判定**：13 個產品中沒有任何真正閉環。本質為「ingestion plane」，不在 inference 路徑上。

**重點觀察**：
- **Helicone**：曾最有潛力（cost-based routing + Auto-Improve + semantic caching + multi-dim rate limits），**2026/3 被 Mintlify 收購進 maintenance mode**，產品聚焦轉向 documentation/agent platform。
- **Maxim AI / Bifrost**：最強 budget gating — virtual keys spend caps **block subsequent requests**；cost-aware routing 可從 GPT-4o → GPT-4o-Mini graduated downgrade。但 pre-request 而非 mid-stream。
- **Braintrust Loop**：唯一 AI 助理級「post-optimization recommendations」，但 human approval 才套用。
- **Galileo Agent Control（2026/3 開源）**：最強控制平面語意 — 「centralized policy layer for enforcing governance」+「real-time updates to agent policies」+「steering model selection to control token costs」。Policy 在 agent-step 邊界而非 mid-token-stream。

**Marketing-vs-reality gap**：

| 行銷詞 | 實際意涵 |
|---|---|
| "智慧路由" | 規則表 / round-robin / failover |
| "AI 驅動洞察" | cluster anomalies + LLM 摘要 |
| "成本最佳化" | dashboard 顯示 spend |
| "自我改進" | operator-promoted variant |
| "預測成本" | input × price + max × output_price 的上界 |
| "閉環" | 提供 Metrics API 讓你自建 |

### 5.2 Gateway / Router 層

**路由智慧光譜（dumb → smart）**：

1. Round-robin / weighted shuffle — LiteLLM 預設
2. Reactive heuristic — `least-busy`、`latency-based`
3. Hand-written rules — Cloudflare、Portkey Conditional、Vellum、Narev IF/THEN
4. Embedding-similarity to operator descriptions — Kong AI Proxy Advanced
5. Within-family learned classifier — AWS Bedrock IPR
6. Cross-family learned classifier (offline-trained) — Not Diamond、Martian、RouteLLM、Unify、**Azure Foundry Model Router**
7. Online bandit at variant level — **TensorZero**（唯一）
8. Online prompt-conditional bandit — **不存在商品化**（BaRP arXiv:2510.07429 為 research-only）

**TensorZero 深度分析**（最接近閉環）：
- Rust gateway（<1ms p99 overhead）+ ClickHouse + Python optimization SDK
- **In-gateway multi-armed bandit**（Track-and-Stop / GLRT），每幾分鐘從生產 metrics 更新採樣比例
- **Dynamic In-Context Learning (DICL)**：gateway 在 inference 時嵌入 prompt → 查詢 k 個最近 examples → 注入
- **GEPA**：offline 跑，產出新 prompt/model configs，operator promote
- 閉環在**系統層級**真實存在，**單次請求**內部非自適應；bandit 的 arm 是靜態 (model, prompt) pair

**Azure Foundry Model Router**（唯一主流雲商品化的 trained classifier）：
- 「trained language model that intelligently routes your prompts in real time」
- 3 modes：Balanced（1-2% quality band）、Cost（5-6% band）、Quality
- 22 個跨家族模型（GPT、DeepSeek、Llama-4、Grok-4、Claude）
- 限制：「effective context window is limited by the smallest underlying model」；無 mid-stream switch；customer 無 feedback hook

### 5.3 AI FinOps 層

**綜合判定**：類別存在於行銷話術中，實質產品分裂為三孤島。CAST AI 是最接近垂直閉環的。

**Narev AI 深度分析**（行銷與實質的落差）：
- 自我定位「AI FinOps 類別領導者」，行銷詞「missing experimentation layer」、「closed-loop」、「real-time model switching」
- 實際：開源 GitHub `narevai/Narev`（**29 stars**）為 FOCUS-1.2 ETL utility；Pricing API；Stripe/Polar/Lago/OpenMeter invoice 生成（**這是 AI sellers 的 monetization 工具，不是 AI buyers 的 optimization 工具**）；Custom leaderboard；Enterprise tier 才有 model router
- Free / $20 / Custom — 非 enterprise-sales-led；無公開募資紀錄
- **判定**：不是 Datadog clone with AI labels，而是**比那更小**

**CAST AI 深度分析**（最接近閉環）：
- **GPU 層**：bin-packing、time-slicing、MIG — 全自動套用
- **LLM 層**：「AI Enabler」自動路由至最便宜符合 quality 的 model
- 缺口：無明確 eval-gated quality preservation；無 cross-customer federated learning；LLM 端 quality threshold 定義較鬆

**Portkey 深度分析**（最強 hard budget enforcement）：
- 規模：每天處理 400B+ tokens，200+ enterprise（2026/3）
- Workspace 與 API-key 美元預算；週期重置；rate limits
- 「all subsequent requests from that workspace will be rejected until the time interval resets」
- 缺口：無 prediction、無 autonomous recommendations、無 learning

**Budget Enforcement 真實檢驗**：

| Vendor | 宣稱 hard cap | 實際阻擋 requests？ |
|---|---|---|
| **Portkey** | 是 | **驗證** — workspace + API-key |
| **Helicone** (maintenance) | 是 | **驗證** — multi-dim limits、fallback chains |
| AI Security Gateway / Unkey | 是 | 驗證 — HTTP 402/429 |
| Langfuse | 否 | 提供 Metrics API 讓你自建 |
| Datadog / New Relic / Dynatrace / Splunk | 否 | Alert-only |
| CloudZero / Vantage / Finout / Apptio | 否 | Allocation + alerts |
| Narev | 否 | Router suggests，無美元硬阻擋 |
| **CAST AI** | partial | Auto-apply model swap — soft cap，非 402/429 |

**結論**：只有 Gateway 架構（Portkey、Helicone）有可信的 runtime enforcement。其他都是 alerting。

**FinOps Foundation 動向**：
- **State of FinOps 2026**：98% 從業者管理 AI spend
- **FOCUS v1.2（2025/5）**：新增 **non-monetary pricing units**（credits、tokens）+ SaaS/PaaS。**Token-based AI pricing 與 cloud spend 同 schema**
- 標準化 vendors：Vantage、Finout、CloudZero、Apptio、nOps 全部對齊 FOCUS

### 5.4 Inference Engine 層

**綜合判定**：截至 2026/5 沒有 mainline OSS inference engine 上游有 output length predictor。

**為什麼研究 → 生產有大缺口**：
1. **Workload mismatch** — Alpaca-trained predictor 在 RL/CoT 上嚴重退化
2. **Heavy-tailed conditional distribution** — 同 prompt 多次採樣可變化 50×
3. **Reasoning models 打破抽象** — o1/R1 推論長度方差太大
4. **vLLM v1 scheduler 介面阻力** — hidden-state hooks 與簡化設計衝突
5. **邊際效益受 KV cache 與 PD-disaggregation 蓋過**

---

## 6. 學術研究 SOTA

### 6.1 Output Length Prediction（最關鍵）

| 方法 | 論文 | 年份 | 準確度 | 在生產嗎？ |
|---|---|---|---|---|
| **EGTP + PLP**（hidden-state, ICLR 2026） | arXiv:2602.11812 | 2026/2 | **MAE −29.16% vs TRAIL；−55.09% vs BERT proxy** | ❌ |
| **ProD-M / ProD-D**（distributional, heavy-tailed） | arXiv:2604.07931 | 2026/4 | quantile / Pareto-mixture loss | ❌ |
| **TRAIL**（hidden-state, ICLR 2025） | arXiv:2410.01035 | 2024 | MAE 2.66× lower than BERT proxy | ❌ |
| **vLLM-LTR**（learning-to-rank, NeurIPS 2024） | arXiv:2408.15792 | 2024 | 2.8× latency / 6.5× throughput | **fork only, not upstream** |
| **PARS**（cross-model pairwise ranker） | arXiv:2510.03243 | 2025 | 跨模型 generalize | vLLM patch |
| **Magnus**（LaBSE+RF） | arXiv:2406.04785 | 2024 | 66-234% throughput | ❌ |
| **SSJF**（BERT-base proxy） | arXiv:2404.08509 | 2024 | 30-40% JCT 改善 | ❌ |
| **S^3**（DistilBERT bucket） | arXiv:2306.06000 | 2023 | 6.49× throughput | ❌ |
| **Andes**（QoE 統計） | arXiv:2404.16283 | 2024 | 4.7× QoE | ❌ |

**方法分類（精度 → 粗）**：
1. **Hidden-state predictors using model's own activations** — TRAIL、EGTP — 最低 MAE，無額外 forward pass
2. **External small-model regressors / classifiers** — S³、SSJF、Magnus、ELIS — 5-20ms overhead；需 workload-specific retrain
3. **Self-prediction by serving LLM** — Heavy overhead；僅 bucketed targets
4. **Pairwise rankers** — vLLM-LTR、PARS — **不估計 length**，僅 relative order
5. **Distributional / statistical** — Andes、Vidur — trace-level priors

**Mooncake (Kimi)** 是唯一在生產部署 learned predictor 的系統 — 但預測 SLO-violation probability 而非長度本身。

### 6.2 Inference Scheduling（research → production）

關鍵系統列表（簡版）：vLLM (SOSP '23)、Sarathi-Serve（chunked prefill 已上游）、DistServe（PD-disagg 已上游）、Splitwise、**Mooncake (FAST '25 Best Paper)**、DynamoLLM (HPCA '25)、Mélange、Vidur、SGLang。

**0 個整合進 vLLM / SGLang / TensorRT-LLM / TGI mainline 預測器（2026/5）**。

### 6.3 KV Cache & Compression

- **H2O**（heavy-hitter eviction）、**StreamingLLM**（attention sinks，整合進 TGI/SGLang）、**SnapKV**、**CacheGen**、**CacheBlend**
- **LLMLingua / LongLLMLingua / LLMLingua-2**（Microsoft）：14-20× 壓縮，**PyPI 已普及**，LangChain / LlamaIndex 原生整合 — **生產化最成熟的層**
- Selective Context、AutoCompressor、Gist Tokens、RECOMP

### 6.4 Model Cascading / Routing

- **FrugalGPT**：98% cost reduction at GPT-4 quality
- **AutoMix**：50%+ compute reduction at parity
- **RouteLLM**：85% cost cut on MT-Bench at 95% GPT-4 quality
- **BEST-Route**、**Cascade Routing**

### 6.5 Reasoning Token / CoT Budget Control

| 方法 | 論文 | 效果 |
|---|---|---|
| DeepSeek-R1 | arXiv:2501.12948 | 純 RL training；展示 overthinking peak |
| **TALE** | arXiv:2412.18547 (ACL '25) | 67% output-token reduction；59% cost reduction |
| **SelfBudgeter** | arXiv:2505.11274 | 61% length reduction at parity |
| BudgetThinker | arXiv:2508.17196 | continuous in-context reminder |
| **TokenSkip** | arXiv:2502.12067 (EMNLP '25) | 40% token reduction at <0.4% accuracy drop |
| Reasoning on a Budget (Survey) | arXiv:2507.02076 | 兩層 taxonomy |

**重要發現**：TALE 與 SelfBudgeter 均達自我估算 budget，誤差 ±30-50% 仍能驅動有用 budget-following 行為。商業模型 Claude 3.7 budget tokens、OpenAI o1 reasoning_effort、DeepSeek-R1 都未公開暴露 predictor — user 必須指定上界。

### 6.6 Closed-Loop Online Learning（最稀有）

整個學術論文集中**只有 Mooncake (Kimi)** 在生產接近 predict → control → optimize → relearn，且 relearning 是 offline retrain。

**為什麼如此稀有**：
1. Distribution drift 真實但很少 catastrophic — 定期 re-train 達 80% 真實 online learning 價值
2. Online learning 破壞 scheduling 穩定性
3. Counterfactual data 困難
4. 實際成本節省天花板小（vLLM-LTR 大約 1 天訓練資料即飽和）
5. 運維 overhead — streaming labels、online evaluation、A/B infra

---

## 7. 技術深度分析

### 7.1 商業端的 token / cost estimation 都用同一方法

```
input_tokens = tokenizer.encode(prompt).length
cost_estimate = input_tokens × input_price + (max_tokens × output_price)  # 上界
actual_cost = (after) tokens_used × pricing_table  # 事後
```

**沒有產品在請求前真正預測 output tokens**。所有「cost-based routing」實質上是：
- LiteLLM `cost-based-routing` = pick the deployment with the lowest **configured** price
- Helicone "cost mode" = pick cheapest provider for the **same model**
- Cloudflare "Budget Limit step" = 反應式，超出後切換 fallback

### 7.2 Mid-stream switching 業界共識

業界共識（aisecuritygateway.ai blog）：

> "the gateway cannot interrupt a response mid-stream. The request that pushes you over the budget completes successfully, and the next request gets a 429."

唯一例外：**llama.cpp `--reasoning-budget`** 透過注入 end-of-thinking tokens 強制截斷 reasoning，但僅限 thinking 模型。

### 7.3 Online Learning 系統清單

| 系統 | 學習什麼 | 何時 |
|---|---|---|
| **TensorZero** | variant 之間的選擇權重 | 每幾分鐘 bandit 更新 |
| Mooncake | rejection threshold | 從生產 trace 重訓 |
| Spot.io | spot 中斷預測 | 持續 |
| 其他全部 | — | 靜態或人工觸發 |

**沒有產品做 online prompt-conditional learned routing**。

---

## 8. 七大市場缺口

| # | 缺口 | 為什麼難 | 商業價值 | 防禦性 |
|---|---|---|---|---|
| **1** | **Per-request output token 預測** | 跨 workload 普及困難；reasoning 方差極大 | 🔥🔥🔥 解鎖 pre-request cost preview | 中 |
| **2** | **Mid-stream model switching** | 串流中切換需要狀態轉移；KV cache 不相容 | 🔥🔥🔥 真實 budget enforcement | 高（工程難度） |
| **3** | **Reasoning token prediction & control** | o1/R1/Claude-thinking 難測；±30-50% 誤差 | 🔥🔥🔥 reasoning 模型最大成本黑洞 | 中 |
| **4** | **Eval-gated autonomous recommendations** | 需 customer-specific eval set + shadow traffic + statistical gate | 🔥🔥🔥 把建議變自動套用的關鍵 | 中（運維複雜度） |
| **5** | **Cross-customer federated learning** | 隱私 + 信任 + 加密聚合 | 🔥🔥 真正的 moat | **最高（網絡效應）** |
| **6** | **Per-agent-step budgets** | 現代 agent 多 tool calls；budget 必須能在 step 間檢查 | 🔥🔥 agentic AI 時代愈來愈痛 | 中 |
| **7** | **GPU + Token 統一推薦** | 兩個生態在不同團隊；買家不同 | 🔥🔥 frontier — 僅 CAST AI 嘗試 | 中 |

---

## 9. 為什麼還沒人做出來

### 9.1 結構性原因

1. **組織激勵錯位** — Allocation/dashboard 賣 FinOps + Finance；Gateway 賣 Platform/Eng；Eval/Obs 賣 ML/Quality。**閉環需要同時擁有三個 buyers — 困難 GTM motion**

2. **Eval 基礎設施是 prerequisite** — Auto-applying swap 需要 customer-specific evals，多數團隊沒有；vendor 必須自建/代管

3. **自治變更的責任風險** — Auto-routing GPT-4 traffic to Sonnet 沒有事先 approval = 生產事故 exposure；Gateways 因恐懼而停在「建議」

4. **Helicone 退場讓 OSS 賽道空虛** — Helicone 本有技術元件（gateway + caching + observability）+ OSS 分發，**2026/3 Mintlify 收購**轉向 documentation/agent platform

5. **學術 SOTA 剛公開** — EGTP/PLP（2026/2）、ProD（2026/4）剛公開；Productionization 需大量 production data 與 cross-workload validation 才能驗證

6. **vLLM/SGLang 核心團隊資源他用** — 預算花在 KV cache 重新設計、speculator、disaggregation；Predictor 不在 Q2 2026 roadmap

### 9.2 技術障礙

1. Workload heterogeneity — Alpaca-trained 在 RL/CoT 退化
2. Heavy-tailed conditional distribution — 同 prompt 變化 50×
3. Reasoning models 打破抽象
4. vLLM v1 scheduler 介面阻力
5. 邊際效益受 KV cache 與 PD-disaggregation 蓋過

---

# Part II：產品策略

## 10. 核心定位

### 10.1 一句話定義

> **Agent Runtime Spend Guardrails**
>
> **在 agent step / tool call / reasoning spend 邊界做 budget decision、policy enforcement、approval、rollback、audit 的 runtime 安全層。**

關鍵動詞：**decision、enforcement、approval、rollback、audit**。  
非關鍵：dashboard、attribution、forecast、recommendation。後者是入口必備，但**不是賣的東西**。

### 10.2 三條紅線

| 不做 | 為什麼 |
|---|---|
| ❌ 不做更好的 dashboard | LangSmith / Langfuse / Helicone / Datadog 已商品化 |
| ❌ 不做更準的 attribution | OpenAI Agents SDK / LangGraph / Pydantic-AI 原生有 usage primitive |
| ❌ 不做更便宜的 router | Portkey / TensorZero / Not Diamond 紅海 |
| ✅ 做的是 | **跨 runtime 的 step-boundary policy decision engine + audit-grade evidence trail** |

### 10.3 三個 buyer 的價值主張

| Buyer | 痛點 | 解決什麼 |
|---|---|---|
| **CTO / VP Eng / Platform Eng** | LLM/agent 成本不可預測；reasoning model 失控；agent 跑爛預算 | Per-route / per-agent-step budget + runtime guardrails + risk-band forecast |
| **CFO / FinOps** | AI spend 帳單看不懂；無法 chargeback；forecast 不準 | Spend attribution（app/customer/feature/agent step）+ FOCUS v1.2 export + accurate forecast |
| **DevOps / SRE** | Reasoning runaway / retry loop 引發 incident | Anomaly detection + audit log + circuit breaker |

### 10.4 必須修正的七個過強主張

| # | 過強版本 | 修正版本 |
|---|---|---|
| 1 | 「既有 vendors 全做不到 per-agent-step」 | 「既有 vendors 提供 step-level **tracing/usage primitive**，但缺**跨 runtime 的 step-level budget enforcement**」 |
| 2 | 「Reasoning token control 是 Phase 1 武器」 | 「Reasoning policy 分四級：hard / semi-hard / soft / unsupported」（見 §13） |
| 3 | 「Risk band 是 killer feature」 | 「Risk band **必須綁定 decision** 才有產品價值；獨立 forecast 是 commodity」（見 §14） |
| 4 | 「Safe Downgrade Simulator」 | **改名「Safe Downgrade Candidate Generator」**；無 BYO eval / human review 不能宣稱 quality-safe（見 §15） |
| 5 | 「FOCUS v1.2 export 是 wedge」 | 「FOCUS v1.2 export 是 **FinOps/reporting integration 支援**，與 CloudZero / Vantage 互通；不是 wedge」 |
| 6 | 「Cross-tenant pattern library 是早期 moat」 | 「Cross-tenant pattern library 是 **opt-in benchmark**（Phase 2-3）；護城河走 **per-tenant data accumulation**」 |
| 7 | 「Per-agent-step attribution 是 wedge」 | 「Attribution 是**入口商品**；wedge 是**邊界決策引擎**」 |

---

## 11. T→L→C→D→E→P 閉環基本元

### 11.1 不是 feature 列表，是產品閉環

```
┌─────────────────────────────────────────────────────────┐
│             Agent Runtime Spend Guardrails              │
├─────────────────────────────────────────────────────────┤
│                                                          │
│   ┌─────────┐      ┌─────────┐      ┌─────────┐        │
│   │  Trace  │─────▶│ Ledger  │─────▶│Contract │        │
│   └─────────┘      └─────────┘      └─────────┘        │
│        ▲                                  │             │
│        │                                  ▼             │
│   ┌─────────┐      ┌─────────┐      ┌─────────┐        │
│   │  Proof  │◀─────│Evidence │◀─────│Decision │        │
│   └─────────┘      └─────────┘      └─────────┘        │
│                                                          │
└─────────────────────────────────────────────────────────┘
```

### 11.2 六層原語定義

| 層 | 名稱 | 內容 | 為什麼是 product 一部分 |
|---|---|---|---|
| **T** | Trace | Canonical schema：穩定取得 agent run / step / tool call / model call / customer / route / cost 的 join key | 沒有穩定 join key，後續所有層 broken |
| **L** | Ledger | Provider usage normalization + step-level cost tree + pricing override + invoice reconciliation | 跨 provider 數字一致性 = 治理基礎 |
| **C** | Contract | Per-agent-run / per-customer / per-route / per-feature 的 budget 與 policy DSL | **這是 platform lock-in 的關鍵** |
| **D** | Decision | 在 step 邊界做 continue / degrade / skip optional step / stop run / require approval | **這是真 wedge — 沒人做** |
| **E** | Evidence | 每個 policy decision 有 reason、approver、rollback record、audit trail | 企業 compliance 必備；獨立資料 asset |
| **P** | Proof | 輸出 savings、avoided runaway、margin protection 的 case study material | Sales loop + customer renewal 武器 |

### 11.3 為什麼是閉環而非 feature list

任何一層斷掉，整條閉環不成立：
- Trace 沒有 join key → Ledger 無意義
- Ledger 不準 → Contract 無法定義
- Contract 沒有 policy DSL → Decision 沒語法
- Decision 沒有 Evidence → 無法 audit / rollback
- Evidence 沒有 Proof → 客戶不知道為什麼續約
- Proof 沒有 Trace 校準 → forecast 失準

Vendors 從 dashboard 切入只能做到 T+L；從 gateway 切入只能做到 D（無 C 與 E）；從 eval 切入只能做到部分 P。**完整 T→L→C→D→E→P 是真正的 product moat**。

### 11.4 Contract DSL 是真正的 platform lock-in

當客戶在 Contract DSL 寫下 50-200 條 budget policies：
- 每條 policy 描述「在 X 條件下，Y 預算超過 Z 時，做 W」
- 跨 runtime 一致語法（LangGraph / Pydantic-AI / OpenAI Agents 同一 DSL）
- Policy diff + git-tracked + immutable audit log

切換到 Portkey 或 Helicone 等 = 重寫 200 條 policies + 重新 audit + 重新 train ops team。**這是真正的 switching cost，比 attribution 整合或 dashboard UI 高一個量級**。

### 11.5 為什麼閉環只有 T→L→C→D→E→P，不含 Continuous Learning

原研究定義「閉環」為四件事：Predict + Control + Optimize + Learn。**本產品策略主動把 Learn 移出**，理由：

| 維度 | Predict / Control / Optimize | Continuous Learning |
|---|---|---|
| 難度類型 | 技術 / 統計 / 工程 / 物理約束 | 合規 / 隱私 / GDPR / consent |
| 解決後產出 | 產品深度 + IP + customer reference | 「不被告」 |
| 對 moat 貢獻 | 客戶 self-host 不可行 = lock-in | 法律風險上限 = ceiling |
| 失敗成本 | 工程 bug | 訴訟 / 監管罰款 |

**Learning 的難度多數是監管上限，創造 ceiling 而非 moat**。三留一捨後，閉環從 4 件事簡化為 3 件事，Phase 5+ 才考慮 active learning，且即便那時也優先個別 tenant 而非 federated。

**保留的部分**：Per-tenant data 在 Trace + Ledger + Evidence 中**自然累積**，產生 passive data network effect — 這是 by-product，不是 active learning loop。Predictor 升級（如 Phase 3 的 ML predictor）使用這些累積資料 offline 訓練，但不做 online learning。

類比：Cedar / Open Policy Agent 在 cloud auth 領域。我們是「**OPA for AI cost policies**」。

---

## 12. Runtime Adapter L0-L4 分級

### 12.1 五級定義

| Level | 名稱 | 能力 | 技術複雜度 | 前置依賴 |
|---|---|---|---|---|
| **L0** | Ingest | OTel / proxy 接收 trace；無控制 | 低（既有 OTel 管道） | 無 |
| **L1** | LLM-call attribution | 從 trace 拼出 LLM call → cost mapping | 中（per-provider mapping + invoice 對齊） | L0 |
| **L2** | Step attribution | 從 trace 拼出 agent step / tool call → cost tree | 中-高（runtime tracing protocol 解析） | L1 + Trace canonical schema 確認 |
| **L3** | Policy hook | 在 step 邊界 inject decision callback；可 stop / degrade / approve | 高（runtime callback API 整合 + fail-safe） | L2 + Contract DSL evaluator |
| **L4** | Runtime-native interruption | 與 runtime 內部協議深度整合，可在 inference mid-stream 介入 | 極高（多數 runtime 不可行；KV cache 跨模型不相容） | 不依賴 — 屬研究級工程，多數情境無法達成 |

### 12.2 Runtime 優先級

| Runtime | Phase 1 目標 | 為什麼 |
|---|---|---|
| **Pydantic-AI** | L3 | 已有 `UsageLimits` first-class；社群活躍；FastAPI 生態主流 |
| **LangGraph** | L3 | Checkpoint / super-step 邊界清楚；最大 agent runtime 市占 |
| **OpenAI Agents SDK** | L1-L2 | tracing 但 budget primitive 弱；社群影響力大但鎖入 OpenAI |
| **CrewAI** | L1-L2 | tracing 有；budget primitive 弱；adoption 中等 |
| **Anthropic Claude SDK** | L0-L1 | cost tracking 有但無 enforcement primitive |
| **AutoGen → MS Agent Framework** | L0 | 移轉中；不是 priority |
| **Smolagents** | L0 | 規模小 |
| **DSPy** | L0 | 不是 runtime，是 optimizer；當作上游資料源 |

**不要承諾全部 L4**。Mid-stream interruption 屬於 research 級工程，不應出現在客戶 pitch。

### 12.3 客戶 onboarding 累積階梯（condition-driven）

| 階段 | 進入條件 | 客戶體驗 |
|---|---|---|
| **Connect** | 接入完成 | OTel ingest（L0）；dashboard 看到 trace |
| **Reconcile** | 累積 ≥1 個 invoice cycle 的 traces | LLM-call attribution（L1）；ledger 對齊 invoice |
| **Map** | Trace 量達 step-level join key 可穩定建構（per-route minimum sample 達標） | Step attribution（L2）穩定；可寫第一條 contract budget |
| **Enforce** | 客戶完成 dry-run 觀察期（policy shadow 至 calibration tolerance 達標） | Policy hook（L3）啟用；decision 開始執行；evidence trail 累積 |
| **Prove** | 累積足夠 enforcement events 形成 case study material | 第一個 proof：「自動 stop 過 $X reasoning runaway，省 $Y」 |

每個階段對應 expansion 機會。**進入條件由資料累積與信任建立決定，不由 calendar 決定。**

---

## 13. Reasoning Policy 四級分類

### 13.1 分類定義

| 級別 | 定義 | Provider 範例 | 我們能做什麼 |
|---|---|---|---|
| **Hard** | Provider 接受明確 token cap，超過直接截斷 | Anthropic `budget_tokens`、llama.cpp `--reasoning-budget` | 完整 enforcement |
| **Semi-hard** | Provider 接受離散級別（不可指定 token 數） | OpenAI o1 `reasoning_effort`（low/medium/high）、Gemini thinking budget | Mapping policy → 適當級別 |
| **Soft** | 無 native control；可透過 prompt engineering 影響 | DeepSeek-R1（早期）、開源 thinking 模型 | Prompt injection + 異常偵測 |
| **Unsupported** | 無法控制；只能監控與告警 | 部分 closed-source thinking models | Anomaly detection only |

明確標示在 UI / docs：「對 Provider X 是 Hard control；對 Provider Y 是 Anomaly only」。誠實避免「我們能控制所有 reasoning」的過度承諾。

---

## 14. Risk Band 必須綁 Decision

### 14.1 原則

> **Risk band 獨立 = forecast，無價。Risk band 綁 decision = 產品。**

### 14.2 Risk Band 必備揭露

| 揭露項 | 為什麼必要 |
|---|---|
| **Sample size** | n=12 與 n=12,000 的 P90 不同信賴區間 |
| **Confidence interval** | 揭露不確定性而非佯裝精確 |
| **Cold-start fallback** | 新 route / 新 model 沒資料時的回退（保守上界） |
| **Calibration report** | 「過去 30 天，P90 預估超出實際 X% 次」— 證明準確度 |
| **Distribution shape** | 是否 heavy-tailed（reasoning 模型常見）— 影響 P99 |

無這五項揭露 = 不能上線。

### 14.3 Risk Band 綁 Decision 範例

| Policy | 範例 |
|---|---|
| `reject_if_p90_exceeds(budget)` | 「P90 預估超過 $50，拒絕請求」 |
| `downgrade_if_p50_exceeds(threshold)` | 「P50 預估超過 $20，路由至 Haiku」 |
| `cap_max_tokens_to_p90` | 「設 max_tokens = P90 預估值 × 1.2」 |
| `skip_optional_step_if_run_p99_exceeds` | 「整個 run 的 P99 超過 budget，跳過 optional step」 |
| `require_approval_if_p99_unbounded` | 「P99 為無界（heavy-tail），要求人工 approval」 |

**Risk band 是 Contract 的輸入；Decision 是 Contract 的輸出**。

---

## 15. Safe Downgrade Candidate Generator

### 15.1 為什麼改名

「Safe Downgrade Simulator」隱含品質保證承諾。沒有 customer eval / human review / shadow replay，不能宣稱 safe。改名為 **Safe Downgrade Candidate Generator** — 產生候選方案，不保證安全。

### 15.2 工作流分層

| 階段 | 動作 | 我們做 | 客戶做 |
|---|---|---|---|
| 1. Generate | 從 production traces 識別 downgrade 候選 | ✅ | — |
| 2. Project | 計算 projected $ savings 與 candidate latency | ✅ | — |
| 3. Replay | 在 historical traces 上執行 candidate model | ✅（infra） | — |
| 4. Eval | 用 customer's eval set 對比 quality | ✅ 整合 BYO eval | ✅ 提供 evals |
| 5. Decide | 基於 eval 結果決定是否 promote | — | ✅ |
| 6. Deploy | 透過 Contract DSL 套用 | ✅（mechanism） | ✅（approve） |
| 7. Monitor | 觀測 quality drift；觸發 rollback | ✅ | — |

我們提供 1-3 與 6-7（infra）；客戶 4-5 是 human-in-loop。**這是 Phase 3 才完整成立**，Phase 1 只做 1-2。

---

## 16. Enterprise Trust Architecture

Phase 0 不應只寫 SOC2/RBAC/VPC。完整清單：

| 類別 | 項目 |
|---|---|
| **Failure handling** | Fail-open / fail-closed 策略；Kill switch；任意 policy / decision rollback |
| **Operational safety** | Policy dry-run / shadow mode（新 policy 先 shadow 至 calibration tolerance 達標）；Service accounts / API key rotation |
| **Data governance** | Prompt retention policy（0/30/90 days）；Prompt redaction（PII at gateway）；No-training guarantee；Data residency（US / EU / 自訂） |
| **Provider governance** | Model / provider allowlist；Contract pricing override；Invoice reconciliation |
| **Audit & compliance** | Policy diff（git-tracked）；Immutable audit log（append-only + signed）；Approver tracking |
| **Legal** | DPA；Subprocessor list；Data flow diagram；SOC 2 Type II；ISO 27001 / FedRAMP（Phase 2+） |

**Phase 0 必須交付這 19 項中的 15 項以上**（FedRAMP / ISO 可延後）；這是 Phase 1 開放給 enterprise 客戶的前置條件。沒有這些 = pipeline 過不了 procurement。

---

## 17. FOCUS / Cross-tenant 重新定位

### 17.1 FOCUS v1.2 export

| 維度 | 定位 |
|---|---|
| 性質 | **Reporting / FinOps integration 支援** |
| 目的 | 讓客戶 FinOps team 整合 Apptio / CloudZero / Vantage |
| Phase | Phase 1 必備（table stakes），但不是 wedge |
| 行銷 | 「我們與你既有 FinOps stack 互通」而非「我們是 FinOps 工具」 |

### 17.2 Cross-tenant Pattern Library 與 Continuous Learning 的處理

**戰略決策**：Continuous Learning（含 federated learning）整體**移出產品範圍**。理由詳見 §11.5 與 §22.4。

| 層 | 性質 | 是否做 |
|---|---|---|
| **Per-tenant data accumulation** | Trace + Ledger + Evidence 自然累積（passive by-product） | ✅ Phase 1 起，無需 active learning |
| **Anonymized opt-in benchmarks** | 「您的 reasoning runaway 比同業 P75 多 30%」 | 🟡 Phase 3+ 可考慮，純統計報告無模型訓練 |
| **Pattern library**（匿名 best-practice templates） | 業界 budget policy starter library | ✅ 從 Phase 1 內部累積，但不從客戶資料學習 |
| **ML predictor offline retrain** | 用 per-tenant 累積資料 offline 訓練 predictor | 🟡 Phase 3 可做，但**不做 cross-tenant** 也**不做 online learning** |
| **Online learning / bandit routing** | TensorZero 式 in-gateway bandit | ❌ 不做 |
| **Cross-tenant federated learning** | 隱私保護聚合 | ❌ 不做（Phase 5+ 之後重新評估） |

**真正的 moat 路徑**：

> Per-tenant data network effect（**passive accumulation, not active learning**）+ Contract DSL switching cost + Audit trail accumulation = **三重複合 moat**

不是 federated learning。也不是 online learning。**資料累積本身即 moat，無需主動學習迴圈**。

---

## 18. 失敗模式

| 失敗模式 | 觸發信號 | 對策 |
|---|---|---|
| 被當作另一個 LangSmith / Helicone | 客戶問「跟 LangSmith 差別？」需解釋超過 30 秒 | 拒絕 visibility-led pitch；只談 Decision-at-boundary |
| Phase 1 過度承諾 reasoning control | 客戶遇到 DeepSeek-R1 發現我們無 hard control | UI / docs 明確標示 hard/semi-hard/soft/unsupported |
| Risk band 失準導致信任崩塌 | 客戶 P90 跑超實際 → 投訴 | 預設保守（P95 起跳）+ calibration report 透明 |
| Contract DSL 太複雜或太簡單 | 客戶寫不出 policy / 寫的 policy 不符合需求 | DSL design 分 5 個 maturity tier；提供 starter templates |
| 採購卡 SOC 2 | 第一個 enterprise audit 卡關 | Phase 0 SOC 2 Type II 為 Phase 1 開放 enterprise 的前置條件 |
| Runtime 整合廣度陷阱 | 同時做 8 個 runtime 全部 L1 | 死守 Pydantic-AI + LangGraph 至 L3；其他 L0-L1 |
| Decision boundary 觸發生產事故 | 我們的 enforcement 害客戶服務掛掉 | Fail-open 預設 + dry-run 強制 + kill switch |
| Per-step attribution 被 LangSmith 趕上 | LangSmith 加上 budget enforcement | 加速 Contract DSL + Decision engine 深度；attribution 是入口非護城河 |
| 客戶要求 cross-tenant federated learning | sales 過度承諾 | 明確：產品**不做** continuous learning；only passive data accumulation；§11.5 與 §22.4 解釋理由 |

---

## 19. Phase 路線圖（condition-driven，非 time-driven）

> **AI agent 開發下，dev time 不再是 sequencing 約束。Phase 由「前置依賴 + 累積條件 + 可驗證性」決定，不由月份決定。**

### Phase 0 — Trust Architecture Foundation
**進入條件**：項目啟動  
**離開條件**（解鎖 Phase 1 enterprise pipeline）：
- Enterprise trust architecture 19 項中 ≥15 項到位
- 設計合作夥伴 ≥5 家簽約
- Contract DSL v0 spec 完成（語法 + 評估語意 + reference design）

### Phase 1 — T→L→C→D→E→P 閉環 MVP
**進入條件**：Phase 0 離開條件達成  
**交付**：
- **T**：OTel + Pydantic-AI + LangGraph adapter（L2-L3）
- **L**：Provider normalization + step-level cost tree + invoice reconciliation
- **C**：Contract DSL v1 + 20 starter templates
- **D**：Step-boundary policy hook（continue / degrade / skip / stop / approve）
- **E**：Immutable audit log + policy diff + approval workflow
- **P**：Customer case studies（≥5 個 reference logos 證明 avoided runaway / margin protection）
- Risk band（綁 Decision，含 5 項揭露）
- Reasoning policy 四級分類 with provider mapping table
- Safe Downgrade Candidate Generator（generation only）
- FOCUS v1.2 export（reporting integration）

**離開條件**（解鎖 Phase 2）：
- ≥5 paying customers 完成 Connect → Enforce 階梯
- Per-tenant trace 量足以驅動 risk band 計算
- 至少 3 個 case study 完成
- Cross-runtime 整合教訓累積足以擴展至更多 runtime

### Phase 2 — Runtime 廣度 + Replay
**進入條件**：Phase 1 離開條件達成  
**交付**：
- 加 OpenAI Agents SDK / CrewAI 至 L2-L3
- Replay infrastructure（shadow mode）
- Anonymized opt-in benchmarks
- Predicted-cost gating（risk band P90 reject）

**離開條件**（解鎖 Phase 3）：
- 累積足夠 production traces 量級可訓練 ML output predictor
- 至少 3 家客戶提供 BYO eval set
- Replay infrastructure 在 shadow mode 證明 stable

### Phase 3 — Optimization Lab
**進入條件**：Phase 2 離開條件達成（資料量 + eval 基礎）  
**交付**：
- BYO eval 整合（Braintrust / Galileo / 內部 evals）
- ML output predictor（vllm-ltr / EGTP productionization；以 Phase 2 累積資料為訓練集）
- Bandit routing for variant selection

**離開條件**（解鎖 Phase 4）：
- ML predictor 在 ≥3 個 workload 類型上達 production-grade calibration
- Eval-gated promotion 在 shadow 模式下證明 quality preservation
- 客戶信任累積足夠允許 limited auto-apply

### Phase 4 — Governed Autopilot
**進入條件**：Phase 3 離開條件達成（信任 + ML 基礎）  
**交付**：
- Auto-apply low-risk policies（cache tuning、stop sequence、reasoning cap、context trim）
- Anonymized opt-in benchmarks（純統計報告，無模型訓練）
- Auto-rollback infrastructure
- ~~Federated learning POC~~ → **移除**（已決策不做 continuous learning）

**離開條件**（解鎖 Phase 5）：
- Auto-apply quality drift incidents < 0.1%
- Anonymized benchmarks 對至少 3 家客戶提供可量化價值
- 三大支柱（Predict / Control / Optimize）在 ≥10 enterprise 客戶證明 closed-loop value

### Phase 5+ — 北極星
**進入條件**：Phase 4 離開條件達成
- 三支柱（Predict + Control + Optimize）的深化（不是 add learning）
- L4 runtime-native interruption（如底層協議 / 硬體允許 — 仍多數情境不可行）
- Continuous Learning **不在北極星範圍** — 重新評估僅當合規 / 隱私 / consent 環境根本變化（如 differential privacy 普及、industry-wide consent 標準成熟）

---

## 20. Wedge / Moat 最終答案

### 20.1 Wedge（為什麼客戶選我們）

> **跨 runtime 的 step-boundary policy decision engine**
>
> 既有 vendors 提供 visibility / attribution / forecast / dashboard。**沒有 vendor 在 agent step 邊界做 budget decision、enforcement、approval、rollback、audit**。我們是。

### 20.2 Moat（為什麼客戶不換）

> **三重複合：Per-tenant data accumulation + Contract DSL switching cost + Audit trail accumulation**
>
> 不是 cross-tenant federated learning（過度浪漫的願景）。  
> 不是更好的 dashboard（紅海）。  
> 不是更精準的 attribution（被既有 vendors 商品化）。

### 20.3 Attribution 是入口，不是護城河

| 客戶旅程階段 | 角色 |
|---|---|
| **評估期**（pre-Connect） | Attribution / Trace / Ledger 是新客戶評估我們的入口（必須好用） |
| **使用期**（Connect → Enforce） | Contract DSL + Decision engine + Audit log 是客戶留下的原因 |
| **規模期**（多客戶 + 多 trace 累積後） | 三重 moat 累積，新進入者追不上 |

---

# Part III：策略演進記錄

## 21. v1.0 → v1.1 → v1.2 判斷歷程

### 21.1 三次迭代摘要

| 版本 | 定位 | 主要問題 | 修正觸發 |
|---|---|---|---|
| **v1.0**（市場研究結論） | "AI Runtime Optimization Layer" — 完整閉環平台 | 太大、5 年願景無法 MVP | 用戶要求落地 |
| **v1.1** | "LLM Spend Control Plane for the Agent Era" | 仍紅海；feature list 思維；混淆「市場 gap」與「市場時機」 | Codex v1 反饋（4 個 agents） |
| **v1.2** | **"Agent Runtime Spend Guardrails"** | 收窄至 step-boundary decision engine | Codex v2 反饋 — 「再砍一刀」 |

### 21.2 核心轉折點

**v1.0 → v1.1**：放棄「完整閉環」抽象，採用 4-phase plan（Spend Control → Runtime Guardrails → Optimization Lab → Governed Autopilot）。但 Phase 1 仍包含 visibility / allocation / budget / forecast / anomaly 全範疇。

**v1.1 → v1.2**：放棄「Spend Control Plane」全範疇，收窄至 **decision-at-boundary**。Attribution / dashboard / forecast 降為**入口商品**而非賣點。Phase 1 重構為 T→L→C→D→E→P 閉環基本元而非 feature 列表。Moat 從「cross-tenant federated learning」改為「per-tenant data + Contract DSL + audit trail 三重複合」。

### 21.3 對 codex 反饋的最終處理統計

#### Codex v1（4 agents：市場/競品、技術架構、B2B SaaS、FinOps/採購）

| 反饋 | 處理 |
|---|---|
| 不要早期下注 mid-stream switching | ✅ 採納 |
| 不要早期下注 federated learning | ✅ 採納，補強 per-tenant 累積 |
| 不要早期下注 universal output predictor | ✅ 採納，改用 risk band；保留 ML predictor 作 Phase 3 升級 |
| 不要早期下注 GPU + token 統一 | ✅ 採納 |
| 不要 fork vLLM/SGLang | ✅ 採納，sidecar |
| 4-phase 路線圖 | ✅ 採納 |
| Phase 1 = visibility/allocation/budget/forecast/anomaly | 🟡 部分反駁 — 必須在 agent-era 三向量上強差異化 |
| Eval-gated auto-apply 不該早期 | 🟡 細分 — shadow replay + manual approve 早期可做；自動套用 Phase 4 |
| Reasoning token control 不該早期 | ⚠️ 反駁 — 是 Phase 1 武器 |
| 採購 / 審計 / 合規 補強 | ✅ 採納 |

**v1.1 採納率**：~80%。

#### Codex v2（再砍一刀）

| 反饋 | 處理 |
|---|---|
| 從「Spend Control Plane」收窄為「Agent Runtime Spend Guardrails」 | ✅ 完全採納 |
| Attribution 是入口，不是護城河 | ✅ 完全採納，列為核心論述 |
| Per-agent-step claim 太強 | ✅ 修正為「跨 runtime 的 step-level enforcement」 |
| Reasoning control 拆 hard/semi-hard/soft/unsupported | ✅ 完全採納 |
| Risk band 必須綁 decision | ✅ 完全採納 |
| Safe Downgrade Simulator → Candidate Generator | ✅ 完全採納 |
| FOCUS export 不是 wedge | ✅ 完全採納 |
| Cross-tenant pattern library 不是早期 moat | ✅ 完全採納 |
| Phase 1 改 T→L→C→D→E→P 閉環 | ✅ 完全採納 |
| Runtime adapter L0-L4 分級 | ✅ 完全採納 |
| Phase 0 enterprise trust architecture 完整化 | ✅ 完全採納 |

**v1.2 採納率**：100%。

### 21.4 額外補強（codex 沒明說但我認為重要）

1. **Contract DSL = OPA for AI cost** — 讓 platform engineering buyer 立刻理解 lock-in 機制
2. **三重 moat 而非單一 federated learning** — 重新定義防禦性
3. **L4 runtime-native interruption 工程不確定性誠實標示** — 多數 runtime 不可行（KV cache 跨模型不相容等物理約束）
4. **Risk Band 必須揭露 5 項**（sample size、CI、cold-start、calibration、distribution shape）— 避免假精確的信任崩塌

## 22. 關鍵教訓

### 22.1 從研究到策略的五個盲點（v1.0 → v1.1 修正）

1. **混淆「市場 gap」與「市場時機」** — gap 永遠存在；timing（Helicone 退場 + reasoning model 失控 + FOCUS v1.2 標準化）才是論據
2. **跳過 GTM 與 buyer 分析** — 直接跳到「market layer」抽象論述
3. **低估採購 / 合規門檻** — 沒把 Phase 0 列入
4. **Output predictor 框架太硬** — 應從 risk band 起步，predictor 是升級路徑而非 MVP 核心
5. **沒給條件驅動的 phase 進入 / 離開準則** — 列出 deliverables 但未說明何時可進下一階段

### 22.2 從策略到產品的四個盲點（v1.1 → v1.2 修正）

1. **把 visibility / attribution 當作護城河** — 它們是入口商品；護城河在 enforcement
2. **Feature list 思維 vs 閉環產品思維** — 10 個 feature 列表不如 T→L→C→D→E→P 閉環基本元有產品定義力
3. **過強 claim 的法律與信任風險** — 「Safe」「閉環」「智慧」未驗證即承諾，第一次失敗後信任崩塌
4. **Federated learning 浪漫化** — 真正可達的 moat 是 per-tenant 累積 + DSL switching cost + audit trail，不是技術上炫的聯邦學習

### 22.3 通用啟示

- **市場研究識別事實，產品策略提出判斷** — 兩個範疇不能混淆
- **AI agents 反饋是放大鏡** — 多輪嚴格批評比單輪深思熟慮有效
- **80% → 100% 採納的差距是「自我反駁不夠狠」** — v1.1 仍保留我喜歡的部分（cross-tenant federated learning 為 moat）；v1.2 才放棄
- **「再砍一刀」的勇氣** — 收窄定位需要捨棄合理但不必要的範圍

### 22.4 為什麼主動捨棄 Continuous Learning（4 → 3 支柱決策）

**原研究**將閉環定義為四件事：Predict + Control + Optimize + Learn。**本產品策略主動把 Learn 移出**，理由：

**核心區分：難度創造什麼？**

| 範疇 | 難度類型 | 解決後產出 | 是 moat 嗎？ |
|---|---|---|---|
| 事前預測 | 技術 / 統計 / 物理 | 產品深度、IP、case study | ✅ moat（客戶 self-host 不可行） |
| 事中控制 | 工程 / 物理約束 | operator trust、系統可靠性 | ✅ moat |
| 事後優化建議 | 統計 / 信任 | customer reference、品質 record | ✅ moat |
| 持續學習 | **合規 / 隱私 / GDPR / consent** | **「不被告」** | ❌ **ceiling，非 moat** |

**Learning 的難度多數是監管 / 合規上限**：consent、GDPR right-to-be-forgotten、cross-tenant 匿名化反推風險、training-on-customer-data 條款。解決它們創造的是「不被告」的法律空間，**不是「客戶離不開」的產品深度**。

而 Predict / Control / Optimize 的難度是**技術深度**：heavy-tailed distribution、KV cache 不相容、LLM-as-judge bias、Replay infrastructure。解決它們累積為**產品 IP + 客戶引證 + 工程能力**，這些是真 moat。

**Passive accumulation ≠ active learning**：
- Trace 累積 → Risk band calibration（passive，每客戶獨立）✅ 做
- Per-tenant predictor offline retrain → Phase 3 ML predictor 升級（passive，明確 consent）✅ 做
- Cross-tenant federated learning（active，需 federated infrastructure）❌ 不做
- Online bandit / RL routing（active，狀態跨 request 演化）❌ 不做

**結論**：閉環從「四件事」簡化為「三件事」。Continuous Learning **不是延後到 Phase 5+，是主動排除**。Phase 5+ 之後若監管環境根本變化（differential privacy 普及、industry consent 標準成熟），可重新評估。

### 22.5 為什麼分階段（無時間版本論證）

**前提**：本產品由 AI agents 開發，dev time 在實務上接近瞬時。任何基於「N 個月開發」「N 年到位」的論證都不成立。

**反問**：那為什麼還要分階段？為什麼不直接 ship 北極星？

**答案**：分階段的真正驅動力**從來不是 dev time，是以下五個與 dev time 無關的結構性約束**：

| 結構性約束 | 為什麼 AI agents 加速不了 |
|---|---|
| **Trust 必須以 customer calendar 累積** | 即使系統一小時建好，客戶 day 1 也不會把生產流量交給 auto-apply。Trust 來自實際生產上跑過、沒爆過的觀察記錄 — 這是**客戶側時間**，不是開發側時間 |
| **研究級技術受物理約束** | Mid-stream switching 業界 0 實作不是「沒人做」，是 **KV cache 跨模型架構不相容**這類物理性約束。AI agents 無法把不可能變可能 |
| **客戶學習迴圈必須存在** | 不知道客戶實際需要什麼，直到他們用過。北極星 spec 是假設 — ship-to-learn 仍 beat plan-to-perfect，因為 plan 基於假設、ship reality 是真的 |
| **Liability surface 必須漸進** | 不是「修 bug 要多久」，而是「bug 在多大範圍上爆」。直接 ship 北極星 = bug 在最大範圍爆，無 limited blast radius |
| **採購側 calendar 不可壓縮** | RFP、SOC 2 audit、enterprise procurement、安全審查 — 客戶內部流程是 calendar-paced，與 dev 速度無關 |

**真正的 staging 邏輯**（無時間版本）：

| 維度 | 為什麼分階段 |
|---|---|
| **依賴圖** | T → L → C → D → E → P 有明確 join key 依賴；先後順序不能互換 |
| **可驗證性** | Risk band 從第一個客戶資料即可驗證；ML predictor 需要量級資料才能 calibrate |
| **資料需求** | Per-tenant 學習要先有 per-tenant 資料；federated 要先有 multi-tenant 資料 |
| **防禦性建立** | DSL switching cost 與 audit trail accumulation 是 by-product of **usage**，不是 by-product of **code** |

每個 Phase 開始**不是因為時間到了，而是因為前一 Phase 累積了下一 Phase 必需的條件**。Phase 進入 / 離開條件已在 §19 明確列出。

**一句話**：

> 北極星不是因為**做不完**而分階段，而是因為**每個 Phase 是下一 Phase 的 prerequisite**。  
> 即使 AI agents 寫 code 是免費瞬時的，**客戶資料、信任、validation、liability 累積都不是**。

---

# Part IV：附錄

## 23. 參考論文

### Output Length Prediction
- EGTP / PLP — arXiv:2602.11812（ICLR 2026）
- TRAIL — arXiv:2410.01035（ICLR 2025）
- ProD-M / ProD-D — arXiv:2604.07931（2026/4）
- vLLM-LTR — arXiv:2408.15792（NeurIPS 2024）
- PARS — arXiv:2510.03243（2025）
- Magnus — arXiv:2406.04785（ICWS '24）
- S^3 — arXiv:2306.06000（NeurIPS '23）
- SSJF — arXiv:2404.08509（ASPLOS '24）
- ELIS — arXiv:2505.09142（2025）
- Andes — arXiv:2404.16283（2024）
- Block — arXiv:2508.03611（2025）
- Response Length Perception — arXiv:2305.13144（NeurIPS '23）

### Inference Scheduling
- Orca (OSDI '22)
- vLLM PagedAttention — arXiv:2309.06180（SOSP '23）
- Sarathi-Serve — arXiv:2403.02310
- DistServe — arXiv:2401.09670（OSDI '24）
- Splitwise — arXiv:2311.18677（ISCA '24）
- Mooncake — arXiv:2407.00079（FAST '25 Best Paper）
- DynamoLLM — arXiv:2408.00741（HPCA '25）
- Vidur — arXiv:2405.05465（MLSys '24）
- SGLang — arXiv:2312.07104（NeurIPS '24）

### KV Cache & Compression
- H2O — arXiv:2306.14048
- StreamingLLM — arXiv:2309.17453
- LLMLingua-2 — arXiv:2403.12968
- Selective Context — arXiv:2310.06201
- AutoCompressor — arXiv:2305.14788
- Gist Tokens — arXiv:2304.08467
- RECOMP — arXiv:2310.04408

### Routing & Cascading
- FrugalGPT — arXiv:2305.05176
- AutoMix — arXiv:2310.12963
- RouteLLM — arXiv:2406.18665
- BaRP — arXiv:2510.07429

### Reasoning Token Control
- DeepSeek-R1 — arXiv:2501.12948
- TALE — arXiv:2412.18547
- SelfBudgeter — arXiv:2505.11274
- BudgetThinker — arXiv:2508.17196
- TokenSkip — arXiv:2502.12067
- Reasoning on a Budget (Survey) — arXiv:2507.02076

## 24. 商業產品 URLs

### Observability
- Langfuse: https://langfuse.com/docs/observability/features/token-and-cost-tracking
- Helicone: https://docs.helicone.ai/guides/cookbooks/cost-tracking
- LangSmith: https://docs.langchain.com/langsmith/cost-tracking
- Arize: https://arize.com/docs/phoenix/tracing/how-to-tracing/cost-tracking
- Braintrust: https://www.braintrust.dev/articles/best-tools-tracking-llm-costs-2026
- Maxim/Bifrost: https://docs.getbifrost.ai/overview
- Galileo Agent Control: https://thenewstack.io/galileo-agent-control-open-source/
- OpenLIT: https://openlit.io
- Datadog LLM Obs: https://docs.datadoghq.com/llm_observability/
- Mintlify acquires Helicone: https://www.mintlify.com/blog/mintlify-acquires-helicone

### Gateway / Router
- TensorZero: https://www.tensorzero.com/blog/bandits-in-your-llm-gateway/
- Portkey: https://portkey.ai/docs/product/ai-gateway
- LiteLLM: https://docs.litellm.ai/docs/routing
- Kong AI: https://developer.konghq.com/ai-gateway/semantic-similarity/
- Cloudflare AI Gateway: https://developers.cloudflare.com/ai-gateway/features/dynamic-routing/
- AWS Bedrock IPR: https://aws.amazon.com/bedrock/intelligent-prompt-routing/
- Azure Foundry Model Router: https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-router
- Martian: https://work.withmartian.com/
- Not Diamond: https://www.notdiamond.ai/
- RouteLLM: https://github.com/lm-sys/RouteLLM

### AI FinOps
- Narev: https://www.narev.ai
- CAST AI: https://cast.ai/blog/why-cast-ai-is-best-for-llm-workloads/
- CloudZero: https://www.cloudzero.com/blog/ai-cost-optimization-at-scale/
- Vantage: https://www.vantage.sh/blog/finops-for-ai-token-costs
- nOps: https://www.nops.io/blog/ai-cost-visibility-the-ultimate-guide/
- Densify/Kubex: https://kubex.ai/product/gpu-optimization/
- FinOps Foundation AI: https://www.finops.org/wg/finops-for-ai-overview/
- FOCUS v1.2: https://focus.finops.org/focus-specification/v1-2/

### Inference Infra
- NVIDIA Run:AI: https://www.nvidia.com/en-us/software/run-ai/
- Anyscale Router: https://www.anyscale.com/blog/building-an-llm-router-for-high-quality-and-cost-effective-responses

## 25. 開源 Repos

### Inference Engines
- vLLM: https://github.com/vllm-project/vllm
- SGLang: https://github.com/sgl-project/sglang
- TensorRT-LLM: https://github.com/NVIDIA/TensorRT-LLM
- Mooncake: https://github.com/kvcache-ai/Mooncake
- llama.cpp: https://github.com/ggml-org/llama.cpp
- vllm-ltr (fork): https://github.com/hao-ai-lab/vllm-ltr

### Agent Runtimes
- Pydantic-AI: https://github.com/pydantic/pydantic-ai
- LangGraph: https://github.com/langchain-ai/langgraph
- LlamaIndex: https://github.com/run-llama/llama_index
- CrewAI: https://github.com/crewAIInc/crewAI
- Smolagents: https://github.com/huggingface/smolagents
- DSPy: https://github.com/stanfordnlp/dspy

### Compression & Tokenization
- LLMLingua: https://github.com/microsoft/LLMLingua
- Selective Context: https://github.com/liyucheng09/Selective_Context
- AutoCompressors: https://github.com/princeton-nlp/AutoCompressors
- PCToolkit: https://github.com/3DAgentWorld/Toolkit-for-Prompt-Compression
- tiktoken: https://github.com/openai/tiktoken
- genai-prices: https://github.com/pydantic/genai-prices
- anthropic-tokenizer: https://github.com/anthropics/anthropic-tokenizer-typescript

### Optimizers
- TextGrad: https://github.com/zou-group/textgrad
- SAMMO: https://github.com/microsoft/sammo
- GEPA: https://github.com/gepa-ai/gepa

## 26. Roadmap 與 Issues

- vLLM Q2 2026: https://github.com/vllm-project/vllm/issues/39749
- vLLM Priority Scheduling: https://github.com/vllm-project/vllm/pull/19057
- SGLang Q1 2026: https://github.com/sgl-project/sglang/issues/12780
- SGLang Priority Scheduling: https://github.com/sgl-project/sglang/issues/13526
- llama.cpp Reasoning Budget: https://github.com/ggml-org/llama.cpp/discussions/21445

---

## 27. 一句話總結

> v1.0 想做整個 closed loop（4 件事）。  
> v1.1 想做 spend control plane（仍紅海）。  
> **v1.2 確定 wedge：在 agent step 邊界做 budget decision engine。**  
> **v1.3 確定支柱：三支柱閉環（Predict + Control + Optimize），主動排除 Continuous Learning。**  
>   
> Attribution 是 commodity 入口，Decision-at-boundary 是 product，  
> Contract DSL + per-tenant **passive** data + audit accumulation 是 moat。  
>   
> **難度創造 moat 的條件**：難度必須是技術 / 統計 / 工程性質。  
> **合規 / 隱私難度只創造 ceiling，不創造 moat** — 故 Continuous Learning 不做。  
>   
> **Phase 由依賴條件驅動，非時間驅動** — AI agents 加速 dev，但不加速 trust / validation / liability 累積。

---

*Document version: complete v1.3 (integrated v1.0 research + v1.2 strategy + condition-driven staging + 4→3 pillar decision) | Generated: 2026-05-06 | Research method: 5 parallel AI agents + 4 rounds of codex feedback*
