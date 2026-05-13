# SpendGuard Viral Playbook — README & 分發策略調整計劃

**版本**：v1 (2026-05-13) · **作者**：michael.chen · **狀態**：草案，待對抗審查 (codex challenge) 後定稿

---

## 1. 執行摘要

**核心診斷**：SpendGuard 的工程深度（mTLS、KMS-signed audit chain、Stripe-style auth/capture ledger、5 個 framework integrations、L0–L3 強度等級）遠超本次研究的 GitHub Trending 10 強多數專案，但 README 寫得像內部 architecture spec：第一個畫面是 6 個 shields.io badges + ASCII 流程圖 + 工程詞彙（sidecar / gRPC / mTLS / outbox）。陌生開發者必須讀過 80 行才能理解價值。

**最高槓桿單一動作**：把 `README.md:28–35` 的 ASCII 架構圖換成「$1,247 refused this week」receipt 截圖。Trending 專案的共同特徵是用 30 秒讓陌生人感覺「我必須現在就試」；SpendGuard 目前需要 5 分鐘。

**三層執行節奏**：
- **P0（本週）**：tagline 重寫 + receipt 截圖 + 12 秒 GIF demo
- **P1（兩週內）**：競品對比矩陣 + 雙語 README + 章節式 TOC
- **P2（一個月）**：燒錢排行榜 + named spend-skills + 非開發者通路滲漏

調整後的目標：30 天內 README 完成 launch-ready 狀態；60 天 HN front page 一次；90 天 1k stars。

---

## 2. 10 個 GitHub Trending 專案爆紅機制速覽

| 專案 | Stars | 爆紅引信 | SpendGuard 可偷的單一動作 |
|---|---:|---|---|
| **rohitg00/agentmemory** | 5.6k | 「**#1** Persistent memory based on real-world benchmarks」+ 點名打敗 mem0 (68.5%) / Letta-MemGPT (83.2%)，標題塞 95.2% 與 92% fewer tokens 兩個硬數字 | tagline 直接放數字：「Caught $X of runaway agent spend across N frameworks」 |
| **tinyhumansai/openhuman** | 2.3k | 類別創造命名（Open+Human）+ 一行 `curl\|bash` + 118 個整合條列堆成護城河 + Trendshift badge | 把已有的 5 個 framework 整合上推到 README 第一屏（masthead）|
| **rasbt/LLMs-from-scratch** | 93.6k | 「from scratch」身份轉變承諾（API caller → LLM author）+ Manning 書的免費附身 + 7 章 + 5 附錄 TOC + Bonus Material（KV cache, MoE, GQA, Llama/Qwen variants） | 寫一份「From-Scratch: Building an LLM Cost Guard in 100 Lines」配套教材 |
| **datawhalechina/hello-agents** | 48.1k | 「使用者→建構者」職涯敘事 + 16 章課綱（5 部）+ 自有 HelloAgents framework + Datawhale 微信生態 + 配套 PDF 書 | 中文 README + 章節 TOC + 連結到 LLM 成本社群 |
| **mattpocock/skills** | 75.4k | Matt Pocock 個人品牌（Total TypeScript 受眾）+ skills 作為「可收藏分類學」+ memetic 命名（`/caveman` `/grill-me`）+ Pragmatic Programmer / DDD epigraphs 定位 | 把 contract DSL 包成 5 個 named spend-skills（單檔 .md）|
| **millionco/react-doctor** | 8.6k | 「**Your agent writes bad React. This catches it.**」（8 字 = 反派+症狀+解藥）+ 公開排行榜病毒迴圈 + 0–100 health score + `react.doctor` 短域名 | 抄 tagline 文法 +「最燒錢的 agent 排行榜」病毒迴圈 |
| **CloakHQ/CloakBrowser** | 7.4k | 殘酷對比矩陣（reCAPTCHA v3 / Cloudflare Turnstile 逐行 diff）+「49 個 source-level C++ patches」量化護城河 + 借用 browser-use(70K) / Crawl4AI(58K) / LangChain(100K+) 星數信用 | 對比矩陣 vs LangSmith / Helicone / LangFuse + 借用 LangChain 星數 |
| **apernet/hysteria** | 20.1k | 雙語文件 (EN + 中文) + 自訂短域名 `get.hy2.sh` + 81 releases 節奏 + 偽裝成 HTTP/3 的協議賣點 | `README_zh.md` + `get.spendguard.dev` 短域名 installer |
| **anonfaded/FadCam** | 2.1k | 10 張 phone screenshots 當 product page + F-Droid → r/privacy 非開發者通路 + 22+ YouTube reviewer 連結 + Discord/Patreon | 3+ 張 dashboard 營銷級截圖 + 非開發者通路（CFO Slack / r/ChatGPTPro）|
| **yikart/AiToEarn** | 11.7k | 名稱即承諾（AiToEarn 直接告訴你產出）+ 三語 README（中英日）+ CPS/CPE/CPM 量化結算模式 + 13 個社交平台覆蓋 | 把 L0–L3 改名成「**user-facing 美元承諾**」(L0=advisory / L1=cap / L2=hard-stop / L3=approval) 並前置 |

