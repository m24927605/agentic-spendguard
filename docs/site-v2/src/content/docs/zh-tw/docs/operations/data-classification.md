---
title: "資料分類 (Phase 5 S19)"
---

Agentic SpendGuard 所記錄的每個事件欄位,連同其資料分類 (data
class) 的完整清單。維運在設定 `tenant_data_policy.export_redaction_field_paths`
以及執行合規審查時,以本表為準。

## 分類 (Classes)

| Class            | 說明                                             | 匯出時的預設遮蔽 (redaction)  | 預設保留期            |
|------------------|--------------------------------------------------|------------------------------|-----------------------|
| `metadata`       | 身分、結構性欄位,絕不含使用者內容               | NEVER redacted               | 完整稽核視窗          |
| `pricing`        | 定價凍結 tuple、model id、token 數               | NEVER redacted(計費證據)  | 完整稽核視窗          |
| `decision`       | matched_rule_ids、reason_codes、contract version | NEVER redacted               | 完整稽核視窗          |
| `prompt`         | 使用者 prompt 或 LLM 輸入/輸出文字               | 匯出時預設遮蔽                | tenant.prompt_retention_days |
| `provider_raw`   | provider API 回應原文逐字                        | 匯出時預設遮蔽                | tenant.provider_raw_retention_days |
| `pii`            | payload 內的使用者身分(email、姓名)            | 匯出時預設遮蔽                | tenant.prompt_retention_days |
| `provider_secret`| Webhook 簽章金鑰、API token(僅出現在錯誤 log)| 絕不寫入事件                  | n/a                   |

## 各表逐欄清單

### `audit_outbox`(以及 `canonical_events`)

| Field path                              | Class      | 備註                                                    |
|-----------------------------------------|------------|---------------------------------------------------------|
| `audit_outbox_id`                       | metadata   | UUID v7                                                 |
| `tenant_id`                             | metadata   |                                                         |
| `decision_id`                           | metadata   |                                                         |
| `audit_decision_event_id`               | metadata   |                                                         |
| `event_type`                            | metadata   |                                                         |
| `cloudevent_payload->>'specversion'`    | metadata   |                                                         |
| `cloudevent_payload->>'type'`           | metadata   |                                                         |
| `cloudevent_payload->>'source'`         | metadata   |                                                         |
| `cloudevent_payload->>'id'`             | metadata   |                                                         |
| `cloudevent_payload->>'time'`           | metadata   |                                                         |
| `cloudevent_payload->'data'`            | **prompt** | decision/outcome 內容原文;可能含使用者 prompt          |
| `cloudevent_payload->'data'->>'snapshot_hash'` | metadata | hex hash,非內容                                   |
| `cloudevent_payload->>'producer_id'`    | metadata   |                                                         |
| `cloudevent_payload->>'producer_sequence'` | metadata |                                                       |
| `cloudevent_payload_signature`          | metadata   | Ed25519 簽章 bytes                                      |
| `signing_key_id`                        | metadata   |                                                         |
| `producer_sequence`                     | metadata   |                                                         |
| `idempotency_key`                       | metadata   | 業務意圖的 hash;非內容                                  |
| `recorded_at`                           | metadata   |                                                         |

S19 redaction sweeper 在 prompt_retention_days 到期後,透過
SECURITY DEFINER 的 `redact_audit_outbox_data` SP(migration
0064)就地遮蔽:它把 `cloudevent_payload->'data'` 設成一個
marker JSONB(`{"_redacted": true, "redacted_at": "..."}`),並把
Postgres 正規化後的 `data` JSONB 的 SHA-256 digest(best-effort)
寫進另一個 `cloudevent_payload->'_data_sha256_hex'` 欄位,供維運
做 forensics。這個 digest 不是原始簽章 bytes,絕不可拿來當作
audit-chain 連續性的 anchor:`cloudevent_payload` 在 at rest 是以
JSONB 儲存(migration 0009),所以 Postgres 在 INSERT time 正規化了
key 順序與空白,原始的 canonical 序列化在這裡已無法還原。audit
chain 的完整性是靠另一個 `cloudevent_payload_signature`(Ed25519)
錨定的,而 redaction 依設計就會使其失效 — verifier 是靠 marker
偵測已遮蔽的列,而非從 digest 重新推導 canonical bytes。

