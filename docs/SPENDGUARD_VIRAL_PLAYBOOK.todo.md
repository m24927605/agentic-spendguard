# Agentic SpendGuard — Open TODO Tracker

> 開放工作追蹤。companion to:
> - [`SPENDGUARD_VIRAL_PLAYBOOK.md`](./SPENDGUARD_VIRAL_PLAYBOOK.md) — 戰略 plan
> - [`SPENDGUARD_VIRAL_PLAYBOOK.review.md`](./SPENDGUARD_VIRAL_PLAYBOOK.review.md) — codex 對抗審查
>
> **建立**：2026-05-13 · **本次更新**：2026-05-13

---

## 狀態快照

### ✅ 已完成（不用追了）
- 10 trending repo 深度研究（5 個 Staff 級 agent 並行）
- 戰略 playbook 落筆 + codex 對抗審查
- SERP / 商標 / 同領域競品研究 → 發現 AgentGuard (162⭐) + AgentBudget 是真競品
- Brand 決定：永遠用「Agentic SpendGuard」全名
- README + 4 個高曝光 .md 檔 rebrand
- GitHub repo description + 13 topics + homepage URL 更新
- `agenticspendguard.dev` 註冊 + DNS + GitHub Pages CNAME 設定
- 全部 commit 已 merge 到 `main`，HEAD: `6a6627e`

### ✅ 自動完成（已驗證）
- **A1**：Let's Encrypt 憑證簽發 ✅ — 2026-05-13 完成，apex + www 雙包，2026-08-11 到期自動 renew
- **A2**：docs-deploy GitHub Action ✅ — 已部署，`<title>Agentic SpendGuard</title>` 確認在線
- **B3**：文件站新 brand 上線驗證 ✅ — `https://agenticspendguard.dev/` HTTP/2 200，HTTP→HTTPS 301，brand 文字多處出現

### 🔴 開放工作（依 leverage 排序）

---

## 🔥 Tier 0：真 moat — 多日工作

### M1 · P0-1 Benchmark harness vs AgentGuard / AgentBudget ✅
- **Status**: ✅ 完成 — branch `feat/m1-benchmark-runaway-loop`, 3 commits
- **產出**:
  - `benchmarks/runaway-loop/{compose.yml,Makefile,README.md,RESULTS.md,scenario.yaml}`
  - `benchmarks/runaway-loop/mock_llm/` — FastAPI mock OpenAI endpoint
  - `benchmarks/runaway-loop/runners/{agentbudget,agentguard,spendguard}/`
  - `benchmarks/runaway-loop/spendguard_shim/` — minimal reservation gateway
  - `benchmarks/runaway-loop/analyze/analyze.py` — pricing-table aggregator
- **Result** ($1.00 budget, $0.18/call, gpt-4o):
  - SpendGuard: 5 wire calls, $0.90 (**-10%**), ReservationDenied at #6
  - AgentBudget: 6 wire calls, $1.08 (+8%), BudgetExhausted at #6
  - AgentGuard: 100 wire calls, $18.00 (+1700%), no abort (self-hosted endpoint bypass)
- **Codex challenge**: ✅ 跑了 medium reasoning, 20 issues 提出, must-fix subset (9 個) 已修
- **Follow-up open issues** (separate commits later):
  - Real-sidecar runner (currently uses shim)
  - Noisy-reservation scenario (release-path coverage)
  - Concurrent-agent scenario

### M2 · P0-4 Platform engineer / CTO outreach list ✅
- **Status**: ✅ 完成 — branch `docs/m2-outreach`
- **產出**:
  - `docs/launches/cold-email-template.md` — 公開模板（≤180 字 cold-email + channel guidance）
  - `docs/launches/outreach-list.template.md` — schema 模板進 git
  - `.gitignore`: `docs/launches/outreach-list.md` 阻擋實際 list 進 git
- **下一步（user fills locally）**: 用 schema 填 10 個 trigger-driven 目標到本地 `outreach-list.md`

---

## ⚡ Tier 1：brand & launch hygiene — 30 分鐘

### B1 · 35 個剩餘 .md 檔案 brand sweep ✅
- **Status**: ✅ 完成 — branch `docs/brand-sweep-internal`, merged
- **方法**: `perl -i -0pe 's/\bSpendGuard\b(?![a-zA-Z0-9_-])/Agentic SpendGuard/'` — 只改第一個 standalone-word，保護所有 code identifier
- **改了**: 20 files (有 first user-facing mention 的)
- **沒改的對應**: 18 files 沒有 standalone "SpendGuard"（純 component docs / 全是 code identifier）— 不需要動