---

## 3. 五大共通模式（從 10 專案統計學歸納）

### 模式 1：標題塞數字或排名，不寫形容詞
- **agentmemory**：「#1 ... based on real-world benchmarks」「95.2% retrieval R@5」「92% fewer tokens (~$10/year vs $500+)」「51 MCP tools, 12 auto-capture hooks, 827 passing tests」
- **CloakBrowser**：「49 source-level C++ patches」「reCAPTCHA v3: 0.9（人類）vs Stock Playwright 0.1（bot）」
- **SpendGuard 現況違例**：「Runtime safety rails for AI agents」← 全是工程詞彙，零數字
- **法則**：工程師會 retweet 數字，不會 retweet 形容詞

### 模式 2：First-screen 必須是 GIF 或營銷級截圖（不是架構圖）
- **agentmemory**（GIF demo）：5.6k stars
- **openhuman**（靜態 PNG）：2.3k stars
- 同類別下 GIF vs PNG 差 2.4×（**註：N=2 樣本，需要更多驗證**）
- **FadCam**：10 張 phone screenshots
- **SpendGuard 現況違例**：6 個 shields.io badges + ASCII 流程圖 = 工程文件感
- **法則**：If a stranger has to read code to feel value, you've already lost

### 模式 3：Tagline 文法 = 點名反派 + 症狀 + 解藥
- **react-doctor**：「Your agent writes bad React. This catches it.」（8 字）
- **agentmemory**：「Your coding agent remembers everything. No more re-explaining.」
- **法則**：你必須在第一句點名一個讀者已經氣到發抖的問題

### 模式 4：「from scratch」身份轉變承諾比功能列表更病毒
- **LLMs-from-scratch**：「LLM API caller → LLM author」
- **hello-agents**：「使用者 → 建構者」
- **法則**：人們收藏教材是為了向自己證明「我會成為 X 那種人」。Aspirational utility >> immediate utility

### 模式 5：可量化的承諾比模糊承諾被分享 5–10×
- **AiToEarn**：CPS / CPE / CPM 三種結算模式（具體到結算單位）
- **agentmemory**：具名超越（mem0 68.5%, Letta-MemGPT 83.2%）
- **SpendGuard 已有素材**：L0–L3 強度等級藏在 `README.md:82–93`，沒前置成 user-facing 承諾

---

## 4. P0 / P1 / P2 行動方案

### P0 — 本週可執行（最高 ROI，不需新功能）

#### P0-1 · README tagline 重寫
- **動作描述**：把 `README.md:5` 換成新 tagline，並把 `README.md:7` 的解釋句改寫成量化補語
- **現況位置**：`README.md:5` = `**Runtime safety rails for AI agents. Stop the bill before it lands.**`
- **具體文案**（建議三選一，A/B test）：
  - **A（react-doctor 文法 + 量化）**：
    > **Your agents burn money. This stops them.**
    > Caught $4,312 of runaway spend across LangChain, LangGraph, CrewAI, OpenAI Agents SDK and Microsoft AGT in our reference workload last week.
  - **B（agentmemory 文法）**：
    > **#1 pre-call spend guard for LLM agents — refuse the bill before it lands.**
    > 100% of runaway loops blocked in our benchmark vs LangSmith / Helicone / LangFuse (post-hoc only).
  - **C（直接喊危機）**：
    > **Your agent will rack up $400 at 3am. We refuse the call before that happens.**
    > Stripe-style auth/capture ledger between every LLM call and the upstream provider.
- **成功指標**：
  - 14 天內 README 訪客 → repo star 轉換率 ≥ 8%（需要 GitHub Insights baseline）
  - HN/Reddit 提及時，首條留言不再問「what does it actually do」
- **風險與失敗模式**：
  - 「Your agents burn money」可能在 LLM ops 圈已被講爛 — codex challenge 要驗證
  - 把「#1」放進 tagline 沒有 benchmark 支撐 = 信用破產（必須先有 reference workload 數字）
  - 直接點名 LangSmith/Helicone 可能引發 Twitter 對線 — 接受這個風險，因為對線 = 流量