### `provider_usage_records`

| Field path           | Class           | 備註                                               |
|----------------------|-----------------|----------------------------------------------------|
| `record_id`          | metadata        |                                                    |
| `provider`           | metadata        |                                                    |
| `tenant_id`          | metadata        |                                                    |
| `provider_event_id`  | metadata        |                                                    |
| `model_id`           | metadata        |                                                    |
| `prompt_tokens`      | pricing         | token 數,非文字                                   |
| `completion_tokens`  | pricing         |                                                    |
| `cost_micros_usd`    | pricing         |                                                    |
| `raw_payload`        | **provider_raw**| provider API 回應原文逐字 — 可能含文字            |

針對 `provider_raw_retention_days` 的 S19 redaction sweeper:
清掉 `raw_payload`(設成 `{"_redacted": true, ...}`),只保留上述
結構化欄位供計費 forensics。

### `ledger_transactions` / `ledger_entries`

所有欄位的 class 都是 `metadata` 或 `pricing` — 不會有 prompt /
provider raw 內容落到這些表裡。絕不遮蔽、絕不刪除(S19 不變式;
由 trigger 強制)。

### `approval_requests`

| Field path           | Class      | 備註                                                   |
|----------------------|------------|--------------------------------------------------------|
| `decision_context`   | mixed      | 含定價 tuple(metadata)+ matched rules(decision);可能含 `data` echo(prompt) |
| `requested_effect`   | metadata   | 投影出的 claims;無 prompt 內容                         |
| `resolution_reason`  | metadata   | 維運自行填入的文字(通常是業務理由)                   |

若 `decision_context.data` echo 了 prompt bytes,匯出遮蔽政策的
套用方式,與 `audit_outbox.cloudevent_payload->'data'` 相同。

## 維運 playbook

### 把租戶 prompt 保留期設為 0(只存 hash)

```sql
UPDATE tenant_data_policy
   SET prompt_retention_days = 0,
       updated_by = 'me@example.com'
 WHERE tenant_id = '...';
```

retention sweeper(S19-followup)在下一輪掃描時,會找出此租戶
所有 `cloudevent_payload->'data'` 欄位非空的稽核列,就地遮蔽。
此租戶之後的新事件會在 write time 就被遮蔽(application-level
強制)。

### 對租戶下 tombstone

```sql
UPDATE tenant_data_policy
   SET tombstoned = TRUE,
       tombstoned_at = clock_timestamp(),
       tombstoned_by = 'me@example.com',
       tombstoned_reason = 'customer offboarded'
 WHERE tenant_id = '...';
```

trigger 強制 tombstone 是單向的(無法回復)。已 tombstone 的租戶,
其 audit chain 仍可查詢。

### 稽核近期的遮蔽動作

```sql
SELECT sweep_kind, count(*), sum(rows_redacted)
  FROM retention_sweeper_log
 WHERE started_at > now() - interval '30 days'
 GROUP BY 1;
```

## 絕不遮蔽的部分(S19 不變式)

migration 0028 的 DB 層 trigger 會拒絕對以下各表的 DELETE:
- `audit_outbox`
- `audit_outbox_global_keys`
- `ledger_transactions`
- `ledger_entries`

retention sweeper 只會 UPDATE(就地遮蔽),絕不 DELETE。即使是
SUPERUSER,也必須明確 disable trigger 才能移除列 — 而那個動作
會留在 pg_audit log 裡。

## 缺口(S19-followup)

1. **Retention sweeper service** 尚未出貨。schema 已就位;會掃描
   audit_outbox + provider_usage_records 並套用遮蔽的背景 worker
   是下一塊。
2. **Application-level write-time 遮蔽**(當
   `prompt_retention_days = 0` 時)需要 sidecar +
   webhook_receiver 的程式碼路徑,在寫入 `data` 欄位前先查
   `tenant_data_policy`。
3. **匯出端點遮蔽**(S9)需要查
   `tenant_data_policy.export_redaction_field_paths`,在 JSONL 那
   一行送出去之前,先剝掉那些 JSONB paths。
4. **Application-level tombstone 強制** — sidecar /
   webhook_receiver / control_plane 必須檢查
   `tenant_data_policy.tombstoned`,並拒絕對該租戶的寫入。縱深防禦
   (defense in depth):既有的列仍可查詢。
