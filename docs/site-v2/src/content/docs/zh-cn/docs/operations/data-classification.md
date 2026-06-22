---
title: "数据分类 (Phase 5 S19)"
---

Agentic SpendGuard 记录的每一个事件字段及其数据分类的总目录。运维在配置
`tenant_data_policy.export_redaction_field_paths`、以及做合规审查时,以本文为准。

## 分类

| Class            | 说明                                              | 导出时默认脱敏                   | 默认保留期             |
|------------------|--------------------------------------------------|------------------------------|-----------------------|
| `metadata`       | 身份、结构信息,绝不含用户内容                     | NEVER redacted               | full audit window     |
| `pricing`        | 价格冻结 tuple、model id、token 计数              | NEVER redacted (billing evidence) | full audit window |
| `decision`       | matched_rule_ids、reason_codes、合约版本          | NEVER redacted               | full audit window     |
| `prompt`         | 用户 prompt 或 LLM 输入/输出文本                  | redacted by default at export | tenant.prompt_retention_days |
| `provider_raw`   | provider API 原样响应                            | redacted by default at export | tenant.provider_raw_retention_days |
| `pii`            | payload 中携带的用户身份信息(邮箱、姓名)        | redacted by default at export | tenant.prompt_retention_days |
| `provider_secret`| webhook 签名密钥、API token(仅在错误日志中出现) | NEVER stored in events    | n/a                   |

## 各表逐字段目录

### `audit_outbox` (以及 `canonical_events`)

| Field path                              | Class      | 备注                                                    |
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
| `cloudevent_payload->'data'`            | **prompt** | decision/outcome 原始内容;可能含用户 prompt            |
| `cloudevent_payload->'data'->>'snapshot_hash'` | metadata | hex hash,非内容本身                               |
| `cloudevent_payload->>'producer_id'`    | metadata   |                                                         |
| `cloudevent_payload->>'producer_sequence'` | metadata |                                                       |
| `cloudevent_payload_signature`          | metadata   | Ed25519 签名字节                                        |
| `signing_key_id`                        | metadata   |                                                         |
| `producer_sequence`                     | metadata   |                                                         |
| `idempotency_key`                       | metadata   | 业务意图的 hash;非内容本身                             |
| `recorded_at`                           | metadata   |                                                         |

S19 脱敏 sweeper 在 prompt_retention_days 到期后,通过 SECURITY DEFINER 的
`redact_audit_outbox_data` SP(migration 0064)就地脱敏:把
`cloudevent_payload->'data'` 置为标记 JSONB
(`{"_redacted": true, "redacted_at": "..."}`),并把 Postgres 规范化后的 `data`
JSONB 的 SHA-256 摘要(best-effort)写到单独的
`cloudevent_payload->'_data_sha256_hex'` 字段,供运维取证。该摘要不是原始签名字节,
绝不可当作审计链的连续性锚点:`cloudevent_payload` 在落盘时是 JSONB(migration 0009),
因此 INSERT 时 Postgres 规范化了 key 顺序和空白,原始的 canonical 序列化在这里无法还原。
审计链的完整性由单独的 `cloudevent_payload_signature`(Ed25519)锚定,而脱敏在设计上必然使其失效
—— verifier 通过标记识别已脱敏的行,而不是从摘要反推 canonical 字节。

### `provider_usage_records`

| Field path           | Class           | 备注                                               |
|----------------------|-----------------|----------------------------------------------------|
| `record_id`          | metadata        |                                                    |
| `provider`           | metadata        |                                                    |
| `tenant_id`          | metadata        |                                                    |
| `provider_event_id`  | metadata        |                                                    |
| `model_id`           | metadata        |                                                    |
| `prompt_tokens`      | pricing         | token 计数,非文本                                 |
| `completion_tokens`  | pricing         |                                                    |
| `cost_micros_usd`    | pricing         |                                                    |
| `raw_payload`        | **provider_raw**| provider API 原样响应 —— 可能含文本                |

`provider_raw_retention_days` 的 S19 脱敏 sweeper:清空 `raw_payload`
(置为 `{"_redacted": true, ...}`),并仅保留上述结构化字段用于计费取证。

### `ledger_transactions` / `ledger_entries`

所有字段都属 `metadata` 或 `pricing` 类 —— 这两张表里不会落入 prompt /
provider raw 内容。绝不脱敏、绝不删除(S19 不变式;trigger 强制保证)。

### `approval_requests`

| Field path           | Class      | 备注                                                   |
|----------------------|------------|--------------------------------------------------------|
| `decision_context`   | mixed      | 含价格 tuple(metadata)+ matched rules(decision);可能含 `data` 回显(prompt) |
| `requested_effect`   | metadata   | 投影出的 claims;无 prompt 内容                         |
| `resolution_reason`  | metadata   | 运维填写的文本(通常是业务原因)                       |

若 `decision_context.data` 回显了 prompt 字节,导出脱敏策略的处理方式与
`audit_outbox.cloudevent_payload->'data'` 完全一致。

## 运维 playbook

### 把某租户的 prompt 保留期设为 0(只存 hash)

```sql
UPDATE tenant_data_policy
   SET prompt_retention_days = 0,
       updated_by = 'me@example.com'
 WHERE tenant_id = '...';
```

保留期 sweeper(S19-followup)在下一轮扫描时,会找到该租户所有
`cloudevent_payload->'data'` 字段非空的审计行并就地脱敏。该租户的新事件在写入时
即被脱敏(应用层强制)。

### 给某租户打 tombstone

```sql
UPDATE tenant_data_policy
   SET tombstoned = TRUE,
       tombstoned_at = clock_timestamp(),
       tombstoned_by = 'me@example.com',
       tombstoned_reason = 'customer offboarded'
 WHERE tenant_id = '...';
```

trigger 强制 tombstone 单向(无法回退)。被 tombstone 的租户其审计链仍可查询。

### 审查最近的脱敏记录

```sql
SELECT sweep_kind, count(*), sum(rows_redacted)
  FROM retention_sweeper_log
 WHERE started_at > now() - interval '30 days'
 GROUP BY 1;
```

## 绝不脱敏的内容(S19 不变式)

migration 0028 中的 DB 层 trigger 会拒绝对以下表的 DELETE:
- `audit_outbox`
- `audit_outbox_global_keys`
- `ledger_transactions`
- `ledger_entries`

保留期 sweeper 只做 UPDATE(就地脱敏),从不 DELETE。即便是 SUPERUSER,也必须显式
禁用 trigger 才能删行 —— 而这个动作会在 pg_audit 日志里留下痕迹。

## 缺口(S19-followup)

1. **保留期 sweeper service** 尚未交付。schema 已就位;扫描
   audit_outbox + provider_usage_records 并执行脱敏的后台 worker 是下一个 chunk。
2. **应用层写入时脱敏**(当 `prompt_retention_days = 0` 时)需要 sidecar +
   webhook_receiver 代码路径在写入 `data` 字段前先查
   `tenant_data_policy`。
3. **导出端点脱敏**(S9)需要查
   `tenant_data_policy.export_redaction_field_paths`,在 JSONL 行发出前剥掉那些
   JSONB 路径。
4. **应用层 tombstone 强制** —— sidecar / webhook_receiver / control_plane 必须检查
   `tenant_data_policy.tombstoned`,拒绝该租户的写入。纵深防御:已有的行仍可查询。