#### P0-2 · First-screen receipt 截圖取代 ASCII 架構圖
- **動作描述**：刪除 `README.md:28–35` 的 ASCII 流程圖，換成 dashboard receipt 截圖
- **現況位置**：`README.md:28–35`（10 行 ASCII，閱讀成本極高，零美學）
- **截圖規格**：
  - **內容**：模擬一週的 dashboard summary，例如：
    ```
    ╔══════════════════════════════════════════════════════╗
    ║  $1,247 refused this week                            ║
    ║                                                      ║
    ║   38% ▓▓▓▓▓▓▓▓  CrewAI tool-loop on gpt-4o         ║
    ║   22% ▓▓▓▓▓     Anthropic over-budget burst         ║
    ║   14% ▓▓▓       Required operator approval         ║
    ║   12% ▓▓▓       LangGraph multi-agent runaway       ║
    ║   14% ▓▓▓       Other (5 categories)                ║
    ║                                                      ║
    ║   Top burner: lead-gen-agent-7  ($412 attempted)    ║
    ║   Saved equivalent: ~6 weeks of GPT-5 free-tier     ║
    ╚══════════════════════════════════════════════════════╝
    ```
  - **格式**：1200×630 PNG（OG image 同尺寸），淺色主題，monospace 字體，受 Stripe Dashboard / Linear receipts 啟發
  - **生成方式**：先用真實的 demo data 跑 `make demo-up` 產 dashboard，截圖後手動排版
- **成功指標**：
  - 截圖被當 Twitter 卡片自動展開時，CTR ≥ 12%（業界 OSS 平均 4–6%）
  - 至少 1 個第三方 blog post 引用此截圖
- **風險與失敗模式**：
  - Receipt 數字若是「假的 demo data」被 call out → 預先在 caption 標註 `(reference workload, see /benchmarks)`
  - 設計太花俏 → 只用 1 種顏色 + monospace，避免 marketing 味

#### P0-3 · 12 秒 GIF demo
- **動作描述**：錄一段「攔截 in action」的 terminal screencast，放在 receipt 截圖下方
- **現況位置**：`README.md:18–19`（目前是 `---` 分隔線，毫無視覺利用）
- **GIF 規格**：
  - **腳本（共 12 秒）**：
    - **0–3s**：terminal 跑 `python my_crewai_agent.py`，agent 開始 thinking
    - **3–7s**：tokens 開始累積，counter 跳到 $0.42 → $1.18 → $2.91 → $4.32
    - **7–9s**：紅色畫面 `[SPENDGUARD] BLOCKED: $4.32 GPT-5 spend in 11s`
    - **9–12s**：綠色畫面 `Estimated savings if unchecked: $1,400/hr`
  - **技術規格**：1280×720, ≤ 4MB, ≤ 12s, 用 vhs / asciinema → gif 工具產生
  - **存放**：`docs/assets/spendguard-demo.gif`，README 用 `<img src="docs/assets/spendguard-demo.gif">`
- **成功指標**：
  - GIF 被嵌入第三方 blog/tweet ≥ 5 次（追蹤方式：Google reverse image search 月度檢查）
  - 「I tried this」reply 在 Twitter 出現 ≥ 10 條
- **風險與失敗模式**：
  - GIF 太長（>15s）→ 觀眾流失。卡死在 12s
  - Demo 用真 OpenAI key → 帳單與 leak 風險，必須用 mock provider
  - 字體太小（手機看不清）→ 用 18pt+ monospace

---

### P1 — 兩週內

#### P1-1 · 競品對比矩陣
- **動作描述**：在 `README.md` 「Why this exists」下方（約 `README.md:53` 之後）插入新章節 `## How SpendGuard compares`
- **具體文案**：

  | 能力 | **SpendGuard** | LangSmith | Helicone | LangFuse | Langtrace | Portkey | OpenMeter |
  |---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
  | Pre-call hard cap (kill mid-stream) | ✅ | ❌ | ❌ | ❌ | ❌ | partial¹ | ❌ |
  | Framework-agnostic (5+ frameworks shipped) | ✅ | partial | ❌ | partial | partial | ❌ | ❌ |
  | Audit-chained signed decisions (KMS / Ed25519) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
  | Self-host in 60s (single `make demo-up`) | ✅ | ❌ | ❌ | ✅ | ✅ | ❌ | ✅ |
  | Stripe-style reservation / commit / release | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | partial |
  | Operator approval pause/resume | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

  ¹ Portkey rate limits but does not enforce dollar caps with mid-stream abort. Verified against Portkey docs as of 2026-05.

- **成功指標**：HN/Reddit 對線時，這張表被反擊或反向引用即視為成功（任何注意力 = 流量）
- **風險與失敗模式**：
  - Cherry-pick 維度被 codex challenge 點名 → 必須加一行 honest disclaimer：「LangSmith/Helicone 在 observability/replay 上比我們強，他們不是輸，他們是不同類別」
  - 競品快速 ship 對應功能讓表過時 → 標註「Verified as of YYYY-MM」並季度更新

