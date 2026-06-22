---
title: "演练:strict-signature 隔离量突增"
---

季度演练。验证签名失败场景下的审计链完整性保证:当某个 producer 的签名密钥已经轮换、
但 verifier 的 trust store 没有同步更新时,canonical_ingest verifier 必须把每一条受影响
的行拒收并打入 `audit_signature_quarantine`(strict 模式),或者放行并累加 admit 计数器
(非 strict 模式)。但无论哪种模式,都绝不允许丢字节、也不允许悄悄重新编码字节。

这是下面这些单元测试的线上对应版本:
`services/canonical_ingest/src/verifier.rs::tests::*`,以及
`services/canonical_ingest/src/metrics.rs::tests` 里的 metrics 测试。

## 这个演练覆盖什么

- strict 模式:告警 A5 `SpendGuardCanonicalRejectsHigh` 触发。
- 非 strict 模式(PR #2 round 1 P2#3 修复,提交 `eec0404`):
  `unknown_key_admitted_total` 和
  `invalid_signature_admitted_total` 计数器累加,但行依然落进
  `canonical_events`,这样在滚动密钥轮换期间审计链不会断。
- S7 密钥注册表(canonical-ingest migrations 0008/0009 里的 `signing_keys` +
  `signing_key_revocations` 表)—— 隔离原因会区分 `key_expired` /
  `key_revoked` / `key_not_yet_valid` / `unknown_key` / `invalid_signature`。

## 现象(on-call 看到什么)

- 告警 A5 `SpendGuardCanonicalRejectsHigh` 正在触发。
- `audit_signature_quarantine` 行数在涨。
- `canonical_events` 增长变慢(strict 模式下甚至持平)。
- `audit_outbox.pending_forward = TRUE` 的行数在涨 —— forwarder 一直在
  重试同一批被拒的行。
- 用户侧:对 producer 无即时影响(sidecar / ledger / webhook 仍然在写行)。
  审计消费方看到隔离量增长;合规侧看到缺口。

## 第一步排查

```bash
# 1. Quarantine breakdown by reason (Phase 5 S7 + S8 schema):
psql -h $CANONICAL_PG_HOST -U spendguard -d spendguard_canonical -c "
  SELECT reason, count(*), max(quarantined_at) AS most_recent
    FROM audit_signature_quarantine
   WHERE quarantined_at > now() - interval '1 hour'
   GROUP BY reason
   ORDER BY count DESC;
"

# 2. Which signing keys are involved?
psql -h $CANONICAL_PG_HOST -U spendguard -d spendguard_canonical -c "
  SELECT signing_key_id, count(*) AS quarantined_rows
    FROM audit_signature_quarantine
   WHERE quarantined_at > now() - interval '1 hour'
   GROUP BY signing_key_id
   ORDER BY count DESC;
"

# 3. Compare signing keys claimed by producers vs trust store:
psql -h $CANONICAL_PG_HOST -U spendguard -d spendguard_canonical -c "
  SELECT key_id, valid_from, valid_until, revoked_at IS NOT NULL AS is_revoked
    FROM signing_keys
   ORDER BY valid_from DESC
   LIMIT 10;
"

# 4. Strict mode check (different remediation for strict vs non-strict):
kubectl exec <canonical-ingest-pod> -- env | grep STRICT_SIGNATURES
# true  → strict (rows rejected); false → non-strict (admitted + counted)
```

## 缓解(短期止血)

走哪条路取决于第 1 步里哪个 `reason` 占主导:

### `unknown_key` 占主导

producer 用了一个 verifier 无法识别的密钥。常见原因:密钥轮换先部署到了
producer,canonical-ingest 这边的 trust store 更新滞后。

1. **定位新密钥**(第 2 步 + producer 近期的日志)。
2. **加进 trust store**:
   ```bash
   kubectl edit secret spendguard-signing-trust-store
   # Append the new public key + valid_from window
   kubectl rollout restart deployment canonical-ingest
   ```
3. **回放被隔离的行**:PR #2 round 1 的隔离会原样保留原始字节。trust store
   更新后,从 `audit_signature_quarantine` 表手动重新 ingest 进
   `canonical_events`(S8-followup 特性;目前需要手写 SQL)。

### `invalid_signature` 占主导

这个更严重 —— 字节和声明的签名对不上。可能性:
- producer 代码回归(签了错误的 canonical 字节)
- 链路上有人在主动篡改(mTLS 配置错了?)

1. **立刻停掉受影响的 producer**,在查清根因之前保持停止状态:
   ```bash
   kubectl scale deployment <producer-name> --replicas=0
   ```
2. **把 producer 镜像和已知良好版本做 diff**,看 canonical-form 序列化有没有改动。
3. **在排除篡改之前,绝不丢、也不回放隔离行** —— 这些字节是取证证据。

### `key_expired` / `key_revoked` 占主导

S7 有效期窗口的强制检查。producer 用了一个已过 `valid_until` 或已过 `revoked_at`
的密钥在签名。

1. **把 producer 的签名材料轮换到一个当前有效的密钥**。
2. **审计这个缺口**:在 `valid_from` 到 `valid_until` 窗口内、用那把过期密钥签的行
   仍然合法(当时密钥是有效的);窗口之后才签的行,说明 producer 配置有 bug。

## 升级路径

- 突增持续 **5 分钟** → page platform oncall。
- **15 分钟** 仍无诊断结论 → page sidecar/ledger team oncall(看是哪个 producer 受影响)。
- **`invalid_signature` 出现 >0 行** → 立刻 page security team(可能存在篡改)。
- strict 模式下隔离持续 **30+ 分钟** → 考虑临时切到非 strict
  (operator 决策,需要 Helm gate ack —— 这是在你修根因期间,用审计链完整性
  换可用性)。

## 彩排

```bash
# 1. Bring up demo with strict mode enabled (default for
# production profile).
make demo-up DEMO_MODE=invoice

# 2. Generate a few audit rows.
make demo-up DEMO_MODE=decision

# 3. Inject a "key rotation" scenario by replacing one
# producer's signing key WITHOUT updating the verifier's trust
# store. Easiest via re-running pki-init with a new key, then
# restarting the sidecar:
docker exec spendguard-pki-init /generate.sh --rotate-sidecar
docker restart spendguard-sidecar

# 4. Generate more audit traffic.
make demo-up DEMO_MODE=decision

# 5. Confirm quarantine row appears with reason='unknown_key'.
docker exec spendguard-postgres psql -U spendguard -d spendguard_canonical -c "
  SELECT reason, count(*) FROM audit_signature_quarantine GROUP BY reason;
"
# Expected: unknown_key reason with at least 1 row.

# 6. Mitigation rehearsal: update the trust store + restart
# canonical-ingest, then verify new rows land in canonical_events
# (old rows stay in quarantine for the manual replay step).

make demo-down
```

## 相关

- L5 SLO 定义:`docs/site/docs/operations/slos.md` 的 L5 行
- 告警:`deploy/observability/prometheus-rules.yaml` 里的 A5 `SpendGuardCanonicalRejectsHigh`
- slos.md 里的 D3(签名失败处理)—— 高层版本
- PR #2 round 1 提交 `a4dea4b` —— 非 strict admit 计数器
- PR #2 round 7+8 提交 `409c220`、`d019e94` —— SP 侧 literal-pin 放宽,
  让真实的已签名行能通过
