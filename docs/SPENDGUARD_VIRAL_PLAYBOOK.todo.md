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

### M1 · P0-1 Benchmark harness vs AgentGuard / AgentBudget
- **Why**：codex 的 #1 must-fix。沒這個 launch 會被當場打臉。「Prove the wedge before polishing the wrapper」
- **Scope**：建立 `benchmarks/runaway-loop/` — fixture multi-step agent runaway scenario，跑在 SpendGuard / AgentGuard / AgentBudget 三個對照組上，記錄 mid-stream abort 時機與成本差異
- **產出**：
  - `benchmarks/runaway-loop/scenario.py` — 可重現的 runaway agent
  - `benchmarks/runaway-loop/configs/{spendguard,agentguard,agentbudget}.yaml`
  - `benchmarks/runaway-loop/docker-compose.yml`
  - `benchmarks/runaway-loop/RESULTS.md` — 數字 + 分析
  - `benchmarks/README.md` — 對外宣告 + 可重現步驟
- **成功指標**：3 個工具的對照數據可被第三方 reproduce；至少有一個維度 SpendGuard 顯著勝出（預期：approval workflow 或 multi-tenant 場景）
- **預估工時**：3–5 天（含 token 預算）
- **Branch**：`feat/benchmark-runaway-loop`
- **Codex challenge**：完成後對 RESULTS.md 跑一次

### M2 · P0-4 Platform engineer / CTO outreach list
- **Why**：codex「real ICP」回饋 — 真受眾不是 CFO，是 platform engineering / AI infra leads
- **Scope**：列 10 個目標公司 + 對應 platform engineer / AI infra lead 連絡人 + 一份冷郵範本
- **產出**：
  - `docs/launches/outreach-list.md`（**不要進 git** — 含個資；放本地或 1Password）
  - `docs/launches/cold-email-template.md`（公開可進 git，去個資化）
- **預估工時**：1 天研究 + 0.5 天寫範本

---

## ⚡ Tier 1：brand & launch hygiene — 30 分鐘

### B1 · 35 個剩餘 .md 檔案 brand sweep
- **Why**：brand 一致性。已掃 5 個高曝光面，內部還有 35 個用 bare「SpendGuard」
- **Scope**：以下範疇的檔案，把 H1 / 開頭段 / page title 改用「Agentic SpendGuard」全名（內文後續「SpendGuard」OK）：
  - `docs/site/docs/use-cases/*.md` (3 files)
  - `docs/site/docs/integrations/*.md` (4 files)
  - `docs/site/docs/concepts/*.md` (4 files)
  - `docs/site/docs/deployment/*.md` (2 files)
  - `docs/site/docs/operations/*.md` (含 drills) (~8 files)
  - `docs/site/docs/reference/*.md` (~3 files)
  - `docs/site/docs/poc-vs-ga.md`
  - `docs/site/docs/roadmap/*.md`
  - `services/*/README.md` (8 files)
  - `terraform/aws/README.md`、`charts/spendguard/README.md`
  - 跳過：CHANGELOG.md（歷史性質）、PHASE_4_*.md（內部報告）
- **不要動**：code identifiers (`SpendGuardClient` etc.)、PyPI package name、Helm chart name、Docker images、proto messages
- **Branch**：`docs/brand-sweep-internal`
- **Codex challenge**：不需要

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
