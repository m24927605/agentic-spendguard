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

### CA · Cost Advisor product spec + P0 prompt ready ✅
- **Status**: ✅ spec GREEN (codex r4) — `docs/specs/cost-advisor-spec.md` (822 lines, v3 after 4 rounds adversarial review)
- **Codex iteration**：r1=4.5 → r2=5.2 → r3=5.0(rescope) → r4=GREEN_LIGHT_FOR_P0。Staff escalation 未觸發
- **核心 rescope** (codex r3)：cost advisor 是 closed-loop feature，不是 separate product；findings → proposed contract patches → 既有 approval queue → operator approve → next sidecar reload
- **P0 prompt ready**：`docs/launches/cost-advisor-p0-prompt.md` — 4 步驟（schema audit / proto+trait / migrations / integration design）
- **P0 預估**：4 天（A2 audit 結果可能 +3-10 天 contingency）
- **P0 後**：P1 (5-6 天) 實作 failed_retry_burn_v1 第一條 rule
- **Total to v0.1**：17 天 per §9 phasing

### CA-P0 · Cost Advisor P0 prep phase complete ✅
- **Status**: ✅ done — branch `feat/cost-advisor-p0` (4 commits, 1671 insertions)
- **Step 1 (schema audit)**：✅ `docs/specs/cost-advisor-p0-audit-report.md`. Verdict: **§11.5 A2 scenario 3** (3+ fields missing + fundamental shape mismatch). 6/7 rule-input fields (prompt_hash, agent_id, run_id, tool_name, tool_args_hash, model_family) are **0% populated** in canonical_events; cost data lives in ledger.commits not in audit payload. **No PII blocker** (no prompt text in audit chain — avoided +5-10d branch).
- **Scope cut (revised §5.1)**：v0.1 ships ONLY `idle_reservation_rate_v1` (fireable via ledger.reservations join). Other 3 rules deferred to P1.5 (after P0.5 sidecar enrichment).
- **Step 2 (proto + crate)**：✅ `proto/spendguard/cost_advisor/v1/cost_advisor.proto` (FindingEvidence + 8 enums per spec §4.0); `services/cost_advisor/` Rust crate with CostRule trait, SqlCostRule adapter, fingerprint::compute (SHA-256 per §11.5 A1, with unit tests), placeholder rule. `cargo check` passes on rust:1.91.
- **Step 3 (migrations)**：✅ 4 migrations:
  - `canonical_ingest/0011_add_failure_class.sql` (column + CHECK + partial index)
  - `cost_advisor/01_cost_findings.sql` (partitioned table per §11.5 A7)
  - `cost_advisor/02_cost_baselines.sql` (28d default window per §11.5 A4)
  - `ledger/0038_approval_requests_proposal_source.sql` (proposal_source + proposed_dsl_patch + proposing_finding_id; strengthened immutability trigger; tenant_data_policy retention windows for cost_findings)
  - new init script + compose mount for cost_advisor migrations
- **Verified**：all migrations apply cleanly against postgres:16-alpine; 4 smoke tests pass (NULL failure_class admitted, cost_findings INSERT works, cost_advisor CHECK rejects missing patch, immutability trigger blocks UPDATE on proposed_dsl_patch).
- **Step 4 (integration design)**：✅ `services/cost_advisor/docs/control-plane-integration.md` — closed loop, schema delta, lifecycle state machine, dashboard filter (no new tab — one URL parameter), service identity + mTLS + DB role, 5 open items routed to control_plane / dashboard / bundle_registry / security owners.
- **New schedule**：v0.1 critical path 17d → 20d (P0 4d + **NEW P0.5 enrichment 5d** + P1 4d + P3 4d + P3.5 3d). Within §11.5 A2 +5d envelope.
- **P0.5 (NEW workstream)**：sidecar threads SpendGuardIds.run_id into CloudEvent.run_id + adds agent_id / model_family / prompt_hash to payload_json on every emission site. Unblocks 3 of 4 rules. Proto already carries the fields (just unwired). ~5 days sidecar+adapter work.
- **P1 readiness verdict**: ✅ YES, P1 (skinny rule + CLI) **can start immediately**. Step 2/3/4 outputs are unchanged by the audit's scope cut. P0.5 runs in parallel; P1.5 (the other 3 rules at run-scope) lands after P0.5 + classifier ship.