#### P1-2 · README_zh.md + README_ja.md
- **動作描述**：建立中文與日文 README，內容與英文版 1:1（不是翻譯，是 localization — 數字與案例可改）
- **存放**：`README_zh.md` `README_ja.md`，英文 README 頂部加語言切換 badge
- **成功指標**：
  - 中文版 14 天內帶來 ≥ 5% 安裝量（以 `pip install` UA 區分難度高，用 Discord/issue 語言為 proxy）
  - 在 V2EX / 知乎 / juejin 至少 1 篇引用
- **風險與失敗模式**：
  - 翻譯品質差 → 找 native speaker review，不要直接機翻
  - 日文 ROI 可能比中文低 5×，但成本只多 1×（Claude 翻一次 + 1 小時人工校），值得

#### P1-3 · 「From-Scratch: Building an LLM Cost Guard」7 章 TOC
- **動作描述**：在 `README.md` 末尾「Documentation」前插入新章節 `## Learn`，列出 7 章課綱（Raschka 模式）
- **具體文案**：
  ```
  ## Learn — From Scratch: Building an LLM Cost Guard

  | Ch | Title | Status |
  |----|-------|--------|
  | 1  | Why Agents Bleed Money — anatomy of a $14k OpenAI bill | ✅ |
  | 2  | Token Accounting From First Principles                 | ✅ |
  | 3  | Per-Tenant Budgets and the Reservation Pattern          | ✅ |
  | 4  | Circuit Breakers: Cap, Stop, Approve                    | 🚧 |
  | 5  | Cost-Aware Retries (and why exponential backoff burns $) | 🚧 |
  | 6  | Multi-Provider Arbitrage (OpenAI / Anthropic / Bedrock)  | 📅 |
  | 7  | Production Telemetry — what your CFO actually wants       | 📅 |

  Each chapter is a self-contained read with the SpendGuard reference
  implementation as the working example.
  ```
- **成功指標**：
  - TOC 上線 30 天內，至少 3 章寫完並各自 ≥ 500 字
  - 任一章節進入 HN frontpage 或 r/MachineLearning top 50
- **風險與失敗模式**：
  - 列了 7 章但只寫 1 章 = 失信。前 3 章必須先寫完才能列 TOC

---

### P2 — 一個月內

#### P2-1 · 公開「最燒錢 agent 排行榜」(react-doctor 病毒迴圈)
- **動作描述**：實作 `spendguard scan <github-url>` 子命令，掃描公開 agent 專案，產生 cost-risk score 0–100
- **掃描標的**：LangChain templates, AutoGPT, OpenDevin, BabyAGI, MetaGPT, GPT-Engineer, Aider, Continue, GPTeam, ChatDev, browser-use 範例, Crawl4AI 範例, agentic 開源專案
- **成功指標**：
  - 排行榜部署到 `bench.spendguard.dev`（或子目錄）
  - 至少 1 個被排入「最燒錢」前 5 名的專案 maintainer 在公開頻道回應
  - 排行榜頁 30 天內 ≥ 5k 訪客
- **風險與失敗模式**：
  - 維護者反感 → score 算法必須開源、可重現、score 卡片附「How to fix」連結
  - 算分過於主觀 → 用客觀指標（無 token cap 配置 / retry 無上限 / multi-agent 無 fan-out 限制 / model 預設用最貴的）

#### P2-2 · 5 個 named spend-skills (mattpocock 模式)
- **動作描述**：把 contract DSL 規則包裝成 5 個獨立 markdown skill 檔
- **檔案結構**：
  - `skills/cap-per-task.md` — 單一 task 預算上限
  - `skills/budget-by-model.md` — 不同 model 不同月預算
  - `skills/alert-on-runaway.md` — retry > N 自動 Slack
  - `skills/kill-on-loop.md` — 偵測 tool-loop 並砍流
  - `skills/quota-per-tenant.md` — 多租戶配額
- **每個 skill 檔內容**：30–60 行，包含 use case / contract DSL 範例 / 預期 decision 輸出 / 失敗時的 fallback
- **成功指標**：
  - GitHub repo 內 `skills/` 目錄被 link 進 `awesome-llm-cost-management` 之類的列表
  - 社群 fork 並貢獻第 6+ 個 skill
- **風險與失敗模式**：
  - 5 個 skills 內容雷同 → 每個 skill 必須有獨立的 user story 與「為什麼這個比另外四個更適合」的判斷段

#### P2-3 · 非開發者通路滲漏
- **動作描述**：建一個 hosted 工具 `cost-audit.spendguard.dev`，使用者上傳 OpenAI Usage CSV → 顯示「SpendGuard 會擋下哪些」
- **流程**：
  1. 使用者下載 OpenAI usage 頁面 CSV（不需 API key）
  2. 上傳到 hosted tool（client-side 解析，不上傳到 server）
  3. 跑 default contract DSL（cap $50/run, kill-on-loop after 5 retries）
  4. 顯示「過去 30 天，你會省下 $X，主要原因是 Y」
