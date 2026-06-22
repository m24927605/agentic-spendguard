---
title: "演練:strict-signature quarantine 突增"
---

每季演練。驗證在簽章失敗情境下的稽核鏈完整性保證:當某個 producer 的簽章金鑰輪替、但 verifier 的 trust store 沒有同步跟上時,canonical_ingest verifier 必須把每一筆受影響的 row 都隔離到 `audit_signature_quarantine`(strict mode 下),或是 admit 並 bump admit-counters(non-strict mode 下)。不論哪一種,絕對不能 drop 或偷偷重新編碼那段 bytes。

這份演練是以下 unit tests 的 live 對應版本:`services/canonical_ingest/src/verifier.rs::tests::*`,以及 `services/canonical_ingest/src/metrics.rs::tests` 裡的 metrics tests。

## 這份演練在測什麼

- Strict-mode:告警 A5 `SpendGuardCanonicalRejectsHigh` 觸發。
- Non-strict mode(PR #2 round 1 P2#3 fix,commit `eec0404`):`unknown_key_admitted_total` 與 `invalid_signature_admitted_total` 這兩個 counter 會 bump,但 row 仍然落進 `canonical_events`,所以在 rolling key rotation 期間稽核鏈不會斷。
- S7 金鑰登記表(canonical-ingest migrations 0008/0009 裡的 `signing_keys` + `signing_key_revocations` tables)—— 隔離原因會區分 `key_expired` / `key_revoked` / `key_not_yet_valid` / `unknown_key` / `invalid_signature`。

## 症狀(on-call 會看到什麼)

- 告警 A5 `SpendGuardCanonicalRejectsHigh` 觸發中。
- `audit_signature_quarantine` 的 row 數一路爬升。
- `canonical_events` 數量成長變慢(strict mode 下會直接持平)。
- `audit_outbox.pending_forward = TRUE` 的數量爬升 —— forwarder 一直在對同一批被拒的 row 重試。
- 使用者端可見性:對 producer 沒有立即影響(sidecar / ledger / webhook 還是照常寫 row)。稽核端的 consumer 會看到 quarantine 持續長大;合規端會看到缺口。

## 第一步檢查

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

## 緩解(短期解封)

走哪條路,取決於第一步裡哪個 `reason` 佔大宗:

### `unknown_key` 佔大宗

Producer 正在用一把 verifier 不認得的金鑰。最可能的原因:金鑰輪替先 deploy 到 producer 端,canonical-ingest 上的 trust-store 更新還沒跟上。

1. **找出這把新金鑰**(第二步 + producer 最近的 log)。
2. **把它加進 trust store**:
   ```bash
   kubectl edit secret spendguard-signing-trust-store
   # Append the new public key + valid_from window
   kubectl rollout restart deployment canonical-ingest
   ```
3. **Replay 那些被隔離的 row**:PR #2 round 1 的隔離機制會原封不動保留原始 bytes。trust store 更新後,從 `audit_signature_quarantine` table 手動 re-ingest 回 `canonical_events`(S8-followup 功能;目前還得手動下 SQL)。

### `invalid_signature` 佔大宗

這比較嚴重 —— bytes 跟它宣稱的簽章對不起來。可能性:
- Producer 程式碼 regression(簽到錯的 canonical bytes)
- 線路上有人在動手腳(mTLS 設定有問題?)

1. **立刻停掉受影響的 producer**,直到根因釐清為止:
   ```bash
   kubectl scale deployment <producer-name> --replicas=0
   ```
2. **拿 producer image 跟 known-good 版本 diff**,看 canonical-form 序列化有沒有改動。
3. **在排除 tampering 之前,不要 drop 或 replay quarantine row** —— 那段 bytes 是鑑識證據。

### `key_expired` / `key_revoked` 佔大宗

S7 的 validity-window 強制檢查。Producer 正在用一把已經過了 `valid_until`、或 `revoked_at` 之後的金鑰簽章。

1. **把 producer 的簽章材料輪替到一把目前有效的金鑰**。
2. **稽核這段缺口**:在 `valid_from`-to-`valid_until` 窗內、用該金鑰簽的 row 仍然合法(當時是用一把當下有效的金鑰簽的);窗外才簽的 row 則代表 producer 設定出了 bug。

## 升級

- **5 分鐘**持續突增 → page platform oncall。
- **15 分鐘**還沒診斷出來 → page sidecar/ledger team oncall(看是哪個 producer 受影響)。
- **`invalid_signature` >0 row** → 立即 page security team(可能有人動手腳)。
- **30 分鐘以上**在 strict mode 下持續隔離 → 考慮暫時切到 non-strict(operator 決策,需要 Helm gate ack —— 這是在你修根因的期間,拿稽核鏈完整性換可用性)。

## 演練操作

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

## 相關

- L5 SLO 定義:`docs/site/docs/operations/slos.md` 第 L5 列
- 告警:A5 `SpendGuardCanonicalRejectsHigh`,位於 `deploy/observability/prometheus-rules.yaml`
- slos.md 裡的 D3(signature-failure handling)—— 高層次版本
- PR #2 round 1 commit `a4dea4b` —— non-strict admit counters
- PR #2 round 7+8 commits `409c220`、`d019e94` —— SP 端的 literal-pin relaxations,讓真正有簽章的 row 通過