### F1 · Backport rustls CryptoProvider fix to 9 Rust services ✅
- **Status**: ✅ 完成 — branch `fix/rustls-crypto-provider-backport` (commit `b3b1abf`)
- **Result**: 真 Rust stack 完全 boot；real gpt-4o-mini 呼叫 OK：`output='Hello there, friend!'`
- **Build perf**: cargo cache 把 rebuild 從 30 min 縮到 3 min
- **Modified**: 9 services × (main.rs + Cargo.toml) = 18 files, 63 insertions
- **Notes**: auth + leases lib-only，無需動。9 patched: canonical_ingest / control_plane / dashboard / doctor / endpoint_catalog / ledger / retention_sweeper / sidecar / usage_poller
- **次要 follow-up**: F2 — verify-step7 SQL 寫死 Mock LLM token 數，real OpenAI 變動 → 見下方

### F3 · rustls fix follow-ups (codex challenge findings) 🟡
- **Status**: 🟡 follow-up — 不阻擋 production runtime
- **Source**: codex challenge on F1 commit `b3b1abf` flagged 3 items; #1 immediately patched (publish.rs binary). 2 items remain:
- **F3a · usage_poller tests need provider setup ✅**: `services/usage_poller/src/lib.rs` 加了 `static CRYPTO_INIT: Once` + `ensure_crypto_provider()` helper 在 `mod tests`，並從 `make_obs()` 呼叫保護常用測試路徑。直接構造 `OpenAiClient::new()` / `AnthropicClient::new()` 的 tests 仍應自行加 `ensure_crypto_provider();` 呼叫（doc-comment 提示）。Branch: `fix/rustls-followups-f2-f3a`
- **F3b · rustls pin 寬鬆 `"0.23"`**：理論上 0.23.x 將來可能再 break。Round-2 服務也是 `"0.23"`，先保持一致；如果未來真因小版本 bump 出事再考慮 commit Cargo.lock 或改成 `"=0.23.40"`
- **Branch**: `fix/usage-poller-tests-rustls-init`（單獨）或合併到 F1 merge 後的 main hotfix

### F2 · `make demo-verify-step7` brittleness with real OpenAI ✅
- **Status**: ✅ 完成 — branch `fix/rustls-followups-f2-f3a`
- **Fix**: `deploy/demo/Makefile` 加 `else ifneq (,$(findstring agent_real,$(DEMO_MODE)))` guard，skip verify-step7 with informative message。涵蓋 agent_real / agent_real_anthropic / agent_real_langgraph / agent_real_openai_agents / agent_real_agt
- **長期 follow-up**：寫 `verify-step7-real` 用 range assertion（committed > 0, available 在合理範圍），但目前 skip 已足以 unblock V1 Phase 2-4

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

### V1 · Real-stack framework + cost-control end-to-end verification ✅ (with caveats)
- **Status**: ✅ 完成 — `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md`
- **Frameworks verified**:
  - ✅ LangChain (ChatOpenAI) — real gpt-4o-mini → `output='Hello, how are you?'`
  - ✅ OpenAI Agents SDK (Runner.run) — real gpt-4o-mini → `output='Greetings to you!'` + reserve/commit recorded
- **Decision paths**:
  - ✅ CONTINUE (兩個 framework 都驗)
  - ✅ STOP (deny mode)
  - 🟡 REQUIRE_APPROVAL (dispatch OK，seed bundle 缺 rule)
  - ❌ DEGRADE (未 wired)
- **Cost-control lifecycle**:
  - ✅ 事前預測 (pre-call reservation) — Stripe ledger 真實 record
  - ✅ 事中把控 (in-flight 單一 boundary) — STOP 與 CONTINUE 都證明
  - ✅ 事中把控 (multi-step agent loop) — `agent_real_openai_agents_multistep` 驗了 tool-equipped agent 2-turn loop，ledger doubled (2 reserve + 2 commit + 4 audit decisions + 2 outcomes)，證每步獨立過 sidecar — mid-loop cap 真
  - ❌ 事後建議優化 — 產品沒 suggestion engine，只有原始 audit chain
- **Phase 4 codex review**：🟡 留 separate session
- **Follow-up**：(a) seed bundle 加 REQUIRE_APPROVAL + DEGRADE rule；(b) 新增 multi-step agent demo (帶 tool / ReAct loop) 證明 mid-loop cap；(c) 補 LangGraph / Pydantic-AI / AGT / Anthropic 真 stack 驗；(d) codex challenge 本文件；(e) 評估 post-event suggestion engine 是否值得做
- **Unblocked**：P2-4 LangChain / OpenAI Agents SDK upstream PR 可基於 CONTINUE + STOP 證據誠實展開（不要 over-claim multi-step in-flight 或 post-event suggestions）
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
