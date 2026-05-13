# Session Prompt — P2-4 LangChain Upstream Docs PR

> Self-contained prompt for a fresh Claude Code session to execute P2-4
> from `docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md`.
> Goal: open a docs PR against `langchain-ai/langchain` adding an Agentic
> SpendGuard integration page — clears HN launch precondition #2 and
> borrows LangChain's 100k+ stars credibility.

---

## Prompt to paste into a fresh session

```
任務上下文
=========
你正在 agentic-spendguard 專案執行 Viral Playbook 的後續工作。前面 session 已完成所有 P0/P1 工作 + 真 benchmark 數據，剩下 Tier 3 distribution work。本次任務：**P2-4 — 對 langchain-ai/langchain upstream 開 docs PR，把 Agentic SpendGuard 加進 cost-management 章節**。

直接清掉 HN launch precondition #2（「至少 1 個 framework upstream PR merged」），順便借 LangChain 100k+ stars 的品牌信用。

工作目錄：/Users/michael.chen/products/agentic-spendguard
GitHub：https://github.com/m24927605/agentic-spendguard
新 fork target：langchain-ai/langchain

關鍵戰略決定（不要再質疑）
========================
1. **Brand**：永遠用「Agentic SpendGuard」全名
2. **真差異化**：enterprise infra（KMS-signed audit chain、Stripe-style auth/capture ledger、operator approval workflow、multi-tenant、L0–L3）— **不是** "pre-call cap" wedge
3. **真競品**：AgentGuard / AgentBudget — 我們有 benchmark：SpendGuard −10% / AgentBudget +8% / AgentGuard +1700%
4. **不誇大**：不寫「the only tool」「prevents all runaway costs」這類絕對句（codex must-fix #2）
5. **誠實 disclaimer**：LangSmith / Helicone / Portkey 在 observability/gateway 上更強 — 在他們類別不要硬比

關鍵檔案（讀這些建立 context）
=============================
- `docs/SPENDGUARD_VIRAL_PLAYBOOK.md` — 戰略 plan
- `docs/SPENDGUARD_VIRAL_PLAYBOOK.review.md` — codex 對抗審查
- `docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md` — 開放工作追蹤（這個任務是 P2-4）
- `benchmarks/runaway-loop/RESULTS.md` — 真 benchmark 數據（PR 要引用）
- `README.md` — 看 `## How this compares to other LLM cost tools` section 學一致語氣
- `docs/site/docs/integrations/langchain.md` — 我方 SpendGuard × LangChain 整合文件（PR 要 link 回來）

啟動程序
========
1. cd /Users/michael.chen/products/agentic-spendguard
2. git pull origin main
3. cat docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md | grep -A 5 "P2-4"
4. 報告當前狀態確認

實作流程（嚴格按順序）
====================

### Phase 1：偵察（必做，2-4 小時）

**1.1 找對 langchain repo 對應位置**
- WebFetch https://github.com/langchain-ai/langchain — 看 repo 結構
- 重點找：`libs/langchain/docs/` 或 `docs/docs/` 下 `how_to/`, `integrations/providers/`, `integrations/tools/`, `concepts/` 類別
- LangChain 文件 URL 慣例：`https://python.langchain.com/docs/...`
- 找現有「cost / token / observability / monitoring」相關 docs
- 例如：搜尋 LangSmith cost tracking page、Helicone integration page、tracing & callbacks page

**1.2 看 contribution 規範**
- 讀 langchain `CONTRIBUTING.md`、`docs/CONTRIBUTING.md`（如果有）
- 看他們對 third-party integration 的要求：notebook (.ipynb)? 還是 .md? 要 tests? 有沒有 integration template?
- 看最近 5 個 merged third-party docs PRs 的格式

**1.3 看現有競品有沒有上 LangChain docs**
- AgentGuard / AgentBudget / Helicone / Portkey / LangSmith 在 LangChain docs 各自怎麼出現的？
- 如果有 Helicone integration page，照抄結構
- 如果他們完全沒有，我們是先佔位的優勢

**Phase 1 報告給我看**：建議的 PR target 路徑、預期檔案結構（.md or .ipynb）、和 1-2 個現有 page 作參考

---

### Phase 2：草稿（等 Phase 1 確認後）

**2.1 fork + clone langchain repo**
- 用 `gh repo fork langchain-ai/langchain --clone --remote` 到 `/Users/michael.chen/code/langchain` 或類似位置
- 開新 branch：`integration/agentic-spendguard`