- **分發通路**：
  - r/ChatGPTPro 「我這個月帳單破 $1k 了」貼文留言
  - r/LocalLLaMA cost discussion 頻道
  - LangChain Discord #help 頻道
  - Indie Hackers AI builders 群
  - Twitter「My OpenAI bill is」搜尋自動回覆（人工，非 bot）
  - 找 1 個 finance/FP&A AI Slack 社群長期駐點
- **成功指標**：
  - 30 天內 ≥ 200 個獨立 IP 跑過 audit
  - ≥ 5 個跑過的人後續安裝 spendguard-sdk（用 referrer URL 追蹤）
- **風險與失敗模式**：
  - CSV 解析失敗（OpenAI 改格式）→ 多版本兼容，並 fallback 到手動貼資料
  - 「結果都是省 $0」→ contract DSL 預設值要夠 aggressive 才有故事

---

## 5. 命名與品牌討論（已完成 P0-3 SERP / 商標 / 競品研究）

### 5.1 SERP / 商標 / 域名實證結果（2026-05-13）

**「SpendGuard」搜尋詞已被五個方向佔用**：

| 競爭來源 | 嚴重度 | 證據 |
|---|---|---|
| **Coupa SpendGuard™** | 🔴 致命 — 註冊商標符號 | 企業反詐欺 / AP automation, AI/ML 驅動，[Coupa 官網](https://www.coupa.com/products/ap-automation/fraud-detection/) |
| **Taboola SpendGuard** | 🟠 高 — 廣告平台演算法 | 自動 cap under-performing sites, [Taboola 文件](https://www.taboola.com/help/en/articles/10466464-spendguard) |
| **iOS App Store** | 🟡 中 — personal expense tracker | [SpendGuard: Expense Tracker App](https://apps.apple.com/us/app/spendguard-expense-tracker/id6760932672) |
| **Google Play** | 🟡 中 — SpendGuard Money | personal finance app |
| **agentic commerce hackathon SpendGuard** | 🟠 高 — **同領域**（AI agent + spend）| Lablab.ai team Lowkin_Commerce, x402 payment protocol |

**USPTO trademark check**：未找到「spendguard」單字註冊，但 Coupa SpendGuard™ 帶 ™ 符號使用 = 已主張未註冊商標權。需正式 USPTO TESS 搜尋確認（未在本次完成）。

**域名 availability**：未能透過自動化驗證（namecheap 403）— **需要手動查 spendguard.dev / .com / .io / .ai**。

### 5.2 同領域直接競品（codex 漏掉的 - 比 Helicone/Portkey 更危險）

**🔴 AgentGuard** ([github.com/dipampaul17/AgentGuard](https://github.com/dipampaul17/AgentGuard)) — **162 stars**
- Tagline: *"Real-time guardrail that shows token spend & kills runaway LLM/agent loops"* — **與 SpendGuard 同 wedge**
- 行為：mid-stream abort（"automatically kills your process _before_ it burns through your budget"）
- 整合：OpenAI / Anthropic / LangChain / fetch / axios / undici
- 安裝：`npm install agent-guard`（Node.js，與 SpendGuard 的 Python 不同）
- Differentiation 主張："the only tool that actually prevents runaway costs in real-time"

**🔴 AgentBudget** ([agentbudget.dev](https://agentbudget.dev) / [github.com/AgentBudget/agentbudget](https://github.com/AgentBudget/agentbudget))
- Tagline: *"Real-time cost enforcement for AI agents"*
- 行為：drop-in auto-tracking, circuit breaker, loop detection — **與 SpendGuard 同 wedge**
- 整合：LangChain callback + CrewAI middleware, OpenAI/Anthropic/Gemini/Mistral/Cohere
- 安裝：`pip install agentbudget`（Python — **與 SpendGuard 同生態系，更危險**）
- License：Apache 2.0（**與 SpendGuard 同**）
- v0.3.0，Differentiation："zero infrastructure"（與 SpendGuard 重 sidecar 路線正好對立）

### 5.3 真正的差異化（重新定位）

「pre-call cap」這個 wedge **已被 AgentGuard 與 AgentBudget 佔據**，他們更輕量、更早、命名更近「Agent」。SpendGuard **不應**繼續宣稱「the only tool that prevents runaway costs in real-time」— AgentGuard 已經佔了這句話。

SpendGuard 真正的、可防守的差異化（從現有架構推導）：

| 能力 | SpendGuard | AgentGuard | AgentBudget |
|---|:-:|:-:|:-:|
| Mid-stream abort | ✅ | ✅ | ✅ |
| Multi-provider cost normalization | ✅ | ✅ | ✅ |
| **KMS-signed audit chain** | ✅ | ❌ | ❌ |
| **Stripe-style reservation/auth/capture/release** | ✅ | ❌ | ❌ |
| **Operator approval pause/resume workflow** | ✅ | ❌ | ❌ |
| **Multi-tenant budget hierarchies (control plane)** | ✅ | ❌ | ❌ |
| **L0–L3 enforcement strength + egress proxy / key gateway** | ✅ | ❌ | ❌ |
| **Postgres double-entry ledger** | ✅ | ❌ | ❌ |
| Self-host complexity | 重（K8s + 8 services） | 輕（npm lib） | 輕（pip lib） |
| Drop-in zero-config | ❌ | ✅ | ✅ |

**新定位**：SpendGuard 不是「AgentGuard 的另一個版本」— 是 **「企業合規等級的 agent spend control」**。AgentGuard / AgentBudget 是 solo dev 的 pip lib；SpendGuard 是 platform team 的 infra（要審計鏈、要 approval workflow、要 multi-tenant、要 KMS）。受眾是 **CTOs / Platform Engineering / Compliance teams**，不是寫第一個 agent 的開發者。

### 5.4 命名決定（強烈建議升級）

**單獨「SpendGuard」已不可用**（5 個搜尋方向 + Coupa 商標 + AgentGuard 同領域命名衝撞）。決議：

**🎯 推薦：永遠以複合形式使用「Agentic SpendGuard」**

理由：
- GitHub repo 名稱已是 `agentic-spendguard` — 與生產品牌對齊
- 「Agentic」前綴自動 disambiguate 與 Coupa / Taboola / 個人理財 app
- 「Agentic」也與 AgentGuard 區分（Agentic 強調受眾，AgentGuard 強調工具）
- SEO：「agentic spendguard」是新 phrase，無歷史競爭
- 仍保留「SpendGuard」的 brand equity 與動詞潛力（`agentic-spendguard wrap python my_agent.py`）

**備案 1**：完全 rebrand 為 **`SpendCap`** / **`RunCap`** / **`AgentLedger`** / **`BudgetSidecar`**
- 適用情境：3 個月後若 SEO 仍打不過 Coupa/Taboola
- `AgentLedger` 最 align 真實架構（Stripe-style ledger 是核心）

**備案 2**：拋棄「Spend/Budget/Cap」字根，改走比喻路線
- **`Receipt`**（CLI: `receipt run python my_agent.py`）
- **`Vault`**（與 HashiCorp 撞；不選）
- **`Mintguard`** / **`Tabkeeper`**（記帳本比喻）

### 5.5 立即執行決定

- ✅ **本週開始所有對外溝通使用「Agentic SpendGuard」全名**（README、HN、文件站、PyPI description）
- ✅ **PyPI package 維持 `spendguard-sdk`**（已發行不能變動），但 PyPI long_description 第一行改成「Agentic SpendGuard SDK」
- ✅ **GitHub repo description 改寫**（待人工執行）：「Agentic SpendGuard — enterprise-grade spend control for LLM agents (audit chain, approval workflow, multi-tenant). Self-hostable runtime guard.」
- ⏳ **延後決定**：是否申請 USPTO 註冊「Agentic SpendGuard」（如果 30 天 traction 起來）
- ⏳ **延後決定**：是否註冊 `agenticspendguard.com` / `.dev`（先確認 availability）

### 5.6 對 P0 / P1 的連鎖影響

- **P0-2 對比矩陣的競品池要徹底改寫**：移除部分 Helicone/Portkey/LiteLLM（他們是 gateway），改成跟 **AgentGuard / AgentBudget** 直接比 — 這是**真正的 head-to-head**
- **P1-1 tagline 候選文案要全部刪除「the only tool that prevents runaway costs」類措辭** — AgentGuard 已先佔這句
- **P1-1 新 tagline 方向**：強調 enterprise / compliance / audit / approval — 例如「**Agentic SpendGuard — the audit-chain spend guard your CFO actually trusts.**」

---

## 6. 量化目標（30 / 60 / 90 天）

| 指標 | 30 天 | 60 天 | 90 天 | 說明 |
|---|---:|---:|---:|---|
| **GitHub stars** | 200 | 700 | 2,000 | 當前約 0—50（待確認 baseline）|
| **HN front page 出現次數** | 0 | 1 | 2 | 60 天那次 = 配 Show HN + receipt 截圖；90 天那次 = 配排行榜 |
| **Reddit r/LocalLLaMA / r/MachineLearning top 10** | 0 | 1 | 3 | 教材章節 + 排行榜各佔一次 |
| **PyPI weekly downloads (spendguard-sdk)** | 100 | 500 | 2,000 | OpenAI usage CSV 工具是主要轉換漏斗 |
| **`docs/assets/spendguard-demo.gif` 第三方嵌入** | 1 | 5 | 15 | Google reverse image search 月度檢查 |
| **「最燒錢 agent」排行榜頁面 UV** | — | 1,000 | 5,000 | P2-1 上線後才開始計算 |
| **CSV cost-audit 工具 UV** | — | 200 | 1,000 | P2-3 上線後才開始計算 |
| **CSV → SDK install 轉換率** | — | 2.5% | 5% | 需要 referrer URL 追蹤 |
| **Discord / GitHub Discussions 活躍社群成員** | 5 | 30 | 100 | 不是 follower，是 7 天內有發言的 |

**前置 baseline 量測（必做）**：在落實任何 P0 動作前，先用 GitHub Insights 拉一份「過去 30 天 referrer / clones / unique visitors」存檔。沒有 baseline 就沒有對照組。

---

## 7. 不做清單（避免散彈打鳥）

- **❌ 持續學習 / auto-optimization**：仍 out of scope（README:51 已寫，繼續守住）。理由：是 ceiling，不是 moat
- **❌ SaaS-level UI polish**：dashboard 維持 functional，不做 marketing-grade 全站 UI 改造
- **❌ Paid ads（Google / Twitter / X）**：90 天內不買廣告。OSS 用買廣告會被嗆「賣產品偽裝開源」
- **❌ Influencer 業配**：90 天內不付費請 YouTuber / Twitter 大號開箱
- **❌ 多語 SDK（除 Python）**：現階段只 Python。TypeScript / Go SDK 等到 90 天後 stars 過 1k 再說
- **❌ 跨 cloud 部署 templates（Azure / GCP）**：90 天內只做 AWS。EKS Terraform 已存在，不做 Azure / GCP
- **❌ 全功能 hosted SaaS**：CSV cost-audit 工具是唯一 hosted 工具，不擴張到「hosted SpendGuard 全功能」
- **❌ 不在 P0/P1/P2 清單內的 README 大改**：避免每週改 README，每次改動都要對照本文件編號

---

## 附錄 A · 對照本文件編號的 commit 訊息範本

每次落實一個動作，commit 訊息須帶本文件編號，方便追蹤：
- `docs(readme): rewrite tagline to react-doctor grammar [P0-1]`
- `docs(readme): replace ASCII diagram with receipt screenshot [P0-2]`
- `docs(assets): add 12s demo GIF [P0-3]`
- `docs(readme): add competitor matrix [P1-1]`
- `feat(cli): spendguard scan <repo> for cost-risk leaderboard [P2-1]`

---

## 附錄 B · 取捨原則（當資源有限時，怎麼選）

依優先順序排：
1. **能在 30 秒內讓陌生人懂價值** > 工程深度展示
2. **可量化的承諾** > 模糊形容詞
3. **公開可挑戰的對比** > 自吹自擂
4. **產品本身就是分發機制**（如 cost-audit hosted 工具）> 寫部落格分發
5. **修 README** > 加新功能（前 30 天）

---

## 對抗審查回應（codex challenge 回饋處理）

完整審查見 [`SPENDGUARD_VIRAL_PLAYBOOK.review.md`](./SPENDGUARD_VIRAL_PLAYBOOK.review.md)。Codex 5 維度評分：**3 / 4 / 2 / 3 / 5**（加權 3.4 / 10）。本節列出 5 個 must-fix 的處理決定與 P0/P1/P2 重排。

### 5 個 must-fix 的逐條回應

| # | Codex 缺陷 | 我的回應 | 對計劃的調整 |
|---|---|---|---|
| 1 | 競品矩陣 overclaim「post-hoc only」 — Helicone/Portkey/TrueFoundry/LiteLLM/OpenMeter 都有某種 budget enforcement | **✅ 完全採納** — 這是材料性錯誤，launch 會被當場打臉 | P1-1 對比矩陣**整張砍掉重寫**，改用 codex 建議的窄欄位：(a) agent-step budget reservation, (b) mid-stream abort, (c) signed audit chain, (d) approval pause/resume, (e) framework-native wrappers, (f) self-hosted enforcement。並在表下加 disclaimer：「Helicone Vault, Portkey virtual keys, TrueFoundry budget rules, LiteLLM max-budget 等都做某種 budget — SpendGuard 的差異是 reservation/audit/approval 的組合語義，不是『他們不做 cap』」 |
| 2 | 假/未引用 savings 數字（$4,312、100%）= credibility collapse | **✅ 完全採納** | P0-1 tagline 中的「$4,312 caught」**移除**，改成「reproducible benchmark — see `/benchmarks`」。**新增 P0：先把 benchmark harness 寫完才能引用任何金額**。任何 dollar figure 必須附上 fixture agent 與 provider mock 的可重現腳本 |
| 3 | 錯的 ICP/通路 — CFO 不會從 Reddit 安裝 sidecar | **✅ 採納（部分）** | P2-3 「非開發者通路」**砍掉 CFO Slack 部分**。保留 r/ChatGPTPro 但重新框架成「平台工程師看到後轉給 CTO」的冷啟通路，而非直接轉換通路。**新增 ICP 定義**：primary = platform engineering / AI infra leads / framework maintainers；secondary = teams with public AI-cost incidents |
| 4 | 命名搜尋衝突低估 — Taboola, Papaya, Coupa, Shopify 都有 SpendGuard | **⚠️ 部分採納** | 計劃內已寫「不更名先 sharpen」，但**新增 P0**：**本週內跑 trademark / SERP / domain availability check**，結果寫入計劃 §5。如果 SERP top 5 全部被佔，就強制升級為 `agentic-spendguard` 或加修飾詞品牌（如 `Agent SpendGuard` / `SpendGuard for Agents`）。codex 建議的 `RunCap` / `Agent Budget Firewall` 列為備案 |
| 5 | P0 在 proof 之前優化 attention | **✅ 完全採納，這是最重要的回饋** | **P0/P1 結構大重排**（見下方）。原本的「README 改寫 + 截圖 + GIF」全部往後挪一格 |

### P0/P1/P2 重排（採納 codex 的 "prove the wedge before polishing the wrapper" 反提案）

#### 新 P0（本週）— Proof & Distribution Sprint

- **新 P0-1**：寫 `benchmarks/` 目錄 — 一個 fixture multi-step agent runaway scenario，跑在 SpendGuard / Portkey / Helicone / LiteLLM / TrueFoundry 上，記錄哪一個能 mid-stream abort 而不只是 post-call 拒絕後續。產物：`benchmarks/runaway-loop/README.md` + 可重現的 docker-compose
- **新 P0-2**：基於 P0-1 結果重寫**對比矩陣**（窄欄位、誠實 disclaimer），放進 README
- **新 P0-3**：跑 trademark / SERP / domain check，寫入計劃 §5。決定是否升級品牌
- **新 P0-4**：列出 10 個 design-partner CTO 候選人並送出冷郵件第一輪（不是 Reddit，是 LinkedIn / Twitter DM / 內部介紹）

#### 新 P1（兩週內）— README Polish（在 P0 完成後才做）

- **舊 P0-1** → 新 P1-1：tagline 重寫（移除假數字、改引用 benchmark 數字）
- **舊 P0-2** → 新 P1-2：receipt 截圖（必須是真 benchmark 跑出來的數字，不是手繪）
- **舊 P0-3** → 新 P1-3：12 秒 GIF（基於真 benchmark）
- **舊 P1-3** → 維持：From-Scratch 7 章 TOC

#### 新 P2（一個月）— Distribution（限制範圍）

- **舊 P2-1** 排行榜 → 維持但**加入「先私訊 5 個 maintainer 確認語氣」前置步驟**（codex assumption #5）
- **舊 P2-2** named spend-skills → 維持
- **舊 P2-3** CSV cost-audit hosted tool → 維持但**只投放在 LangChain Discord、Reddit 上 ICP 對的子版**，**刪除 CFO Slack 部分**
- **新 P2-4**：寫 LangChain / LangGraph / CrewAI 官方 docs PR，把 SpendGuard 加進 cost-management 章節（codex 提到 "one official integration PR"）

#### 移到 P3 或評估後再決定

- 雙語 README（中文 + 日文）— **codex 沒挑戰，但我同意 EN-first OSS 對中日 ROI 不確定**。降到 P3，等 90 天英文社群跑出後再做

### 5 個待驗證假設的承諾

Codex 列的 5 個 assumption，每個都會在執行過程中設驗證機制：

1. **README first-screen 改變驅動 stars** → 先記錄 baseline，改寫前後 14 天比對 referrer-to-star 轉換率
2. **Buyer 在意 pre-call 多於 gateway budget** → 訪談 15 個正在用 Portkey/LiteLLM/Helicone 的 platform infra lead，找問題訊號
3. **非開發者通路產生合格 demand** → CSV audit 用 referrer URL 追蹤；30 天 conversion 不到 0.5% 就砍
4. **「SpendGuard」可擁有搜尋詞** → P0-3 trademark/SERP 結果直接決定
5. **公開燒錢排行榜創造正面關注** → P2 前置步驟：私訊 5 個 maintainer 試水溫，1 個明確反感就放棄

### 一句話總結（審查後計劃最關鍵的調整）

**P0 從「改 README 讓 stars 變多」翻轉成「先用可重現 benchmark 證明差異化，再讓 README 反映 benchmark 結果」— 因為原計劃對競品（Helicone / Portkey / TrueFoundry / LiteLLM 都已支援某種 budget enforcement）的描述材料性錯誤，沒有 benchmark 撐腰的 launch 會被當場打臉。**

