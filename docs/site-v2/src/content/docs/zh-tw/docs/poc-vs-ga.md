---
title: "POC vs GA 的關卡"
description: >-
  誠實盤點 Agentic SpendGuard 哪些能力今天就可以上 production,哪些還卡在
  GA hardening slices 後面;每一道還沒過的關卡,都附上對應的 roadmap 條目,
  說明要靠哪一塊把它收掉。
---


這份 POC 裡哪些東西已經 production-ready,哪些是我們明擺著留到
GA hardening 才處理的,這裡講清楚。

## ✅ 已具備 production 雛形(已做過 end-to-end 驗證)

- 所有 service 之間都走 mTLS gRPC
- Postgres SERIALIZABLE + transactional `audit_outbox`
- 3 階段 commit 生命週期(estimated → reported → invoiced)
- 原子化 reservation + TTL release
- Contract DSL hot-path evaluator(`<5ms`)
- 6 個 framework adapter(Pydantic-AI、LangChain、LangGraph、OpenAI
  Agents、真的 OpenAI / Anthropic)
- 跨 provider、以 USD 計價的 budget
- Operator dashboard + control plane API
- Helm chart + Terraform AWS module

## ⛔ GA 關卡(還沒到可以上 production 的程度)

下面這幾項,是真的會擋住 production 部署的:

### 多 pod 的工作分配

- `sidecar.replicas > 1` 會在 `audit_outbox_global_keys` table 上
  撞出 `producer_sequence` 的 race。
- `outbox-forwarder.replicas > 1` 會把同一筆 row double-forward(目前
  還沒有 leader election)。
- `ttl-sweeper.replicas > 1` 也是一樣的問題。
- **解法**:leader election(k8s Lease primitive 或 DB row-lock 兩種做法)
  + 每個 instance 各自切分 producer_sequence。

### Fencing acquire RPC

- POC 是直接用 SQL 把一個 fencing scope 的 `current_epoch=1` 種進去。
- production 部署需要一個 `Ledger.AcquireFencingLease()` RPC,在接手時用
  CAS 把 epoch 加一;sidecar 啟動後、發出任何 reserve 之前,都必須先呼叫它。
- 少了這個,sidecar 重啟之後就可能重用到過期的 lease。

### 真正的 signing key

- POC 在 canonical_ingest 裡用的是 `strict_signatures=false`;producer
  的 signature 是 placeholder `b''`。
- production 需要真的 Ed25519 key rotation + 由 KMS 撐腰的簽章,照
  Stage 2 §17 來做。

### CI quarantine 的耐久性

- `audit_quarantine` migration(canonical 0003)現在只是個 placeholder。
- ORPHAN_OUTCOME 的 reaper 還沒做 — 找不到對應 decision 的 outcome,目前
  會永遠卡在 audit_outbox 裡。

### Chaos 測試套件(Stage 2 §13)

- spec 裡列了 7 個情境,但都還沒自動化:
  - ReserveSet commit 過程中發生 network partition
  - decision 進行到一半 Postgres failover
  - sidecar publish 到一半 OOM
  - 諸如此類

### 真的接 provider webhook

- demo 的 webhook receiver 驗的是 mock-llm 的 HMAC。
- OpenAI 沒有出 billing webhook → 需要寫一個 `/v1/usage` 的 poller。
- Anthropic enterprise webhook 需要拿到他們的 signing key。
- 每家 provider 的 adapter,都是 operator 一家一家去接的工。

### 自動更新價格的 poller

- `pricing_table` 的基礎設施在 Phase 4 O3 就上了。
- 定期去對 provider 文件做 refresh 這件事還留著沒做。
- 靜態 YAML 對 POC 夠用;production 需要每天同步。

### 跨 region failover

- Stage 2 spec 已經把設計講完了(跨 region replication + failover
  policy);實作留到 GA。

## 🟡 還沒做完的 primitive

- **Refund / Dispute / Compensate(Step 10)** — Contract §5.1a spec
  已經 lock 了;ledger 那支 SP 還沒實作。本質上就是把 Step 9 的
  invoice-reconcile pattern 機械式延伸出去而已。
- **CEL evaluator** — POC 用的是宣告式的 when/then。完整的 CEL
  predicate 語言排在 v1 roadmap。
- **Bundle hot-reload** — POC 只在啟動時載入。帶 last-known-good fallback
  的 hot-reload 留到 GA。
- **多層次的 approval flow** — `REQUIRE_APPROVAL` 在 POC 裡是終點。
  接 operator(Slack / PagerDuty / 之類的)算 product 的工,不是
  infrastructure。

## 這對使用者代表什麼

- **試玩 POC**:clone + `make demo-up`,全部都會動。
- **跑在 dev k8s**:Helm chart 可以用。replica 預設單 pod,避開多 pod
  的資料 hazard。
- **跑在 production**:還不行。fencing-acquire + 多 pod 這幾道關卡得先
  落地。在「我敢把我 agent 的 budget 押在這上面」成立之前,還有一大塊
  工要做。

設計、實作、測試驗收、review 關卡怎麼把這些 blocker 拆成可以各自獨立出
的 PR,請看 [GA hardening slices](roadmap/ga-hardening-slices.md)。

POC 的每一個限制,我們都在 code 跟 docs 裡明確標出來,讓 operator 從頭
讀到尾,就能稽核自己拿到的到底是什麼。
