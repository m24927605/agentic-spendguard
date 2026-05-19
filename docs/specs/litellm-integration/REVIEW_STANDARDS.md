# LiteLLM Integration — REVIEW_STANDARDS.md

> Status: Doc-first; lands before any implementation slice.
> Scope: Gating protocol for every slice of the LiteLLM ⇄ SpendGuard integration.
> Sibling docs: `DESIGN.md` (what we ship), `IMPLEMENTATION.md` (10-slice plan).
> Audience: Any implementer — human or another AI — picking up slice N.

---

## 1. Overview & rationale

Two facts from project memory shape this doc:

1. **Codex finds bugs static reading misses** (`feedback_codex_review.md`).
   Every Codex round on cost-advisor and egress-proxy surfaced P1/P2
   issues human reading had glossed over.
2. **Codex ✅ is necessary but not sufficient**
   (`feedback_demo_quality_gate.md`). Phase 2B Step 6 demo surfaced 14
   wire-time bugs that 14 Codex rounds had not caught (prost vs
   protobuf-python field-name drift, IPv4/IPv6 mismatch, Pydantic-AI
   duck-typing). Codex sees code, not the runtime.

Every slice must pass **adversarial Codex** *and* the demo. Neither alone
is the gate.

**Why ≤250-LOC slices.** `feedback_codex_iteration_pattern.md` shows that
10k+ LOC PRs produce open-ended Codex loops (each fix is new review
surface). A bounded slice gives a realistic 5-round cap. Reviewer fatigue
is the real constraint.

**What this replaces.** No separate pre-merge review. The combined gate
is per-slice Codex loop (§3) + demo gate (§7) + review log (§6). The
owner reads the log, not a PR thread.

---

## 2. Slice acceptance bar

A slice **passes** and the next slice begins only when all hard gates are
green. Soft gates produce GitHub issues but do not block.

### 2.1 Hard gates

| # | Gate | Verification |
|---|---|---|
| H1 | Diff ≤250 LOC additions (generated code marked + excluded). | `git diff --stat <base>..HEAD -- <slice-paths>` |
| H2 | All existing tests pass on slice branch. | `cargo test` / `pytest` |
| H3 | New tests cover the slice's new behaviour. | Inspected in review log §6.3 |
| H4 | Codex loop completed per §3 with stopping rule §3.4 met. | review-log §6.2 |
| H5 | Zero unresolved P0 findings (resolved = fixed in-slice OR linked issue with owner approval). | review-log §6.2 |
| H6 | Demo gate green per §7. | review-log §6.4 |
| H7 | Review log committed in same PR as slice code. | `ls docs/specs/litellm-integration/review-logs/slice-NN.md` |

### 2.2 Soft gates (do not block)

- **S1.** P1 out of slice scope → issue with `deferred-from-slice-N`
  label, linked from log.
- **S2.** P2/P3 cosmetic → aggregate in log §6.5; no issue unless
  repeated across slices.
- **S3.** Demo flake outside slice surface (e.g. ttl_sweep per
  `project_known_demo_flakes.md`) → cite memo, re-run.

### 2.3 What does NOT count as passing

- "Codex says LGTM" → see §3.4.
- "Tests pass and humans read it" → bypasses the Codex loop.
- "No demo mode exists yet" → pure-doc slices still re-run the adjacent
  `DEMO_MODE=decision` for regression.

---

## 3. Codex review loop

### 3.1 Round prep — what Codex sees

Per round, the implementer hands Codex (via `codex:rescue` subagent):

