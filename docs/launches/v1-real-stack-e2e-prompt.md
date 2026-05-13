# Session Prompt — V1 Real-Stack End-to-End Verification

> Self-contained prompt for a fresh Claude Code session.
> Goal: prove Agentic SpendGuard works end-to-end with a **real**
> LangChain agent + **real** Rust sidecar stack + **real** OpenAI/Anthropic
> calls — not the benchmark shim. This is a hard gate before opening
> any framework-upstream PR (P2-4) or making any externally cited
> "Agentic SpendGuard works with X" claim.
>
> **Why this exists**: M1 benchmark used `spendguard_shim/` (a minimal
> reservation gateway, not the real Rust sidecar) against a mock LLM.
> Codex challenge already flagged "Real-sidecar runner (currently uses
> shim)" as an open follow-up. Without V1, the LangChain PR would be
> closed as premature.

---

## Prompt to paste into a fresh session

```
任務上下文
=========
你正在 agentic-spendguard 專案執行 Viral Playbook 的 hard gate 任務：**V1 — 真實環境 end-to-end verification**。

前面 session 已完成：M1 benchmark（用 shim + mock LLM 證明三方對照）/ P1 系列文件 / brand rebrand / 域名上線。剩下要對 LangChain 等 framework 開 upstream PR 之前，**必須先在真實環境跑通 SpendGuard**：真 LangChain agent → 真 Rust sidecar + Postgres ledger + canonical_ingest → 真 OpenAI / Anthropic API call。

這個任務本身就是 Tier 0 等級 — 它揭露真實的產品成熟度。如果 demo_mode=agent_real_* 跑不起來，比起任何 README 改善都重要：產品根本還沒準備好給陌生人用。

工作目錄：/Users/michael.chen/products/agentic-spendguard
GitHub：https://github.com/m24927605/agentic-spendguard

關鍵戰略決定（不要再質疑）
========================
1. **Brand**：永遠用「Agentic SpendGuard」全名
2. **真差異化**：enterprise infra（KMS-signed audit chain、Stripe-style ledger、operator approval、multi-tenant、L0–L3）— **不是** "pre-call cap" wedge
3. **不誇大**：真實 e2e 跑得出來什麼就寫什麼。跑不通的決策路徑就誠實標 "not yet verified"
4. **這個任務的成功 = 揭露真相**：跑得通就有 launch 武器；跑不通就找 bug 修；無論結果都是進步

關鍵檔案（讀這些建立 context）
=============================
- `README.md` line 65-79 — 列出已存在的 DEMO_MODE，其中：
  - `DEMO_MODE=agent_real` — 真 OpenAI call
  - `DEMO_MODE=agent_real_anthropic` — 真 Anthropic call
  - `DEMO_MODE=agent_real_langgraph` — LangGraph integration（**沒有 langchain pure mode！**）
  - `DEMO_MODE=agent_real_openai_agents` — OpenAI Agents SDK
  - `DEMO_MODE=agent_real_agt` — Microsoft AGT
- `Makefile` — `make demo-up` 的實作位置
- `deploy/demo/compose.yaml` — full docker stack
- `sdk/python/src/spendguard/integrations/langchain.py` — LangChain integration 模組
- `docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md` — open work tracker
- `benchmarks/runaway-loop/RESULTS.md` — shim/mock benchmark（要區別於 V1 的真 e2e）

啟動程序
========
1. cd /Users/michael.chen/products/agentic-spendguard
2. git pull origin main
3. cat docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md | head -30
4. 確認 `OPENAI_API_KEY` env var 存在（用：`echo ${OPENAI_API_KEY:0:10}...`）— 如果空的，請我設定，不要繼續
5. 報告當前狀態確認

實作流程
========

### Phase 1：Smoke test 既有 demo modes（1-2 小時）

逐個跑下列指令，記錄成功 / 失敗：

```bash
# 1. 純 OpenAI 模式（不經 LangChain）— 最 baseline
make demo-up DEMO_MODE=agent_real

