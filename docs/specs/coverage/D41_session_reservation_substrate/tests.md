# D41 session reservation substrate - Tests

## 1. Ledger tests

| ID | Test | Verifies |
|---|---|---|
| TP-D41S-01 | Reserve session inserts a live hold with reserved > 0. | Basic reserve. |
| TP-D41S-02 | Duplicate reserve with same idempotency key and same payload returns same outcome. | Idempotency replay. |
| TP-D41S-03 | Duplicate reserve with same key and different payload returns conflict. | Replay safety. |
| TP-D41S-04 | Commit delta increases committed amount and decreases remaining hold. | Delta accounting. |
| TP-D41S-05 | Duplicate commit delta same payload does not double count. | Streaming commit idempotency. |
| TP-D41S-06 | Duplicate commit delta different amount returns conflict. | Commit replay safety. |
| TP-D41S-07 | Commit delta that exceeds reserved amount is rejected. | Hard cap. |
| TP-D41S-08 | Release settles only uncommitted remainder and is idempotent. | Settlement. |
| TP-D41S-09 | TTL sweep expires live session and releases remainder. | Crash backstop. |

## 2. SDK tests

| ID | Test | Verifies |
|---|---|---|
| TP-D41S-10 | TS client builds reserve/commit/release envelopes with handshake session id. | TS wire shape. |
| TP-D41S-11 | Python client builds equivalent envelopes. | Python wire shape. |
| TP-D41S-12 | TS/Python idempotency fixture for session operations is byte-equivalent if new derivation helpers are added. | Cross-language parity. |
| TP-D41S-13 | Zero or negative commit delta rejected client-side and server-side. | Positive delta invariant. |
| TP-D41S-14 | Tuple mismatch on commit is rejected. | Pricing/unit/window integrity. |

## 3. Audit tests

| ID | Test | Verifies |
|---|---|---|
| TP-D41S-15 | Reserve/commit/release events are signed CloudEvents and canonical-ingest compatible. | Audit chain. |
| TP-D41S-16 | Denied session reserve emits `spendguard.audit.session.denied`. | Denial visibility. |
| TP-D41S-17 | TTL expiration emits `spendguard.audit.session.expired`. | Expiry visibility. |

## 4. Acceptance gates

| ID | Command | Pass condition |
|---|---|---|
| TA-D41S-01 | Rust ledger/sidecar test command selected by implementation | exits 0. |
| TA-D41S-02 | TS SDK test command | exits 0. |
| TA-D41S-03 | Python SDK test command | exits 0. |
| TA-D41S-04 | `make demo-down` | exits 0. |
| TA-D41S-05 | `make demo-up DEMO_MODE=session_reservation` | prints `[demo] session_reservation ALL 7 steps PASS`. |
| TA-D41S-06 | `make -C deploy/demo demo-verify-session-reservation` | SQL hard gates pass. |
| TA-D41S-07 | Existing request-scoped adapter demo smoke selected in acceptance.md | still passes. |

## 5. Slice mapping

| Slice | Tests |
|---|---|
| `COV_D41S_01_session_contract_spec_and_proto` | TP-D41S-10..13 skeleton wire tests |
| `COV_D41S_02_ledger_session_reservation` | TP-D41S-01..09, TP-D41S-14 |
| `COV_D41S_03_sdk_session_client` | TP-D41S-10..13 |
| `COV_D41S_04_streaming_commit_and_reconnect` | TP-D41S-02..08, reconnect replay tests |
| `COV_D41S_05_substrate_demo_gate` | TP-D41S-15..17, TA-D41S-01..07 |

## 6. Bridge follow-up

Runtime sidecar coverage is specified separately in
[`D41_sidecar_session_bridge/tests.md`](../D41_sidecar_session_bridge/tests.md).
Those tests are not a substitute for the direct-ledger D41S substrate tests:
bridge closeout must run both the new sidecar-path gates and the existing
`session_reservation` direct substrate demo.