1. The slice diff (not full files — Codex behaves better on diffs).
2. The slice's mini-spec paragraph from `IMPLEMENTATION.md`.
3. The relevant `DESIGN.md` section.
4. For round N>1: prior round findings with status (fixed-here /
   deferred-issue-#NN / disputed-because-...).

### 3.2 Severity model

Mirrors existing project commits (`codex P1.5 r2`):

| Severity | Meaning | Action |
|---|---|---|
| **P0** | Hard correctness/safety bug. Audit-chain break, double-spend, auth bypass, ledger invariant violated, data loss. | **Must fix in slice.** Cannot be deferred without owner approval. |
| **P1** | Important defect with workaround. Wrong typed exception, fail-open when fail-closed expected, off-by-one in TTL. | Fix in slice OR issue + explicit defer note. |
| **P2** | Code/test quality. Mocking behaviour under test, opaque error, missing edge-case test. | Fix in slice if local; otherwise issue + defer. |
| **P3** / note | Cosmetic. Doc nit, micro-rename. | Aggregate in §6.5; no issue unless repeated. |

Codex findings without an explicit severity get the implementer's
provisional severity in the log, with one-line justification.

### 3.3 Inter-round actions — what counts as "addressed"

A finding is **addressed** only if one of:

- **fixed-here**: next round's diff contains the fix AND a new/updated
  test exercises it ("tested manually" is not acceptable).
- **deferred-issue-#NN**: GitHub issue open per §5, linked from log.
- **disputed-because-...**: implementer writes a reasoned counter-argument
  in the log; it re-enters the next prompt verbatim so Codex can confirm
  or push back. **P0 cannot be disputed** — fix or escalate.

Silently dropping a finding between rounds is a protocol violation. Every
finding's fate is in the log.

### 3.4 Stopping rule

The loop stops, and the slice passes review, when round N satisfies:

> **(A)** Every P0 from rounds 1..N is `fixed-here` or `deferred-issue-#NN`.
> No P0 can be `disputed-because-...`.
>
> **AND (A')** Every **critical-path P1** from rounds 1..N is either
> `fixed-here`, `deferred-issue-#NN`, or `disputed-because-...` with
> explicit owner acceptance. Critical path = slice's exported API,
> sidecar wire protocol, ledger SP boundary, audit-chain emission
> path. (Round 3 P1.4 fix — earlier wording only required round-N to
> not introduce new P1s, but allowed earlier-round critical P1s to
> hang unaddressed.)
>
> **AND (B)** Round N introduces **zero new P0** and **zero new P1 in
> critical path**.
>
> **AND (C)** N ≥ 2. Round 1 alone never stops the loop, even on zero
> findings — a confirmation round is always required.

This replaces "until 👍" per `feedback_codex_iteration_pattern.md`. On a
≤250-LOC slice, condition (B) typically holds at round 2 or 3.

**Worked example.** R1: 2P0/4P1/3P2 → fail (A). R2: P0 fixed, 1 new P1 in
exported API → fail (B). R3: P1 fixed, 1 P3 → **pass** (A holds, B holds,
N=3).

### 3.5 Maximum-rounds escape hatch

If round 5 closes with the rule unmet, the slice **fails**. Implementer
chooses:

1. **Split the slice.** Running over 5 rounds is evidence the slice is
   doing too much. Split along smallest cohesive sub-feature; restart at
   round 1 on each half. Document in IMPLEMENTATION.md.
2. **Defer the slice.** Open issues for open findings; close branch; move
   to next independent slice. Only safe when slice is non-blocking.
3. **Escalate to owner** with: round-by-round table, the threatened
   invariant, and a concrete recommendation. "Halp" is not escalation.

Going 6+ rounds without escalating is itself a P0 protocol violation.

---

## 4. Codex prompt templates

### 4.1 Round 1 (new slice)

```
You are reviewing slice {N} of the LiteLLM ⇄ SpendGuard integration.

Mode: ADVERSARIAL. Your job is to BREAK this slice. Look for:
- Audit-chain invariants violated (canonical_events hash-chain breaks,
  missing emission on error paths, double-emission)
- Single-writer-per-budget ledger invariant violated
- Reserve/commit/release lifecycle errors (leaked reservations,
  double-commit, commit after release)
- Fail-closed inversions (sidecar unreachable should DENY by default
  per DESIGN §5)
- Typed-exception drift (DecisionDenied / SidecarUnavailable
  / SpendGuardConfigError per DESIGN §5 — P0.8 spec lock: typed-deny
  exception is `DecisionDenied` everywhere)
- Idempotency-key derivation that lets retries double-charge
- Streaming claim leaks (TTL too short/long, wrong reserved amount)
- Wire-protocol assumptions Codex cannot verify (tag P2 with
  "demo-required-to-confirm")
- ≤250 LOC slice budget; flag if exceeded
- Mocks that mock out the behaviour under test
- New abstractions in a slice with no user

Severity scale (use exactly): P0 (must fix in slice) / P1 (defect with
workaround) / P2 (code-quality) / P3 (cosmetic).

Slice mini-spec: {paste IMPLEMENTATION.md §slice-N}
Relevant DESIGN section: {paste DESIGN.md §X}
Slice diff: {paste git diff <base>..HEAD -- <slice-paths>}

Per finding output: Severity / file:line / What is wrong / Why
(invariant violated) / Concrete proposed fix.

End with per-severity counts and one-sentence overall risk.
```

### 4.2 Round N>1 (follow-up)

```
You are reviewing slice {N} round {R}. Prior round findings table:
{paste round R-1 table with status: fixed-here / deferred-issue-#NN /
disputed-because-...}

Counter-arguments for disputed findings (verbatim):
{paste, or "none"}

Updated diff vs prior round: {paste git diff <prev-round-sha>..HEAD}

Tasks:
1. Verify each fixed-here is actually fixed AND did not regress
   anything. Flag fix-regressions as P0/P1.
2. Confirm or push back on disputes; if pushing back, restate the
   invariant and explain why the dispute does not resolve it.
3. Fresh adversarial pass (§4.1 checklist) on this round's new diff.
   New findings tagged "new-in-r{R}".

Apply stopping-rule check at end:
- Any unresolved P0?
- Any new P0 this round?
- Any new P1 in critical path (exported API / sidecar wire / ledger SP /
  audit-chain emission)?

If all three are NO, state: "STOPPING-RULE-MET (N={R})."
Otherwise: "STOPPING-RULE-NOT-MET — {which condition failed}."
```

### 4.3 Disputed finding (in log + next prompt)

```
Codex finding (verbatim, r{R-1}, P{X}):
> {paste}

Counter-argument: The finding asserts {restated claim}. This is incorrect
because {specific reason citing invariant / DESIGN section / test}. Code
at {file:line} {does the correct thing because ...}. No change.

Evidence: {test name | DESIGN ref | RFC section}
```

---

## 5. Residual issue tracking

Per `feedback_codex_iteration_pattern.md`: residuals are GitHub issues,
not gap-bullets that decay after merge.

### 5.1 Title format

```
[litellm-integration] codex r{R} P{sev}: {summary} (deferred from slice {N})
```

Example: `[litellm-integration] codex r2 P1: streaming TTL leaks reservation on client disconnect (deferred from slice 4)`

### 5.2 Required labels

- `area:litellm-integration`
- `kind:deferred-codex`
- `slice:NN` (two-digit, e.g. `slice:02`)
- `severity:P0` | `severity:P1` | `severity:P2`

**P0 special-case:** cannot be deferred without owner approval. Issue
body must record approver + date.

### 5.3 Issue body

```markdown
## Codex finding
{verbatim, including file:line}

## Why deferred from slice {N}
{out of scope / multi-slice refactor / blocked on external dep / etc.}

## Acceptance criteria for closure
- [ ] {concrete test or behaviour proving the fix}
- [ ] {regression test added at sdk/.../tests/...}

## Link-back
- Slice review log: `docs/specs/litellm-integration/review-logs/slice-{NN}.md` (round {R})
- DESIGN: §{X}
```

### 5.4 Link-back from review log

Every deferred finding cites the issue URL:

```
- [P1] r2: streaming TTL leak — deferred to #NNNN
```

No URL → not deferred → fix in slice or escalate.

---

## 6. Per-slice review log file

### 6.1 Path

```
docs/specs/litellm-integration/review-logs/slice-NN.md
```

`NN` zero-padded. Committed in the **same PR** as the slice code (H7).

### 6.2 Schema

```markdown
# Slice {NN} review log

- Scope: {one-line, mirrors IMPLEMENTATION.md}
- Base commit: {sha} → Head commit: {sha}
- LOC delta: {+X / -Y, generated excluded}
- DESIGN sections implemented: §A.B, §C

## Round summary
| Round | Date | New P0 | New P1 | New P2 | New P3 | Fixed-here | Disputed | Result |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-MM-DD | … | … | … | … | — | … | not-met |
| 2 | 2026-MM-DD | … | … | … | … | … | … | **STOPPING-RULE-MET** |

## Findings (chronological)
### Round 1
- [P0] file:line — {summary} → fixed-here in commit {sha}; test: `tests/...::test_name`
- [P1] file:line — {summary} → deferred to #NNNN
### Round 2
- [P1] file:line — {summary} (new-in-r2) → fixed-here in {sha}

## Disputed findings
(empty if none)

## Deferred-cosmetic aggregation
- Naming nit in `litellm.py:42` — bundle into post-v1 polish

## Demo gate
(see §7.4)

## Sign-off
- Stopping rule met at round {N}
- H1–H7 all green
- Implementer: {handle}
- Date: 2026-MM-DD
```

### 6.3 Test-coverage evidence

Each `fixed-here` for P0/P1 must cite the test that exercises the fix
(file + test name). Example:

```
- [P0] sdk/.../litellm.py:88 — reserve before handshake → fixed-here in a1b2c3d;
  regression test: `sdk/python/tests/integration/test_litellm_callback.py::test_handshake_before_reserve`
```

### 6.4 Demo evidence

In the `## Demo gate` section (template in §7.4).

---

## 7. Demo gate

Per `feedback_demo_quality_gate.md`: Codex cannot catch cross-language
wire issues, OS-layer (IPv4/IPv6), package drift, or class-vs-duck-typing.
Demo is the wire-time check.

### 7.1 Required demo per slice kind

| Slice kind | DEMO_MODE |
|---|---|
| Doc/scaffolding (no runtime change) | `decision` (regression only) |
| SDK callback module | `decision` until the litellm mode exists, then `litellm_callback_minimal` |
| LiteLLM proxy integration | `litellm_real` (DESIGN G5 target) |
| Demo-only slice | the mode it introduces |

### 7.2 Exact commands

```
cd deploy/demo
make demo-down                             # clean state
DEMO_MODE={mode} make demo-up              # bring up + run
DEMO_MODE={mode} make demo-logs > logs.txt # capture
make demo-down                             # tear down
```

`logs.txt` is not committed (large + timestamped); excerpts go into §7.4.

### 7.3 Expected output snippets

**`DEMO_MODE=litellm_real`** (acceptance target; ACCEPTANCE.md §5.1 is
authoritative — this section mirrors it verbatim). Stdout must contain
**all four** step lines in this order:

```
[demo] DEMO_MODE=litellm_real → litellm proxy + sidecar + ledger ready
[demo] handshake ok session_id=...
[demo] step 1: ALLOW — litellm.acompletion → DECISION_ALLOWED → INVOICE_COMMITTED
[demo] step 2: DENY — over-budget → DecisionDenied raised
[demo] step 3: STREAM — sse complete → INVOICE_COMMITTED with real usage
[demo] step 4: PROXY — POST /v1/chat/completions team=t1 → INVOICE_COMMITTED
[demo] PASS — all 4 steps OK
```

**Partial-completion note.** The demo is built incrementally across
slices. Until Slice 9 lands, the final line and step 3/4 lines are
absent. During Slice 6 review (steps 1+2 only), the expected partial
trace is steps 1+2 + a placeholder line `[demo] WIP — steps 3+4
deferred to slice 9`. During Slice 9 review, ALL four step lines + the
`PASS — all 4 steps OK` line MUST land.

**`DEMO_MODE=litellm_deny`** (3 fail-closed sub-steps, ACCEPTANCE.md
§5.2 authoritative):

```
[demo] DEMO_MODE=litellm_deny → fail-closed scenarios
[demo] handshake ok session_id=...
[demo] step 1: budget exhausted — DecisionDenied raised (provider untouched)
[demo] step 2: sidecar offline — SidecarUnavailable raised (provider untouched)
[demo] step 3: resolver returns None + no default budget — SpendGuardConfigError raised
[demo] PASS — all 3 deny paths OK
```

For `DEMO_MODE=decision` regression (pure-doc slices): the existing
`[demo] PASS — handshake + decision + confirm OK` tail.

Absent the literal `PASS — all N steps/paths OK` line for the
slice-applicable mode → demo gate **fails** regardless of exit code.

### 7.4 Review-log demo section

```markdown
## Demo gate
- Mode run: DEMO_MODE={mode}
- Date: 2026-MM-DD
- Result: PASS | FAIL | FLAKE-RETRIED
- Log tail (last 30 lines):
  ```
  {paste}
  ```
- Hash of logs.txt: {sha256}  # local artifact, not committed
```

### 7.5 Demo flake handling

1. **Known flake outside slice surface** (cite `project_known_demo_flakes.md`)
   → mark `FLAKE-RETRIED`; pass only if re-run is PASS.
2. **Flake on slice surface** → treat as new P0 in next Codex round; demo
   gate fails.
3. **Demo never reaches the test steps** (boot/health fail) → P0
   environmental bug, likely a wire/version issue exactly of the kind
   `feedback_demo_quality_gate.md` warns about. Fix and re-run from
   clean state.

---

## 8. Code review style guidance (human reviewer)

Posture from `feedback_working_principles.md`:

- **Rule 2 (短 reply, long doc).** PR description references the log;
  no duplicated findings inline.
- **Rule 3 (spec lock = no redesign).** A slice does not silently modify
  DESIGN.md. Redesign triggers: chaos-test breaks invariants, new
  high-irreversibility gap, production data reveals critical flaw.
  Otherwise: fix in slice within locked design, or defer.
- **Rule 5 (review discipline > role).** Pre-write and post-write Codex
  rounds; two rounds on irreversible decisions.

Owner check order: (1) log schema complete; (2) H1–H7 green; (3)
stopping rule §3.4 demonstrably met (not "Codex was happy"); (4) demo
gate §7.4 has `PASS` + log hash; (5) spot-check one fixed-here — does
the cited test fail before the fix?

Comment markers (per `engineering-code-reviewer.md`): 🔴 blocker,
🟡 suggestion, 💭 nit.

---

## 9. Anti-patterns that fail review

1. **Silent `except Exception: pass` in the callback path.** Fail-closed
   by contract (DESIGN §5). P0.
2. **Mocking the behaviour under test.** Mocking `SpendGuardClient` in a
   test meant to verify the callback's reserve/commit lifecycle defeats
   the test. Mock the upstream provider, not the SUT boundary.
3. **New abstraction in a slice with no user.** Speculative
   generalisation. Ship the second user in the same slice or skip the
   abstraction.
4. **Going over ≤250 LOC.** Split. The cap is what makes the 5-round
   bound realistic.
5. **Inventing a new SDK exception class.** The three per DESIGN §5 are
   the contract; new types need a DESIGN revision, not slice-local
   addition.
6. **Reserving on every retry with the SAME `decision_id`.** If two
   retries of the same logical call use the **same** `decision_id`,
   the sidecar's idempotency dedupes them — that's correct behaviour
   (Pydantic-AI internal retry pattern). The anti-pattern is the
   **opposite**: minting a fresh `decision_id` for the SAME logical
   call (e.g. random UUID per attempt without basing it on
   `litellm_call_id`) causes the ledger to double-reserve. The
   integration handles this correctly because LiteLLM mints a fresh
   `litellm_call_id` PER ATTEMPT (so distinct attempts deliberately
   have distinct decision_ids), and `decision_id` is deterministic
   from `litellm_call_id` via `derive_uuid_from_signature(
   f"litellm:{litellm_call_id}", scope="decision_id")` (DESIGN.md §5
   retry row, ADR-002, P0.9 clarification from Phase 0 review). P0
   only if the implementation does NOT use the deterministic
   derivation.
7. **Skipping the demo on a "small" change.** Slices touching sidecar
   wire / exception types / callback registration / proxy config must
   run the demo. Pure-doc slices run `DEMO_MODE=decision` regression.
8. **Disputing a P0.** §3.3 forbids; fix or escalate with owner approval.
9. **`# TODO` for residuals.** Use issues (§5).
10. **Stale demo log.** §7.4 log must reflect the final code; demo
    timestamps must postdate the final commit.
11. **Re-opening DESIGN.md mid-slice.** Escalate per §3.5; no silent
    revision.
12. **Skipping rounds 2+ on a clean round 1.** §3.4 (C) requires N≥2.

---

## 10. References

- `feedback_codex_review.md` — adversarial mode, 100% adoption, two
  rounds on irreversible decisions.
- `feedback_codex_iteration_pattern.md` — explicit stopping rule
  up-front, residuals as issues, reviewer fatigue is the constraint.
- `feedback_demo_quality_gate.md` — Codex ✅ necessary not sufficient;
  demo is the wire-time check.
- `feedback_working_principles.md` — short reply / long doc; spec lock;
  review discipline > role.
- `DESIGN.md` — what ships; source of every invariant this protocol
  enforces.
- `IMPLEMENTATION.md` (10-slice plan) — slice-by-slice plan feeding §3.1 / §4.1.
- `engineering-code-reviewer.md` — 🔴/🟡/💭 markers for human PR
  comments.
