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

### F1 · Backport rustls CryptoProvider fix to 9 Rust services ✅
- **Status**: ✅ 完成 — branch `fix/rustls-crypto-provider-backport` (commit `b3b1abf`)
- **Result**: 真 Rust stack 完全 boot；real gpt-4o-mini 呼叫 OK：`output='Hello there, friend!'`
- **Build perf**: cargo cache 把 rebuild 從 30 min 縮到 3 min
- **Modified**: 9 services × (main.rs + Cargo.toml) = 18 files, 63 insertions
- **Notes**: auth + leases lib-only，無需動。9 patched: canonical_ingest / control_plane / dashboard / doctor / endpoint_catalog / ledger / retention_sweeper / sidecar / usage_poller
- **次要 follow-up**: F2 — verify-step7 SQL 寫死 Mock LLM token 數，real OpenAI 變動 → 見下方

### F2 · `make demo-verify-step7` brittleness with real OpenAI 🟡
- **Status**: 🟡 follow-up — 不阻擋 V1 後續，product behavior 正常
- **Symptom**: `ERROR: EXPECTED available_budget balance 458; got 482` (差 24 atomic units)
- **Root cause**: verify-step7 SQL 寫死 Mock LLM 回 ~42 token 的預期值，real gpt-4o-mini 變動回應
- **Fix options**:
  1. Makefile guard：`DEMO_MODE=agent_real` 時 skip verify-step7（一行）
  2. SQL assertion 改 range（>0、合理上界）
  3. 寫獨立 `verify-step7-real`
- **Recommended**: option 1 短期解、option 3 長期
- **Branch**: `fix/verify-step7-real-openai` 或合併到 P2-4 prep

### V1 · Real-stack LangChain end-to-end verification ⏳ BLOCKED on F1
- **Why**：rustls 0.23.40（PR #35 Rust toolchain bump 帶進來）requires explicit `CryptoProvider::install_default()`。round2 新加的 3 個 service（outbox_forwarder / ttl_sweeper / webhook_receiver）有修；其餘 11 個漏修：auth / canonical_ingest / control_plane / dashboard / doctor / endpoint_catalog / leases / ledger / retention_sweeper / sidecar / usage_poller
- **Symptom**：`make demo-up DEMO_MODE=agent_real` 失敗，ledger + canonical_ingest 立即 panic
- **Fix**：每個 service `main()` 第一行加：
  ```rust
  rustls::crypto::aws_lc_rs::default_provider()
      .install_default()
      .map_err(|_| anyhow::anyhow!("failed to install rustls aws_lc_rs default provider"))?;
  ```
- **Branch**：`fix/rustls-crypto-provider-backport`
- **Block 下游**：V1 必須等 F1 ✅ 才能繼續
- **Detail**：[`docs/launches/v1-phase1-bug-report.md`](./launches/v1-phase1-bug-report.md)
- **預估**：30 min 改 + 30-60 min 重 build + smoke test = ~1.5-2.5 小時

### V1 · Real-stack LangChain end-to-end verification 🔄 Phase 1 done, P2 next
- **Status**: 🔄 進行中
- **Phase 1**：✅ 完成 — F1 fix 驗證後，real Rust stack + real OpenAI 整條跑通
- **Phase 2**：🔴 開放 — 是否新增 `agent_real_langchain` mode（看 `agent_real_langgraph` 是否已涵蓋 LangChain 需求）
- **Phase 3**：🔴 開放 — 4 個 decision path（CONTINUE / STOP / REQUIRE_APPROVAL / DEGRADE）真 stack 驗證
- **Phase 4**：🔴 開放 — 寫 `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md` + codex review
- **Why**：M1 benchmark 用的是 `spendguard_shim/`（minimal reservation gateway, **不是真的 Rust sidecar**）跑 mock LLM。沒有真 LangChain → 真 sidecar stack → 真 OpenAI 的 e2e 證據前，任何 framework upstream PR (P2-4) 都會被 close 為 premature
- **Scope**：跑通 `make demo-up DEMO_MODE=agent_real_langchain`（如不存在則新建），對 4 個 decision path（CONTINUE / STOP / REQUIRE_APPROVAL / DEGRADE）都收 evidence
- **產出**：
  - `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md` — 環境 + 步驟 + 4 path evidence + bug list
  - `benchmarks/real-stack-e2e/evidence/{continue,stop,approval,degrade}.log`
  - `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.review.md` — codex challenge
  - 可能：`agent_real_langchain` demo mode 新建（如不存在）
