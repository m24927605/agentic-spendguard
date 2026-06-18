# HARDEN_03 Production-Blocker Issue Triage

Date: 2026-05-31
Scope: open GitHub issues #90-#177 in `m24927605/agentic-spendguard`.

Priority definitions:
- P1: production readiness blocker in HARDEN_03 scope, or explicit cross-slice P1 that must remain tracked.
- P2: security, correctness, or performance work that is important but assigned to a later HARDEN slice or non-blocking follow-up.
- P3: documentation, cleanup, test broadening, performance polish, duplicate, or historical cleanup that does not block HARDEN_03.

Staff+ panel used for triage:
- Software Architect: classify behavioral correctness and spec gaps.
- Backend Architect: classify service, migration, and API production risks.
- Security Engineer: classify credential, replay, PII, network, and trust-boundary risks.
- Database Optimizer: classify RLS, migration, indexing, and Postgres operational risks.
- Domain Expert: classify SpendGuard predictor-upgrade gating, audit-chain, and SDK impact.

## P1 Closure Set

| Issue | Priority | HARDEN_03 disposition |
|---:|---|---|
| #90 Python SDK proto stub missing STOP_RUN_PROJECTION | P1 | Fixed by `307eed4`; SDK maps and surfaces `STOP_RUN_PROJECTION`, with enum/round-trip/client tests. |
| #137 Control plane sampling-rate API persistence | P1 | Fixed by `071ae54`; API persists per-tenant/model overrides under RLS and emits audit outbox events. |
| #143 verify-chain admission for tokenizer_drift_alert | P1 | Fixed by `307eed4`; verify-chain only requires prediction mirrors for decision/outcome event types, so audit-routed drift alerts are admitted. |
| #145 Plaintext DB URL in Deployment manifest | P1 | Fixed by `ad85d9b`; rendered K8s manifests now use `valueFrom.secretKeyRef` for DB URLs. |
| #150 pg_indexes smoke check schemaname filter | P1 | Fixed by `307eed4`; 0018 smoke check filters `schemaname='public'`. |
| #160 integration tests | P1 | Fixed by `88461a5`; real Postgres cycle, RLS injection, and drift-alert audit-row integration tests. |
| #168 tokenizer shadow sink AppendEventsRequest envelope | P1 | Fixed by `3925551`; sink sends producer_id, signing_key_id, schema_bundle, and OBSERVABILITY route. |
| #169 sidecar/canonical mirror columns verification | P1 | Fixed by `307eed4`; append handler tests prove model and `model_family` populate aggregator mirror columns. Existing sidecar/egress code already emits these payload fields. |
| #171 per-tenant SVID cert minting | P1 | Cross-slice P1. Remains open and owned by HARDEN_08 per `docs/internal/slices/HARDEN_08_per_tenant_svid_cert.md`. |

## Full Reviewed Set