### B2 · README 第一屏視覺改善
- **Why**：playbook P0-2，但等 benchmark 才能用真實數字
- **現況**：README:28-35 是 ASCII 流程圖
- **改成什麼**：暫時用 `<pre>` 包的 terminal screenshot（真 demo 跑出來的，非假數字）；正式 receipt 截圖等 M1 benchmark 完成
- **Branch**：`docs/readme-first-screen`
- **Block on**：M1 完成才能用真數字

### B3 · 文件站新 brand 上線驗證
- **Why**：確認自動化沒壞
- **動作**：
  1. 開 https://agenticspendguard.dev/ 看 H1 是否顯示「Agentic SpendGuard」
  2. 檢查 OG meta tags（用 https://www.opengraph.xyz/ 貼網址）
  3. 檢查 Google Search Console 抓取狀態（人工）
- **預估工時**：10 分鐘

---

## 🟡 Tier 2：playbook P1（兩週內）

### P1-1 · 競品對比矩陣重寫
- **Why**：codex must-fix #1。原矩陣材料性錯誤
- **新 columns**（codex 建議）：
  - agent-step budget reservation
  - mid-stream abort
  - signed audit chain
  - approval pause/resume
  - framework-native wrappers
  - self-hosted enforcement
- **新 rows**：SpendGuard / **AgentGuard** / **AgentBudget** / Portkey / LiteLLM / TrueFoundry / Helicone
- **加 disclaimer**：「Helicone Vault, Portkey virtual keys, TrueFoundry budget rules, LiteLLM max-budget 等都做某種 budget — SpendGuard 的差異是 reservation/audit/approval 的組合語義，不是『他們不做 cap』」
- **Block on**：M1 benchmark 完成後可填數字

### P1-2 · Receipt 截圖（真 demo data）
- **Block on**：M1 benchmark + B2

### P1-3 · 12 秒 GIF demo
- **Block on**：M1 benchmark

### P1-4 · From-Scratch 7 章 TOC + 前 3 章內文
- **Why**：playbook §4 P1-3，Raschka 模式
- **Scope**：在 README 加 TOC + 寫前 3 章（500 字以上 each）
- **預估工時**：每章 2–4 小時

---

## 🟢 Tier 3：playbook P2（1 個月）

### P2-1 · 公開「最燒錢 agent 排行榜」
- `spendguard scan <github-url>` CLI + `bench.agenticspendguard.dev` 排行榜頁
- **前置**：codex assumption #5 — 先私訊 5 個 maintainer 確認語氣

### P2-2 · 5 個 named spend-skills
- `skills/cap-per-task.md` 等 5 個獨立檔案

### P2-3 · CSV cost-audit hosted 工具
- `audit.agenticspendguard.dev`，使用者貼 OpenAI usage CSV → 顯示「會擋下哪些」
- **不投放**：CFO Slack（codex must-fix #3）
- **投放**：r/ChatGPTPro / LangChain Discord / r/LocalLLaMA

### P2-4 · LangChain / LangGraph / CrewAI 官方 docs PR
- 在每個 framework 的 cost-management 章節加 SpendGuard

---

## ❌ 不做清單（per playbook §7）

- 持續學習 / auto-optimization
- SaaS-level UI polish
- Paid ads（Google / Twitter）
- Influencer 業配
- 多語 SDK（除 Python） — 90 天後 stars 過 1k 再考慮
- Azure / GCP 部署 templates — 只 AWS
- 全功能 hosted SaaS

---

## 工作流程慣例

### 狀態符號
- 🔴 開放（pending）
- 🔄 進行中（in progress）
- ⏳ 等外部（blocked / waiting）
- ✅ 完成（done）
- ❌ 取消（killed）

### Branch 命名
- `feat/<id>-<slug>` — 新功能（M1, P2-* 之類）
- `docs/<id>-<slug>` — 文件 / brand
- `fix/<id>-<slug>` — bug fix

### Commit message
- 開頭加 `[<TODO_ID>]` 方便對應追蹤，例如：
  ```
  feat(benchmarks): runaway loop scenario [M1]
  docs(brand): sweep internal service READMEs [B1]
  ```

### 完成後更新本檔
- 該項狀態 🔴 → ✅
- 加上 commit hash 連結
- 如有 follow-up 開新項目

### Codex challenge 要求
| Tier | 是否需要 codex challenge |
|---|---|
| Tier 0（真 moat）| ✅ 一定要 |
| Tier 1（brand sweep / 視覺）| ❌ 不需要 |
| Tier 2（P1）| ✅ 對矩陣 / 教材內容做 |
| Tier 3（P2）| ✅ 對對外 launch 內容做 |

---

## 全部完成後的下一階段

當 M1 + P1-1 + P1-2 + P1-3 都完成 → 觸發 HN launch 條件評估（見 `docs/launches/hn-show-hn-draft.md` 第 3 個 precondition）

當 stars 過 1k → 重新評估命名（是否需要進一步從「Agentic SpendGuard」轉成更短的 sharper 名稱）