# 2. LangGraph 模式（最接近 LangChain 的存在 mode）
make demo-up DEMO_MODE=agent_real_langgraph
```

每一個跑完後：
- ✅ 如果 docker-compose stack 起來、agent 跑、ledger 寫入、agent 收到合理回應 → 記錄成功
- ❌ 任何環節 fail → 記錄錯誤訊息、stack trace、相關 service log
- 用 `docker compose logs <service>` 抓相關 log

---

### Phase 2：建立 LangChain pure mode（如不存在）

如果 Phase 1 確認 `agent_real_langchain` mode 不存在：

1. 在 `deploy/demo/runtime/` 或對應位置寫一個 `agent_real_langchain.py`，呼叫 LangChain `ChatOpenAI` 並用 `spendguard.integrations.langchain` 包覆
2. 在 Makefile 加 `DEMO_MODE=agent_real_langchain` case
3. 跑：`make demo-up DEMO_MODE=agent_real_langchain`

如果存在但壞了 → debug 修好。

---

### Phase 3：驗證 4 種 decision path（real LangChain stack）

對每個 path 設計一個觸發場景，跑一次，截 evidence：

| Path | 觸發方式 | 預期結果 |
|---|---|---|
| **CONTINUE** | budget 充足，正常呼叫 | LangChain agent 收到 OpenAI 回應 |
| **STOP** | budget 設 $0.01，呼叫 gpt-4o | sidecar 拒絕，LangChain raises `DecisionStopped` |
| **REQUIRE_APPROVAL** | 設 contract 要求 approval；操作員 approve | agent 暫停 → resume 後完成呼叫 |
| **DEGRADE** | 設 contract: gpt-4o → gpt-4o-mini mutation | agent 收到 mini model 的回應 |

對每個 case 記錄：
- 終端 stdout / stderr
- ledger DB 內 reservation 紀錄（用 `docker compose exec ledger psql ...`）
- canonical_ingest 收到的 audit row
- agent 觀察到的行為（response, exception, retry）

---

### Phase 4：寫 `REAL_LANGCHAIN_E2E.md`

路徑：`benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md`（新檔，與 benchmarks/runaway-loop/ 平行）

文件結構：
1. **Environment** — versions of LangChain / Python / Rust / docker / OpenAI SDK
2. **Setup steps** — 從 git clone 到第一個 CONTINUE 跑出來的完整指令序列
3. **Decision path verification table** — 4 個 path 的 evidence
4. **Known bugs / limitations** — 任何在過程中發現的東西
5. **Performance observation** — 觀察到的 sidecar overhead（如果可量），即使是粗略數字也好
6. **Reproducibility** — 別人怎麼從零跑一遍

附帶 evidence files：
- `benchmarks/real-stack-e2e/evidence/continue.log`
- `benchmarks/real-stack-e2e/evidence/stop.log`
- `benchmarks/real-stack-e2e/evidence/approval.log`
- `benchmarks/real-stack-e2e/evidence/degrade.log`

---

### Phase 5：codex challenge

對 `REAL_LANGCHAIN_E2E.md` 跑 `/codex challenge`（**medium reasoning，不要 high+web**）：
- 從「LangChain maintainer 看完這份 e2e 文件後會問什麼」視角
- 哪段是 marketing 包裝技術 demo？
- 哪個失敗 case 沒測到？
- 哪段需要修才能變成 PR 證據？

把 codex 回饋寫到 `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.review.md`，必修項回頭改 evidence。

---

### Phase 6：更新 TODO + commit

- 在 `docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md` 開新 Tier 0 條目 V1，狀態 ✅ + commit hash
- 把 V1 列為 P2-4 的 dependency
- commit：`feat(verify): real-stack LangChain end-to-end verification [V1]`

---

完成標準
========
- [ ] `agent_real_langchain` demo mode 存在且跑得起來
- [ ] 4 個 decision path（CONTINUE / STOP / REQUIRE_APPROVAL / DEGRADE）都有 evidence log
- [ ] `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md` 落筆
- [ ] codex review 跑過、必修項處理完
- [ ] todo 文件 V1 標 ✅
- [ ] 一句話總結：產品 e2e 是真的能跑，還是有 X 個 bug 要先修才能對外宣稱「LangChain integration works」

可能的失敗結果（也是合法產出）
============================
這個任務允許 STATUS=BLOCKED：

- 如果 Phase 1 既有 demo mode 跑不起來 → 寫 bug report + 列出 fix list 即可結束 V1 第一輪
- 如果 LangChain integration 模組根本沒對應到 LangChain 0.3+ 新 API → 寫 mismatch report
- 跑不通本身就是價值 — 因為它告訴我們「現在還不能對外說 LangChain integration is production-ready」，避免 PR 被 close

不要做
======
- 不要為了讓 e2e「看起來成功」掩蓋 bug
- 不要硬編 mock response 讓決策看起來生效
- 不要跳過任何一個 decision path 不測
- 不要在 evidence log 裡編造數字
- 跑不通就跑不通，誠實標 ❌

開始
====
請執行啟動程序與 Phase 1 smoke test，先告訴我兩個 demo mode 的真實狀況。
```

---

## Notes for the human running this prompt

- This V1 task is itself Tier 0 priority. Run it before P2-4 LangChain PR.
- Requires `OPENAI_API_KEY` (and optionally `ANTHROPIC_API_KEY`) in the env.
- Real API costs: ~$1-3 of OpenAI tokens for full Phase 3 verification.
- Phase 1 + 2 may surface bugs; that is expected and desired output.
- After V1 completes successfully, the existing `p2-4-langchain-pr-prompt.md`
  becomes runnable. Without V1, P2-4 should not start.

## Related artifacts

- Strategic plan: `../SPENDGUARD_VIRAL_PLAYBOOK.md`
- Adversarial review: `../SPENDGUARD_VIRAL_PLAYBOOK.review.md`
- Open work tracker: `../SPENDGUARD_VIRAL_PLAYBOOK.todo.md`
- LangChain PR prompt (blocked on V1): `./p2-4-langchain-pr-prompt.md`
- Benchmark with shim (NOT real e2e): `../../benchmarks/runaway-loop/RESULTS.md`
