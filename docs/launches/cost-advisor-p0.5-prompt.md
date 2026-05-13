# Session Prompt — Cost Advisor P0.5 — Sidecar Audit Enrichment

> Self-contained prompt for a fresh Claude Code session.
> **Goal**: thread `run_id` / `agent_id` / `model_family` / `prompt_hash` through sidecar audit emission so cost_advisor rules have the dedup keys they need.
> **Estimated**: 5 days.
> **Tracker**: GitHub issue #48.
> **Status going in**: CA-P0 GREEN-merged on main (commit `42cb787`). P0 audit identified this workstream as required before any rule can fire.

---

## Prompt to paste into a fresh session

```
任務上下文
=========
你正在 agentic-spendguard 專案實作 Cost Advisor 的 P0.5（sidecar audit-
payload enrichment）。CA-P0 audit (docs/specs/cost-advisor-p0-audit-
report.md §3 + §8) 證實 5 of 6 規則需要的欄位在 canonical_events
.payload_json 完全沒有：
- prompt_hash (never emitted)
- agent_id (proto SpendGuardIds.step_id 存在但沒接到 CloudEvent)
- run_id (column 存在但 sidecar 每個 emission site 都寫 String::new())
- model_family (proto 存在但 adapter_uds.rs:949 寫成空字串)
- tool_name / tool_args_hash (deferred to v0.2 SDK)

P0.5 接上能拿到的 4 個欄位。tool_* 留給 v0.2。

工作目錄：/Users/michael.chen/products/agentic-spendguard
GitHub：https://github.com/m24927605/agentic-spendguard
Issue：#48

關鍵戰略決定（不要再質疑）
=========================
1. 走 additive payload — 加 payload_json 欄位、不動 envelope schema
   （除了 CloudEvent.run_id 它本來就是 canonical_events 的 column 沒被填）
2. prompt_hash normalization 走範式 (a)：SHA-256 of raw prompt text bytes
   經過 UTF-8 normalization (NFC) + 去前後 whitespace（具體規則 Step 1 鎖）
3. 只動 sidecar + Pydantic-AI adapter 一個 SDK。LangChain / OpenAI Agents
   留給 v0.2
4. 不要動 canonical_ingest classifier（那是 P1 #51 的事）

關鍵檔案（必讀）
==============
- services/sidecar/src/decision/transaction.rs（5 個 audit emission site）
- services/sidecar/src/server/adapter_uds.rs（resume-after-approval 第 6 個 site）
- proto/spendguard/sidecar_adapter/v1/adapter.proto（DecisionRequest.inputs.runtime_metadata）
- proto/spendguard/common/v1/common.proto（SpendGuardIds, UnitRef）
- adapters/pydantic-ai/spendguard_pydantic_ai/（Pydantic-AI adapter SDK）
- docs/specs/cost-advisor-p0-audit-report.md §3（payload shape ground truth）

啟動程序
========
1. cd /Users/michael.chen/products/agentic-spendguard
2. git pull origin main
3. git switch -c feat/cost-advisor-p0.5
4. cat docs/specs/cost-advisor-p0-audit-report.md §3 §8（複習 emission sites）

P0.5 實作流程
=============

### Step 1：prompt_hash normalization 規則鎖定（半天）

決策：
- 是否 NFC unicode normalize？YES（agent runtime 可能不同 OS 不同編碼）
- trim 前後 whitespace？YES（不影響語義）
- internal whitespace 怎麼處理？保留（"hello world" ≠ "hello  world"
  因為 LLM 可能對 token boundary 敏感）
- 是 SHA-256(prompt_text) 還是 SHA-256(template + bindings_json)？
  → P0.5 走 (a) raw prompt text。Template-aware 留給 v0.2 + LangChain
   support 一起做（LangChain 有 ChatPromptTemplate 比較好接 template）
- Hex output vs base64？hex（matches canonical_events fingerprint）

**產出**：services/sidecar/src/prompt_hash.rs 新 module 含 normalize +
compute（單元測試 5 個 fixture：ascii, unicode, leading whitespace,
trailing whitespace, NFC vs NFD）

### Step 2：proto + envelope wiring（1 天）

DecisionRequest.inputs.runtime_metadata: google.protobuf.Struct 已存在。
Define stable key convention（in proto comment）：
- `runtime_metadata.fields.prompt_hash`: hex SHA-256 string

Sidecar 從 runtime_metadata 抽 prompt_hash，放到 payload。

Patch 5 個 sidecar emission site:
- services/sidecar/src/decision/transaction.rs:
  - line 365-400 (run_through_reserve audit.decision)
  - line 472-490 (run_record_denied_decision audit.decision)
  - line 805-840 (commit_estimated audit.outcome)
  - line 1000-1030 (release audit.outcome)
- services/sidecar/src/server/adapter_uds.rs:
  - line 949 (UnitRef.model_family 從空字串改成從 pricing snapshot 抽)
  - line 974-993 (resume-after-approval audit.decision)

每個 site 加：
```rust
CloudEvent {
    // ... existing fields,
    run_id: ctx.run_id.clone(),  // 從 SpendGuardIds.run_id
    // ... data payload now includes:
    //   "agent_id": ctx.step_id (from SpendGuardIds.step_id)
    //   "model_family": <from UnitRef.model_family resolved at decision time>
    //   "prompt_hash": <from DecisionRequest.inputs.runtime_metadata>
}
```

DecisionContext struct 需要新增 run_id / step_id / prompt_hash fields。
從 DecisionRequest 抽出來傳下去（不要 mutate 既有 fields）。

**產出**：cargo test --workspace pass，所有 demo modes（decision /
invoice / agent / release / ttl_sweep / deny）pass

### Step 3：Pydantic-AI adapter 接 prompt_hash（半天）

adapters/pydantic-ai/spendguard_pydantic_ai/client.py 加 `compute_prompt_hash()`
helper（match sidecar 的 normalize 規則 — 共享測試 vector）。

Adapter 在 DecisionRequest.inputs.runtime_metadata 填入 prompt_hash：
```python
metadata = Struct()
metadata["prompt_hash"] = compute_prompt_hash(messages)
```

test fixture：Python 算的 hash 與 Rust 算的 hash byte-equal。

**產出**：Python + Rust 共享 5 個 test vector 都 byte-equal

### Step 4：Integration test against benchmark fixtures（1 天）

benchmarks/runaway-loop/ 已有 LangChain runaway-loop fixture。
擴成 Pydantic-AI runaway-loop 版本（或新增 benchmarks/runaway-loop-pydantic-ai/）。
跑後查 canonical_events.payload_json：
```sql
SELECT
    payload_json->>'agent_id' AS agent_id,
    payload_json->>'model_family' AS model_family,
    payload_json->>'prompt_hash' AS prompt_hash,
    run_id  -- 從 envelope column
