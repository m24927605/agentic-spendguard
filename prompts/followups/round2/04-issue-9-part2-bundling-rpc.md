# Round-2 #9 part 2 — Approval bundling RPC + sidecar wiring

GitHub issue: #9 (still open after PR #15 shipped part 1). Original prompt:
`../02-issue-9-approval-bundling-sp.md`.

## What part 1 already shipped (PR #15, merge `fba2c1d`)

- migration 0036 schema: `bundled_at`, `bundled_ledger_transaction_id`
  columns + immutability trigger update
- `mark_approval_bundled` SP — atomic state assertion + idempotent UPDATE

## Round-2 strategy (split into 4 sub-PRs to keep each reviewable)

### PR 9b — Proto + ledger handler

1. `proto/spendguard/ledger/v1/ledger.proto` add:
   - `GetApprovalForResume(approval_id) → (state, decision_context, requested_effect)`
   - `MarkApprovalBundled(approval_id, ledger_transaction_id) → (was_first_bundling, ledger_transaction_id)`
2. `services/ledger/src/handlers/get_approval_for_resume.rs` — read
   `approval_requests` row + return state + decision_context +
   requested_effect for the sidecar's resume path
3. `services/ledger/src/handlers/mark_approval_bundled.rs` — call SP
   from PR #15
4. Wire both into `services/ledger/src/main.rs` gRPC server
5. Cargo test: at least 1 unit test per handler + proto regen smoke

### PR 9c — Sidecar Resume handler real path

1. `services/sidecar/src/server/adapter_uds.rs::resume_after_approval` —
   replace stub:
   - Call `Ledger.GetApprovalForResume(approval_id)`
   - If `state='approved'`: build a fresh `RequestDecision` from
     decision_context + requested_effect, call existing
     `Ledger.ReserveSet` with `idempotency_key = sha256(approval_id || ":resume")`
   - Call `Ledger.MarkApprovalBundled(approval_id, tx_id)`
   - Return `DecisionResponse` with the new reservation
2. If `state='denied'`: return `ResumeAfterApprovalDenied`
3. Other states: return typed `Error`

### PR 9d — Python adapter SDK

1. `sdk/python/src/spendguard/client.py::ApprovalRequired.resume()` —
   call `ResumeAfterApproval` over UDS
2. Adapter exception flow: typed `ApprovalLapsedError` /
   `ApprovalDeniedError` etc.
3. Update `templates/onboarding/python-langchain/sdk_adapter.py` if
   relevant

### PR 9e — Demo mode

1. New `DEMO_MODE=approval`:
   - Contract.yaml has a REQUIRE_APPROVAL rule
   - Demo flow: reserve via sidecar → REQUIRE_APPROVAL response →
     simulate approver via control_plane REST → resume → real
     reservation lands
2. New `verify_step_approval.sql` asserts:
   - 1 approval_request row in `approved` state
   - 1 ledger_transactions row with `bundled_ledger_transaction_id`
     populated
   - Audit chain rows for both decision + resume

## Acceptance per PR

- Sub-PR 9b: cargo test ledger crate passes; proto regenerates
- Sub-PR 9c: cargo test sidecar passes; existing demo modes regress-clean
- Sub-PR 9d: pytest sdk passes
- Sub-PR 9e: `make demo-up DEMO_MODE=approval` PASS

## Round-2 starting commit

Build off latest main; PR #15 (part 1) is at commit `fba2c1d`.
