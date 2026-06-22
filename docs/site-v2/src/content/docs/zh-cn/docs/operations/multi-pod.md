---
title: "多 Pod 部署 runbook(Phase 5 S5)"
---

本 runbook 讲怎么安全地把 sidecar、outbox-forwarder、ttl-sweeper 三个服务做横向 / 多节点部署。前提是 S1(lease 原语)、S2(per-pod producer instance id)、S3(Ledger fencing RPC)、S4(sidecar fencing-lease 生命周期)都已经上线。

**一句话**:outbox-forwarder 和 ttl-sweeper 可以安全地扩到 N 个 pod,走 leader election。sidecar 是 DaemonSet —— 每个节点一个 pod —— 任一时刻只有一个 sidecar pod 持有配置的 fencing scope,其余在启动时 fail-closed,等接管。这是 **active/standby**,不是横向扩容。

## 各组件的扩容模型

### outbox-forwarder

- **类型**:Deployment。
- **多 Pod 模型**:leader election。只有 leader 去 poll `audit_outbox` 表并转发到 canonical-ingest;standby 副本对 lease 做 heartbeat,具备接管资格。
- **Helm gate**:当 `leaderElection.mode = disabled` 时,`outboxForwarder.replicas > 1` 会被拒绝(S1 gate;符合设计预期)。
- **该怎么配**:
  ```yaml
  outboxForwarder:
    replicas: 2  # or 3 for region-spread
  leaderElection:
    mode: postgres   # or k8s after S5+S7
    region: us-west-2
    ttlMs: 15000
    renewIntervalMs: 5000
  ```
- **故障转移行为**:当 active leader pod 挂掉,Postgres lease 的 TTL 在 `ttlMs`(默认 15s)后过期。某个 standby 调用 `acquire_lease` SP 拿到锁,转发恢复。不会产生重复的 canonical events,因为 `audit_outbox.pending_forward` 是持久化游标。

### ttl-sweeper

- **类型**:Deployment。
- **多 Pod 模型**:和 outbox-forwarder 完全一致(leader election;只有 leader 去 poll + 释放过期的 reservation)。
- **Helm gate**:当 `leaderElection.mode=disabled`(S1)时,`ttlSweeper.replicas > 1` 被拒绝。
- **推荐**:`replicas: 2` 做 HA。再往上加副本不会提升吞吐(只有一个 pod 在 sweep)。

### sidecar

- **类型**:DaemonSet(按设计每节点一个 pod —— 与挂载 UDS adapter socket 的 workload pod 同节点共置)。
- **多 Pod 模型**:每个 pod 通过 downward API 从 `metadata.name` 派生出唯一的 `workload_instance_id`(S2)。启动时每个 pod 调用 `Ledger.AcquireFencingLease`(S4);Ledger SP 用 `FOR UPDATE` 串行化,把 lease 只授予一个 pod。其余 pod 在启动时 fail-closed,报 `S4: acquire fencing lease at startup`,停在 CrashLoopBackOff 或 Pending。
- **这不是横向扩容**。任一时刻每个 fencing scope 只有一个 active 的、对外提供决策的 sidecar。
- **那为什么还用 DaemonSet?** 为了共置:每个节点都有一个本节点 app pod 可达的 UDS socket。fencing scope 是 per-tenant(或 per-tenant×region)粒度;只有一个节点的 sidecar 持有它。
- **Helm gate**:必须设 `sidecar.acknowledgeMultiPod=true`,以此表明 operator 明确知晓 active/standby 语义。启用多 Pod 时绝不能设 `workloadInstanceIdOverride`(override 意味着单 Pod 身份)。

## 故障转移与接管

### Sidecar fencing 接管

当 active sidecar pod 挂掉(OOM、eviction、节点故障):