FROM canonical_events
WHERE event_type = 'spendguard.audit.decision'
ORDER BY event_time DESC LIMIT 10;
```

預期：4 欄都 populated > 80%，不是 NULL/empty string。

**產出**：benchmark report 顯示 enrichment populate rate per field

### Step 5：Demo mode + verify（半天）

deploy/demo/ 加新 DEMO_MODE=p0_5_enrichment：
- Pydantic-AI demo with new compute_prompt_hash 接線
- verify_p0_5.sql 跑 SELECT COUNT(*) FILTER (WHERE
  payload_json->>'prompt_hash' IS NOT NULL) AS hash_populated
- assert hash_populated > 0

執行慣例
========
- **Branch**: `feat/cost-advisor-p0.5`
- **Commit prefix**: `[CA-P0.5]`
- **Codex challenge**: 完成後跑 2 輪（涉及 hot-path sidecar code; 不可逆 if
  emission shape 變 break canonical_ingest verifier）
- **更新 issue**: #48 標進度

完成標準
========
- [ ] prompt_hash module + 5 個 unit fixture pass（Step 1）
- [ ] 5 個 sidecar emission site 都 emit 4 欄（Step 2）
- [ ] Python + Rust prompt_hash byte-equal vector match（Step 3）
- [ ] benchmark canonical_events 顯示 > 80% populate rate（Step 4）
- [ ] DEMO_MODE=p0_5_enrichment PASS（Step 5）
- [ ] 2 輪 codex challenge GREEN
- [ ] issue #48 closed

不要做
======
- 不要動 tool_name / tool_args_hash（v0.2）
- 不要動 LangChain / OpenAI Agents adapter（v0.2）
- 不要 backfill 既有 canonical_events 行（immutability trigger 阻擋，
  rule 處理 NULL 為 not-fireable 是 spec §5.1.2 認可的 degraded path）
- 不要動 canonical_ingest classify.rs（P1 issue #51）
- 不要為 prompt_hash 加 PII redaction（hash 本身不是 PII；spec §11.5 Q7
  確認）

開始
====
請執行 Step 1 prompt_hash normalization 規則鎖定 + module 寫好 + 5 unit test pass，回報後進 Step 2。
```

---

## Notes for the human running this prompt

- **Pre-req**: docker-compose 跑 + DEMO_MODE=agent baseline pass
- **OpenAI key**: 第 4 步 benchmark 要真跑 LLM（agent_real_*），需要 ~/.env 有 OPENAI_API_KEY
- **Schedule risk**: 第 3 步 Python+Rust hash 對齊可能花更多時間如果 NFC normalize 行為差異（unicodedata module vs unicode-normalization crate）

## Related artifacts

- Spec: `../specs/cost-advisor-spec.md` §5.1.2 + §11.5 A2
- Audit: `../specs/cost-advisor-p0-audit-report.md` §3 §8.5 (authoritative)
- Sibling: `cost-advisor-p0.6-prompt.md` (parallel workstream)
- Downstream: `cost-advisor-p1-prompt.md` (gated on this + P0.6)

## After P0.5 completes

If P0.6 also done → start CA-P1 (issue #50). The runtime + rule SQL consumes
the enriched payload.
