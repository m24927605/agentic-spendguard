# Agentic SpendGuard — Real-Stack LangChain End-to-End Verification

**Date**: 2026-05-13
**Branch**: `main` after F1 + F2 + F3a merge (HEAD `e198273`)
**Purpose**: V1 Phase 1-4 from `docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md` — prove the **real** Rust sidecar stack (not the benchmark shim) works end-to-end with a **real** LangChain agent calling **real** OpenAI.

This is the precondition for opening upstream framework docs PRs (P2-4). Without this evidence, any LangChain maintainer trying `make demo-up` would see the stack panic at startup and close the PR as premature.

---

## TL;DR

| Decision Path | Status | Evidence |
|---|:-:|---|
| **CONTINUE** | ✅ Verified | Real LangChain `ChatOpenAI` → SpendGuard sidecar → real `gpt-4o-mini`: `output='Hello, how are you?'` |
| **STOP** | ✅ Verified | `deny` mode: sidecar returns STOP, `denied_decision` row written, **0 ledger entries**, **0 reservations**, **audit row signed** |
| **REQUIRE_APPROVAL** | 🟡 Partial | Dispatch + SDK transit OK; demo seed contract bundle does **not** ship a REQUIRE_APPROVAL rule for the demo claim shape, so decision returns CONTINUE. Resume-flow code paths are covered by unit tests (PRs #37/#38/#39) |
| **DEGRADE** | ❌ Not exercised | No DEMO_MODE wired for DEGRADE; would require adding a new mode + a DEGRADE rule in seed bundle. SDK path exists (`spendguard.errors.DegradeApplied`) but no e2e demo |

**Overall verdict**: real Rust stack **does** boot and **does** run real LangChain calls end-to-end. **The pre-F1 claim that "Phase 5 GA hardening" applied to the self-hosted demo path was incorrect**; with F1 / F2 / F3a now landed it is materially true for CONTINUE + STOP. APPROVAL and DEGRADE need additional contract bundle + demo wiring before they can be claimed as e2e-verified.

---

## Environment

- macOS Darwin 25.3.0 / Apple Silicon
- Docker Desktop, BuildKit enabled, cargo cache mounts active (`9e018ed perf(deploy): BuildKit cargo cache mounts`)
- Python 3.12 in demo container
- Rust 1.91 toolchain (`fc98cb9 round2-8a-rust-toolchain-bump`)
- `langchain >= 0.3`, `langchain-openai >= 0.2`, `openai >= 1.50`
- Real OpenAI key in `~/.env` (`OPENAI_API_KEY`); model: `gpt-4o-mini`
- Repository state: F1 (`b3b1abf`) + F2 + F3a (`f0ca4f8`) merged to main (`e198273`)

---

## Setup steps (reproducible)

```bash
git clone git@github.com:m24927605/agentic-spendguard.git
cd agentic-spendguard

# Required: real OpenAI key for CONTINUE and APPROVAL modes
export OPENAI_API_KEY="sk-..."
# Optional: Anthropic key for agent_real_anthropic
# export ANTHROPIC_API_KEY="sk-ant-..."

# CONTINUE path (full real LangChain + real OpenAI):
make demo-up DEMO_MODE=agent_real_langchain

# STOP path (Mock LLM is fine — sidecar decision is independent of LLM):
make demo-down && make demo-up DEMO_MODE=deny

# REQUIRE_APPROVAL transit path (no real OpenAI call gets made):
make demo-down && make demo-up DEMO_MODE=approval

# Always clean up after:
make demo-down
```

Each `make demo-up` cycle:
1. Docker BuildKit incrementally rebuilds the 9 Rust services + Python demo container (~3 min warm; ~30 min cold)
2. Brings up the full topology (postgres + pki + bundles-init + manifest-init + endpoint-catalog + ledger + canonical-ingest + sidecar + webhook-receiver + outbox-forwarder + ttl-sweeper)
3. Runs the demo container with `SPENDGUARD_DEMO_MODE=<mode>`
4. For `agent_real*` modes, F2 guard skips `demo-verify-step7` (verify SQL assumes Mock LLM's fixed token output)
5. Always runs `demo-verify-outbox-closure` to confirm the audit chain forwarding loop closes

---

## Decision Path Verification

### Path 1 — CONTINUE ✅

**Mode**: `DEMO_MODE=agent_real_langchain`
**Trigger**: budget large enough to cover one `gpt-4o-mini` call
**Expected**: LangChain `ChatOpenAI.ainvoke()` returns a real response

**Evidence** (`evidence/continue.log`):

```
[demo] handshake ok session_id=019e2064-0816-79e0-9d9d-e13ddda99e40
[demo] using real OpenAI gpt-4o-mini via LangChain
[demo] langchain ainvoke OK output='Hello, how are you?' run_id=019e2064-09fc-7169-a573-00d799c9b1f8
[demo] DEMO_MODE=agent_real_langchain — skipping demo-verify-step7.
[demo] verify_step7.sql asserts hardcoded Mock LLM token counts (committed=42)
[demo] which never match real provider responses (gpt-4o-mini etc. return
[demo] variable token counts). See SPENDGUARD_VIRAL_PLAYBOOK.todo.md F2.
```

**What this proves**:
- The full Rust stack boots clean (post-F1 fix)
- LangChain `ChatOpenAI` + `spendguard.integrations.langchain` wrapper works
- Sidecar handshake completes via UDS
- Real OpenAI call goes through and returns a real model response
- Audit outbox forwarder closes the loop after the call

---

### Path 2 — STOP ✅

**Mode**: `DEMO_MODE=deny`
**Trigger**: Mock LLM call against a contract bundle that issues STOP for the demo claim
**Expected**: sidecar returns STOP, no reservation made, no ledger entries, but a signed `denied_decision` row is written for audit

**Evidence** (`evidence/stop.log`):

```
=== ledger_transactions (denied_decision) ===
 denied_decision | posted        | 019e2066-6473-7a23-8293-6a13b2165b2e | t

=== ledger_entries for denied_decision (must be 0) ===
=== reservations whose source tx is a denied_decision (must be 0) ===
 spendguard.audit.decision | spendguard.audit.decision | t            | t
NOTICE:  Phase 3 wedge DENY lifecycle PASS: denied_tx=1 audit_rows=1
```

**What this proves**:
- Sidecar contract evaluator correctly returns STOP
- DB invariants: zero entries + zero reservations for the denied tx (no money moved)
- `has_audit_anchor=t`: the denied decision is signed and chained
- Audit outbox emits the decision event with the right CloudEvent type

**Why Mock LLM is fine here**: STOP happens at the decision boundary **before** the LLM call. The choice of LLM is irrelevant — the sidecar decides the call doesn't get made. The LangChain integration tested in Path 1 sends the same `request_decision` RPC; if the sidecar returned STOP under that path, the SDK would raise `DecisionStopped` and the LLM call would never be issued.

---

### Path 3 — REQUIRE_APPROVAL 🟡

**Mode**: `DEMO_MODE=approval`
**Trigger**: a $500 claim that *would* exceed a REQUIRE_APPROVAL threshold if seeded
**Observed**: decision returns CONTINUE, not REQUIRE_APPROVAL

**Evidence** (`evidence/approval.log`):

```
[demo] approval-mode connecting to sidecar at /var/run/spendguard/adapter.sock
[demo] DEMO_MODE=approval — decision returned CONTINUE without REQUIRE_APPROVAL
       (decision_id=019e206f-9575-7bc1-ba61-e502ca6c34e0).
       The seeded contract bundle does not yet contain a REQUIRE_APPROVAL rule
       for the demo claim shape.
       The resume flow surface (sidecar + SDK) is still wired and exercised
       individually by unit tests in PR #37/#38/#39.
```

**What this proves (positive)**:
- Demo dispatch path: `run_approval_mode()` is invoked correctly (no fallback to default agent mode)
- SDK transit: handshake + `request_decision()` RPC against `LLM_CALL_PRE` trigger works
- Sidecar returns a valid `DecisionOutcome` with a decision_id

**What this does NOT prove (gap)**:
- End-to-end `REQUIRE_APPROVAL` → `ApprovalRequired` raise → operator approve → `e.resume(client)` → CONTINUE → LLM call
- Reason: the seed contract bundle shipped with the demo does not yet declare a REQUIRE_APPROVAL rule that matches the $500 claim shape
- The full resume-flow code paths (sidecar resume RPC, SDK `e.resume()`, control plane approve/deny) have unit-test coverage per the comment: "exercised individually by unit tests in PR #37/#38/#39"

**To upgrade to ✅ in a future iteration**:
1. Add a REQUIRE_APPROVAL rule to the demo's seeded contract bundle (e.g., `amount_atomic > 100_000_000` → REQUIRE_APPROVAL)
2. Have the demo trigger that path with an explicit claim
3. Auto-approve via the control plane gRPC API
4. Verify the resumed call completes

---

### Path 4 — DEGRADE ❌

**Mode**: not wired
**Observation**: no `DEMO_MODE=degrade` exists; no DEGRADE rule in the seeded contract bundle

**What exists in the product but not in the demo**:
- `spendguard.errors` ships `DegradeApplied` exception type (per integration adapters)
- Sidecar contract DSL supports `DEGRADE { mutate { ... } }` clauses per spec
- The SDK passes mutations through to the framework adapter

**To upgrade to ✅**:
1. Add a `DEGRADE` rule to the seed bundle (e.g., `model_family="gpt-4o" → mutate model_family="gpt-4o-mini"`)
2. Wire a new `agent_real_langchain_degrade` DEMO_MODE in `run_demo.py`
3. Verify the LangChain agent receives the mutated model in its response

---

## Performance Observation

- **Cold build** (every Rust service from scratch): ~30 min
- **Warm build** (post-F1 with cargo cache): ~3 min per affected service
- **Demo container startup → first sidecar handshake**: < 10 seconds (after services healthy)
- **End-to-end CONTINUE latency** (handshake → LangChain `ainvoke` returns): a few seconds, dominated by the real OpenAI API latency (~1-2s typical for `gpt-4o-mini`)

These are wall-clock observations from one run; not benchmark-quality. For benchmark-quality numbers see `benchmarks/runaway-loop/RESULTS.md` (which uses the shim, not this real stack).

---

## Known limitations of this V1 evidence

1. **Single-machine**: macOS / Apple Silicon Docker Desktop. Not tested on Linux x86_64 production target.
2. **No concurrent agents**: one demo container per run; the multi-tenant / fencing-lease paths aren't exercised here.
3. **No multi-step LangChain agent**: the demo `ainvoke` is a single LLM call. LangChain chains, agents with tool calls, retries — not exercised in this V1.
4. **No streaming**: the LangChain integration is tested with `ainvoke`, not `astream`.
5. **No real Anthropic via LangChain**: `agent_real_anthropic` uses the Anthropic SDK directly, not `ChatAnthropic` from langchain. A pure LangChain + Anthropic mode would need a new DEMO_MODE.
6. **Approval + Degrade gaps**: see Paths 3 and 4 above.
7. **TLS path coverage**: the real run does use mTLS between sidecar / ledger / canonical_ingest (the original F1 rustls panic proved that). But specific cipher / cert-rotation paths aren't exercised.

---

## Reproducibility

1. Clone the repo at `e198273` or later
2. Set `OPENAI_API_KEY`
3. Run the three `make demo-up DEMO_MODE=<mode>` commands above
4. For each mode, the demo container prints `[demo] <something> OK` or `NOTICE: ... PASS` if successful
5. Logs are available via `docker compose logs <service>` while containers are up
6. Bring down with `make demo-down` between runs to reset volumes

---

## Outstanding work (file in TODO)

- **F2 follow-up (long-term)**: write `verify-step7-real` with range assertions instead of equality, so `agent_real*` can run end-to-end verification not just dispatch
- **V1 Phase 3 follow-up**: add REQUIRE_APPROVAL rule to seed bundle + new DEMO_MODE for DEGRADE to close the matrix
- **V1 Phase 4 codex challenge**: per the V1 prompt, this doc should get a `/codex challenge` review from a LangChain-maintainer adversarial perspective; currently deferred to a separate session.

---

## Conclusion

After F1 (rustls CryptoProvider backport), F2 (verify-step7 guard for real-LLM modes), and F3a (test crypto provider init), the **real** Rust sidecar stack boots cleanly and runs end-to-end with a **real** LangChain agent calling **real** OpenAI gpt-4o-mini. CONTINUE and STOP decision paths are e2e verified. REQUIRE_APPROVAL has dispatch coverage but lacks a seeded contract rule to fire the full resume flow. DEGRADE has no demo wiring.

This is sufficient to open the P2-4 LangChain upstream docs PR honestly: the PR can claim "LangChain integration verified end-to-end against real OpenAI" without overstating REQUIRE_APPROVAL / DEGRADE coverage.

## Related

- `../../docs/SPENDGUARD_VIRAL_PLAYBOOK.md` — strategic plan
- `../../docs/SPENDGUARD_VIRAL_PLAYBOOK.todo.md` — open work tracker
- `../../docs/launches/v1-real-stack-e2e-prompt.md` — V1 session prompt
- `../../docs/launches/v1-phase1-bug-report.md` — F1 root-cause + fix history
- `../../docs/launches/p2-4-langchain-pr-prompt.md` — next-session prompt (now unblocked)
- `../runaway-loop/RESULTS.md` — three-way benchmark with shim/mock (different concern from this V1)