| Issue | Priority | Rationale |
|---:|---|---|
| #90 | P1 | SDK runtime correctness; closed in HARDEN_03. |
| #91 | P3 | Spec title/doc cleanup. |
| #92 | P2 | Schema-bundle parity gap; correctness follow-up, not HARDEN_03 P1. |
| #93 | P3 | Operator upgrade-path documentation. |
| #94 | P3 | Unit-test broadening. |
| #95 | P3 | Tokenizer dead-weight/perf cleanup. |
| #96 | P2 | Readiness ordering bug; SLICE_10 prereq but not in HARDEN_03 P1 set. |
| #97 | P3 | Documentation clarification. |
| #98 | P3 | Code quality naming cleanup. |
| #99 | P3 | Test doc accuracy. |
| #100 | P3 | Spec/SLO amendment. |
| #101 | P3 | Spec amendment. |
| #102 | P3 | Asset duplication/perf cleanup. |
| #103 | P2 | Workspace NetworkPolicy hardening; covered later by HARDEN_07. |
| #104 | P3 | Test performance. |
| #105 | P3 | Production deployment documentation. |
| #106 | P2 | Security backlog; owned by HARDEN_05. |
| #107 | P2 | Security surface reduction; owned by HARDEN_05. |
| #108 | P3 | Fixture extensibility. |
| #109 | P3 | Fixture broadening. |
| #110 | P2 | Per-tenant rate limiting follow-up. |
| #111 | P2 | Cross-tenant correlation hardening. |
| #112 | P3 | Shadow-worker calibration follow-up. |
| #113 | P3 | Proto comment documentation. |
| #114 | P2 | SLICE_10 tokenizer request-size prereq. |
| #115 | P3 | Dispatch-table footgun cleanup. |
| #116 | P3 | Encoder duplication refactor. |
| #117 | P3 | Test fixture quality. |
| #118 | P3 | Benchmark broadening. |
| #119 | P3 | Test fixture quality. |
| #120 | P3 | Dead dependency/comment cleanup. |
| #121 | P3 | Doc-string typo. |
| #122 | P3 | Boot observability/perf. |
| #123 | P3 | Cargo documentation accuracy. |
| #124 | P2 | License/security review follow-up. |
| #125 | P2 | Asset integrity script hardening. |
| #126 | P3 | Tamper-rejection test broadening. |
| #127 | P2 | Algorithmic complexity/timeout hardening. |
| #128 | P2 | Migration hardening. |
| #129 | P3 | Cache smoke-test broadening. |
| #130 | P3 | Dispatch performance. |
| #131 | P3 | Documentation drift. |
| #133 | P3 | OpenAI envelope test/design cleanup. |
| #134 | P3 | Unused helper cleanup. |
| #135 | P3 | Documentation / shadow-worker tuning dependency. |
| #136 | P3 | Documentation formatting. |
| #137 | P1 | API persistence; closed in HARDEN_03. |
| #138 | P2 | Security backlog; owned by HARDEN_05. |
| #139 | P3 | Provider expansion. |
| #140 | P3 | Llama tuning dependency. |
| #141 | P3 | Provider key rotation runbook. |
| #142 | P2 | Security backlog; owned by HARDEN_05. |
| #143 | P1 | Audit-chain admission; closed in HARDEN_03. |
| #144 | P2 | Security backlog; owned by HARDEN_05. |
| #145 | P1 | K8s secret handling; closed in HARDEN_03. |
| #146 | P2 | DB privilege hardening. |
| #147 | P3 | Spec/index-name sync. |
| #148 | P3 | Test improvement. |
| #149 | P3 | Schema default follow-up. |
| #150 | P1 | Migration smoke check correctness; closed in HARDEN_03. |
| #151 | P3 | Tokenizer boot integration test. |
| #152 | P3 | Literal type assertion test. |
| #153 | P3 | Historical/covered by #145. |
| #154 | P3 | Slice doc checklist drift. |
| #155 | P3 | Historical commit scope cleanup. |
| #156 | P3 | Serialization/perf concern. |
| #157 | P2 | Drift alert dedup; security backlog. |
| #158 | P3 | Source URI spec drift. |
| #159 | P3 | Spec event-type sync; covered by HARDEN_04. |
| #160 | P1 | Integration coverage; closed in HARDEN_03. |
| #161 | P2 | PredictResponse contract follow-up. |
| #162 | P2 | Drift alert numeric hardening. |
| #163 | P2 | RLS sentinel hardening. |
| #164 | P3 | Postgres runbook. |
| #165 | P2 | Predict RPC rate limit. |
| #166 | P3 | Index usefulness/perf follow-up. |
| #167 | P3 | Spec amendment. |
| #168 | P1 | AppendEventsRequest envelope; closed in HARDEN_03. |
| #169 | P1 | Mirror column verification; closed in HARDEN_03. |
| #170 | P3 | Duplicate of #157. |
| #171 | P1 | Cross-slice P1; HARDEN_08 owner, remains open. |
| #172 | P2 | Security/spec audit ambiguity. |
| #173 | P2 | Security reason-string cap. |
| #174 | P3 | Thundering-herd performance. |
| #175 | P3 | Serve-stale enhancement. |
| #176 | P2 | Security length caps. |
| #177 | P3 | NotServing cleanup. |