- **可能的失敗結果（也合法）**：跑不通 → STATUS=BLOCKED + bug report，這本身就是價值
- **Branch**：`feat/v1-real-stack-langchain-e2e`
- **Codex challenge**：✅ 必做（從 LangChain maintainer 視角）
- **Block 下游**：P2-4 必須等 V1 ✅ 才能啟動
- **Prompt**：`docs/launches/v1-real-stack-e2e-prompt.md`
- **預估**：1-2 天 + ~$1-3 OpenAI token

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

### B2 · README 第一屏視覺改善 ✅
- **Status**: ✅ 完成 — branch `docs/p1-2-3-b2-receipts-demo`
- **改了什麼**: README.md 第 26-50 行的 ASCII flow diagram 換成真 benchmark headline table（Agentic SpendGuard −10% / agentbudget +8% / agent-guard +1700%），引向 `benchmarks/runaway-loop/`
- **不用假 screenshot**: code block 形式更 diff-friendly，數字直接從 RESULTS.md 抄

### B3 · 文件站新 brand 上線驗證
- **Why**：確認自動化沒壞
- **動作**：
  1. 開 https://agenticspendguard.dev/ 看 H1 是否顯示「Agentic SpendGuard」
  2. 檢查 OG meta tags（用 https://www.opengraph.xyz/ 貼網址）
  3. 檢查 Google Search Console 抓取狀態（人工）
- **預估工時**：10 分鐘

---

## 🟡 Tier 2：playbook P1（兩週內）

### P1-1 · 競品對比矩陣重寫 ✅
- **Status**: ✅ 完成 — branch `docs/p1-1-competitor-matrix`
- **位置**: README.md「How this compares to other LLM cost tools」section（Why this exists 與 Quick start 之間）
- **設計**: 兩段
  1. **Direct head-to-head（benchmark-verified）**: 只列 SpendGuard / AgentBudget / AgentGuard，因為這三個有 benchmark 數據；codex must-fix #1 的根因（其他 columns 沒實測）規避
  2. **Adjacent categories（different problems）**: Helicone/Portkey/LiteLLM/TrueFoundry/LangSmith 在獨立 table，每個附「why it's not in the matrix」說明，加 disclaimer「reservation/audit/approval 組合語義不是他們不做 cap」

### P1-2 · Receipt 截圖（真 demo data）✅
- **Status**: ✅ 完成 — branch `docs/p1-2-3-b2-receipts-demo`
- **產出**:
  - `benchmarks/runaway-loop/sample-receipts/spendguard-ledger.jsonl` — 真 ledger reserve→commit→reserve_denied 序列
  - `benchmarks/runaway-loop/sample-receipts/mock-llm-calls.jsonl` — wire calls source-of-truth
  - `benchmarks/runaway-loop/sample-receipts/README.md` — 解釋 receipts，含 ASCII rendering
- **自動化**: analyzer 每次 `make benchmark` 自動 snapshot 到 sample-receipts/

### P1-3 · 12 秒 GIF demo ✅
- **Status**: ✅ 完成 — branch `docs/p1-2-3-b2-receipts-demo`
- **產出**:
  - `benchmarks/runaway-loop/cast/runaway-loop.cast` — asciinema cast (~10s, 2.5 KB)
  - `benchmarks/runaway-loop/cast/record.sh` — re-recording script (pre-builds，只錄 runner+analyzer phase)
  - `benchmarks/runaway-loop/cast/README.md` — play / GIF 轉檔指引
- **GIF 生成**: `brew install agg && agg cast/runaway-loop.cast cast/runaway-loop.gif`（user 本地跑，repo 不存 binary）

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