**2.2 寫文件**
- 標題：類似「Agentic SpendGuard」或「Cost control with Agentic SpendGuard」
- 必含內容：
  - 一段「what problem this solves for LangChain users」（runaway loops, token spend control）
  - LangChain integration 程式碼範例（從我方 `sdk/python/src/spendguard/integrations/langchain.py` 抄）
  - benchmark headline（**真實數字**，引用 `benchmarks/runaway-loop/RESULTS.md`）
  - 連結回 https://agenticspendguard.dev 與我方 GitHub
  - **誠實段落**：在哪裡 LangSmith/Helicone 比較適合（observability、replay、prompt management）
- 不要寫的：
  - 「the only tool」絕對句
  - 假數字（必須對應到我方 benchmark）
  - 把 Helicone/Portkey 描述成不做事

**2.3 在我方 repo 同步加追蹤**
- 把這次 PR URL 寫到 `docs/launches/upstream-prs.md`（新檔）追蹤
- 更新 todo P2-4 狀態為 🔄

---

### Phase 3：codex challenge（必做）

對草稿跑 `/codex challenge` adversarial review（**用 medium reasoning**，high+web 會 deadlock 13 分鐘）：
- 要求 codex 從 LangChain maintainer 視角看：
  1. 這個 PR 會被 close 嗎？理由？
  2. 哪段話會被 maintainer 標 "promotional"？
  3. 哪個技術說明 LangChain 用戶會說「這沒解決我的問題」？
  4. 對比 LangSmith/Helicone 的描述夠誠實嗎？
- 採納 must-fix，修改草稿
- 把 codex review 存 `docs/launches/p2-4-langchain-pr.review.md`

---

### Phase 4：開 PR

**4.1 commit 到 fork branch**
- Commit message：`docs: add Agentic SpendGuard integration page`
- 不要在 commit 裡塞 marketing — 走 LangChain 的低調風格

**4.2 開 PR**
- Title：「docs: add Agentic SpendGuard integration」
- Body 結構：
  - 1 段：what this PR adds
  - 1 段：why（LangChain users 痛點）
  - 1 段：reference benchmark + link 回我方 docs
  - Checkbox section（按 LangChain PR template）
- **不要** @-mention maintainer
- **不要** 寫 emoji 標題

**4.3 把 PR URL 在我方 repo 記錄**
- `docs/launches/upstream-prs.md` 加上 PR URL + status
- 更新 P2-4 狀態 🔄 → ⏳（等審查）

---

完成標準
========
- [ ] LangChain fork PR opened，URL 記錄在我方 repo
- [ ] codex challenge 跑過，回饋已處理
- [ ] 我方 repo todo 文件 P2-4 更新到 ⏳
- [ ] HN draft（`docs/launches/hn-show-hn-draft.md`）的 precondition 3 留 PR 連結（仍需等 merged 才打勾）
- [ ] 一句話總結：PR 強度怎樣，可能會被 close 還是 merged，後續行動

不要做
======
- 不要在 LangChain repo 帶 marketing 語氣（會被當場 close）
- 不要塞超過 1 個 SpendGuard 圖片到 LangChain docs
- 不要碰 LangChain 既有檔案（除非加新檔案的 nav config 必須）
- 不要 force push fork 任何分支
- 不要在 LangChain PR 留我方推銷文字（PR 留純技術描述）

開始
====
請執行 Phase 1 偵察並報告結果，等我說 "go" 再進 Phase 2。
```

---

## Notes for the human running this prompt

- This prompt assumes the previous P0/P1 work landed on `main` (HEAD around `3fc6ae3` or later).
- Phase 1 is investigation-only; check the agent's findings before letting it proceed to Phase 2.
- Phase 2 will fork `langchain-ai/langchain` to your account; if you don't want that fork in your namespace, redirect to a different account before running.
- The codex challenge step (Phase 3) is non-negotiable. LangChain PRs from unknown contributors get rejected fast for promotional language; codex catches this.
- After PR opens, expect 1–4 weeks for maintainer review. Track in `docs/launches/upstream-prs.md`.

## Related artifacts

- Strategic plan: `../SPENDGUARD_VIRAL_PLAYBOOK.md`
- Adversarial review: `../SPENDGUARD_VIRAL_PLAYBOOK.review.md`
- Open work: `../SPENDGUARD_VIRAL_PLAYBOOK.todo.md`
- HN launch draft (gated by this PR merging): `./hn-show-hn-draft.md`
- Benchmark data this PR cites: `../../benchmarks/runaway-loop/RESULTS.md`