1. 该 pod 的 `AcquireFencingLease` lease 在 `SPENDGUARD_SIDECAR_FENCING_TTL_SECONDS`(默认 30s)后超时。
2. 其他节点上启动时崩溃的 standby sidecar 被 kubelet 重启。重启后它们再次调用 `Ledger.AcquireFencingLease`。
3. Ledger SP 看到前一个 lease 已过期,给新 pod 授予一个 `takeover` 动作,带 `epoch_increment = 1`。新 pod 的 audit 行现在以 `fencing_epoch = N+1` 签名。
4. 旧 pod 任何还在飞行中的决策,如果试图用 `fencing_epoch = N` 提交,都会被 Ledger 的 CAS 检查拒绝(`FENCING_EPOCH_STALE` 错误)。审计不变式("无有效 epoch 则无副作用")得以保持。

Operator dashboard 暴露:
- `spendguard_sidecar_fencing_epoch` gauge(per pod)
- `spendguard_sidecar_fencing_acquire_action_total{action}` counter(acquire / renew / takeover)—— takeover 出现尖峰说明发生了故障转移。

### Outbox-forwarder leader 切换

- `coordination_lease_history` 表是审计日志:每次接管都会写一行,`event_type = 'taken_over'`,`transition_count + 1`。
- Operator 监控 `spendguard_outbox_forwarder_leader_age_seconds` histogram 和 `coordination_lease_history` 行。

## 回滚到单 Pod

三个服务的回滚都很简单:

```yaml
sidecar:
  acknowledgeMultiPod: false  # if you set it
outboxForwarder:
  replicas: 1
ttlSweeper:
  replicas: 1
```

不需要动数据库。lease/fencing 状态都在 Postgres 里,由当下还活着的那个 pod 续约 / 接管。

## 混沌演练 checklist

S5 的验收标准要求一个 "kind test:两个 sidecar、两个 forwarder、两个 sweeper,全部健康"。在那个自动化测试落地之前(已推迟到 S5-followup),operator 应当手动验证:

1. 用 `outboxForwarder.replicas=2`、`ttlSweeper.replicas=2`、sidecar DaemonSet 部署到一个 2 节点集群。
2. 验证 `coordination_leases` 里每个 `lease_name`(`outbox-forwarder`、`ttl-sweeper`)恰好显示一个 leader。
3. 验证 `fencing_scopes` 里每个 scope 恰好显示一个 `current_holder_instance_id`。
4. `kubectl delete pod <leader>`。等 `ttlMs + grace`(默认 ~30s)。
5. 验证 `coordination_lease_history` 多出一行 `taken_over`。
6. 验证 ledger / canonical-ingest 没看到重复的 audit 行(`audit_outbox_global_keys` 在 `(tenant, workload_instance_id, producer_sequence)` 上的 UNIQUE 会拒绝重复)。
7. 对 sidecar 重复一遍:`kubectl delete pod <active-sidecar>` —— 另一节点上的 standby sidecar 以 epoch+1 接管。

## 可观测性不变式

每个启用了 S1+S4 的部署都应该对以下情况告警:

- `coordination_lease_history` 中 `event_type='taken_over'` 的行每小时每个 lease 超过 1 次 —— 很可能是 lease-flap(TTL 太短或网络分区)。
- `fencing_scope_events` 中 `action='promote'` 每小时超过 1 次 —— sidecar 接管风暴。
- sidecar pod 处于 `CrashLoopBackOff`,日志里带 `acquire fencing lease at startup` 持续超过 5 分钟 —— 通常意味着预置的 scope 行缺失,或 workload 身份发生冲突。

## 已知限制(S5-followup)

1. **Per-pod fencing scope** 还不支持。所有节点上的所有 sidecar pod 共用配置的 `sidecar.fencingScopeId`。真正的横向扩容需要 per-pod 的 scope 分配;作为 S5-followup 跟踪。
2. 上面那个混沌演练的 **自动化 kind test** 被推迟。
3. 接管期间的 **sidecar pre-stop drain** 已经到位(S4),但接管 SP 目前还不会吊销前一个持有者的 lease —— 它只是让 TTL 过期。更快的接管需要一个显式的 revoke RPC。
