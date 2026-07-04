# Global Development Guidelines

## 開發原則

- YAGNI / DRY / 先用最簡單的方案
- 小步提交、可編譯、有測試
- 修改前先了解現有 codebase 的慣例
- 不跳過 commit hooks、不停用測試
- 卡住 3 次就停下來重新評估架構

## Codex Brainstorming

開發新功能時，不要逐一問用戶問題，改用 Codex 自動探索：

1. **Round 1 — 初始呼叫：**
   ```bash
   ~/.Codex/hooks/codex-brainstorm.sh "<feature request>" --project-dir $PWD [--context-files file1,file2]
   ```
2. **Codex 審查輸出**，評估完整性、可行性、一致性、遺漏
3. **如有不足 — 後續輪次（最多 10 輪）：**
   ```bash
   ~/.Codex/hooks/codex-brainstorm.sh "<original request>" \
     --project-dir $PWD \
     --previous-result /tmp/codex-brainstorm-round-N.json \
     --followup "<specific feedback>"
   ```
4. **最終結論以繁體中文呈現：** 目的與範圍、推薦方案、架構概述、討論輪次摘要、需人工決定的問題
5. 只有真正需要人工判斷的 open_questions 才問用戶

## Codex Review Gate

階段轉換時必須執行 Codex review：

| Stage | Command |
|-------|---------|
| Spec | `CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh spec path/to/spec.md --project-dir $PWD` |
| Plan | `CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh plan path/to/plan.md --project-dir $PWD` |
| Task | `BASELINE=$(git rev-parse HEAD) && CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh task $BASELINE --project-dir $PWD` |
| Final | `CODEX_REVIEW_ITERATION=1 ~/.Codex/hooks/codex-review.sh final branch:main --project-dir $PWD` |

- 每個 stage 的 `CODEX_REVIEW_ITERATION` 從 1 開始，最多 5 次
- Task stage 始終使用開始時的 `$BASELINE`
- 未通過則修正後重試；連續失敗 3 次改用全局修正策略
- 第 5 輪仍未通過時，不得只停止並通知用戶。必須啟動 **Staff+ Expert Recovery Cycle**：
  - 組成多名 Staff 等級以上的 AI coding agent 資深專家團隊，至少涵蓋 spec/architecture、implementation、security/reliability、DX/release、review-gate owner 等視角
  - 團隊先整理 review gate 的所有 blocker，去重、分級，找出根因與互相衝突的規格
  - 團隊必須產出一個可執行的修正方案，明確列出要改哪些文件、測試、實作或驗證流程
  - 由團隊決定最佳方案後直接執行，不要把可自行判斷的問題丟回給用戶
  - Staff+ 團隊的決策取代第 5 輪未通過的 review gate 結論；完成修正與必要驗證後，將該 stage 視為 resolved/approved，記錄 recovery summary，直接進入下一 stage，不再重新從 `CODEX_REVIEW_ITERATION=1` 跑同一 stage
  - 只有在同一個 blocker 經過至少 3 個 Staff+ Expert Recovery Cycle 仍無法決定或執行，或需要外部權限/產品決策時，才停止並用繁體中文回報原因、已嘗試方案、剩餘人工決策

## gstack

Use the `/browse` skill from gstack for **all web browsing**. Never use
`mcp__claude-in-chrome__*` tools.

### Available Skills

`/office-hours`, `/plan-ceo-review`, `/plan-eng-review`,
`/plan-design-review`, `/design-consultation`, `/design-shotgun`,
`/design-html`, `/review`, `/ship`, `/land-and-deploy`, `/canary`,
`/benchmark`, `/browse`, `/connect-chrome`, `/qa`, `/qa-only`,
`/design-review`, `/setup-browser-cookies`, `/setup-deploy`, `/retro`,
`/investigate`, `/document-release`, `/codex`, `/cso`, `/autoplan`,
`/careful`, `/freeze`, `/guard`, `/unfreeze`, `/gstack-upgrade`, `/learn`
