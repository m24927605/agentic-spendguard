---
title: "POC vs GA 准入条件"
description: >-
  对 Agentic SpendGuard 各项能力的诚实评估：哪些今天就能上生产,
  哪些卡在 GA hardening slices 后面还没放行,并给出每个未关闭的准入条件
  以及对应的 roadmap 收口项。
---


这个 POC 里哪些能力已经能上生产,哪些明确推迟到 GA hardening。

## ✅ 接近生产形态(端到端已验证)

- 全部服务走 mTLS gRPC
- Postgres SERIALIZABLE + 事务性 `audit_outbox`
- 3 阶段 commit 生命周期(estimated → reported → invoiced)
- 原子 reservation + TTL release
- Contract DSL 热路径求值器(`<5ms`)
- 6 个框架 adapter(Pydantic-AI、LangChain、LangGraph、OpenAI
  Agents、真实 OpenAI / Anthropic)
- 跨 provider 的 USD 计价 budget
- 运维 dashboard + 控制面 API
- Helm chart + Terraform AWS module

## ⛔ GA 准入条件(尚未达到生产可用)

下面这些才是真正阻塞生产部署的项:

### 多 pod 工作分发

- `sidecar.replicas > 1` 会在 `audit_outbox_global_keys` 表上引发
  `producer_sequence` 竞态。
- `outbox-forwarder.replicas > 1` 会把同一行重复 forward 一遍
  (还没有 leader election)。
- `ttl-sweeper.replicas > 1` 同理。
- **修复方案**:leader election(k8s Lease primitive 或基于 DB 行锁)
  + 按实例切分 producer_sequence。

### Fencing acquire RPC

- POC 直接用 SQL 把 fencing scope 种成 `current_epoch=1`。
- 生产部署需要 `Ledger.AcquireFencingLease()` RPC,在接管时 CAS 自增 epoch;
  sidecar 启动后、签发任何 reserve 之前必须先调它。
- 没有这个,sidecar 重启可能复用陈旧的 lease。

### 真实签名密钥

- POC 在 canonical_ingest 里用 `strict_signatures=false`;producer
  签名是占位的 `b''`。
- 生产需要真实的 Ed25519 密钥轮换 + KMS 托管签名,见 Stage 2 §17。

### CI quarantine 持久化

- `audit_quarantine` migration(canonical 0003)目前是占位实现。
- ORPHAN_OUTCOME reaper 还没做 —— 没有对应 decision 的 outcome
  目前会一直滞留在 audit_outbox 里。

### 混沌测试套件(Stage 2 §13)

- 规范里列了 7 个场景,但都还没自动化:
  - ReserveSet commit 过程中网络分区
  - decision 进行到一半 Postgres failover
  - publish 中途 sidecar OOM
  - 等等

### 真实 provider webhook 集成

- demo 的 webhook receiver 验的是 mock-llm 的 HMAC。
- OpenAI 不提供 billing webhook → 需要 `/v1/usage` 轮询器。
- Anthropic 企业版 webhook 需要他们的签名密钥。
- 每家 provider 的 adapter 是 operator 逐个落地的活儿。

### Pricing 自动更新轮询器

- `pricing_table` 这套基础设施在 Phase 4 O3 已经落地。
- 对着 provider 文档定期刷新还没做。
- 静态 YAML 在 POC 够用;生产需要每日同步。

### 多区域 failover

- Stage 2 spec 覆盖了设计(跨区域复制 + failover 策略);实现是 GA 的事。

## 🟡 未完成的原语

- **Refund / Dispute / Compensate(Step 10)** —— Contract §5.1a 规范
  已锁定;ledger SP 尚未实现。本质上是 Step 9 invoice-reconcile
  模式的机械性扩展。
- **CEL 求值器** —— POC 用的是声明式 when/then。完整的 CEL
  谓词语言排在 v1 roadmap 上。
- **Bundle 热加载** —— POC 只在启动时加载。带 last-known-good 回退的
  热加载是 GA 的事。
- **多级审批流** —— POC 里 `REQUIRE_APPROVAL` 是终态。对接 operator
  (Slack / PagerDuty / 等等)属于产品工作,不是基础设施。

## 这对用户意味着什么

- **试 POC**:clone + `make demo-up`,全都能跑。
- **跑 dev k8s**:Helm chart 可用。单 pod replica 的默认值
  规避了多 pod 的数据风险。
- **上生产**:还不行。fencing-acquire + 多 pod 这几个准入条件
  得先落地。在"我敢把 agent 的预算押在这上面"成立之前,
  还得再啃一大块活儿。

设计、实现、测试验收和 review 准入条件如何把这些阻塞项拆成
可独立交付的 PR,见 [GA hardening slices](roadmap/ga-hardening-slices.md)。

我们在代码和文档里显式标注了每一处 POC 限制,这样从头到尾读一遍的
operator 就能审清自己拿到的到底是什么。
