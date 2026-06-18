# D41 session reservation substrate - Review Standards

Use with a Codex sub-agent. This is substrate work; P0 findings block.

2026-06-12 user-directed reviewer override: the formal R1-R5 adversarial
review gate for D40/D41 coverage work is an independent Codex sub-agent
reviewer. Do not route reviews through an external Codex service, local
Ollama model, or Claude Code `superpowers:code-reviewer`.

## 1. Precedence (P0)

`design.md` is LOCKED. Any change to lifecycle, API names, ledger semantics, or failure behavior requires a dated append-only amendment.

## 2. Ledger invariants (P0)

| Check | Pass condition |
|---|---|
| 2.1 | Committed amount never exceeds reserved amount. |
| 2.2 | Release settles only uncommitted remainder. |
| 2.3 | Reserve, commit delta, and release are idempotent with conflict detection. |
| 2.4 | Commit delta must be positive; zero is rejected. |
| 2.5 | Unit/window/pricing tuple cannot drift between reserve and commit. |

## 3. Failure posture (P0)

| Check | Pass condition |
|---|---|
| 3.1 | Session reserve sidecar outage or DENY fails closed before paid provider connection. |
| 3.2 | Commit failure does not silently continue unbounded. Further provider turns stop or bounded retry/reconnect path applies. |
| 3.3 | Crash/abandon path has TTL release. |
| 3.4 | Reconnect replay is idempotent and bounded. |

## 4. Audit chain (P0)

| Check | Pass condition |
|---|---|
| 4.1 | New session events are signed CloudEvents. |
| 4.2 | Canonical ingest can consume session events. |
| 4.3 | Denied and expired sessions are visible in audit, not silent. |

## 5. Backward compatibility (P1)

| Check | Pass condition |
|---|---|
| 5.1 | Existing request-scoped RPCs and SDK methods are unchanged. |
| 5.2 | Existing adapter tests still pass. |
| 5.3 | No frozen corpus is edited in place. |

## 6. Reviewer prompt template

```text
You are the adversarial code reviewer for slice <SLICE_ID> (round R<N>) of
D41 session reservation substrate.

Reviewer tool: Codex sub-agent, per the 2026-06-12 user-directed override above.

Read in order:
1. docs/specs/coverage/D41_session_reservation_substrate/design.md
2. docs/specs/coverage/D41_session_reservation_substrate/review-standards.md
3. docs/internal/slices/<SLICE_ID>.md
4. The diff under review: <DIFF_REF>

Block on violations of ledger invariants, idempotency, fail-closed reserve,
tuple matching, audit-chain visibility, or backward compatibility. Verify every
SR-V marker touched by this slice is pinned with code/version evidence.

Output numbered findings with severity, file:line, evidence, and spec ref.
End with exactly one verdict line:
VERDICT: PASS
or
VERDICT: FAIL (<b> blockers, <m> majors)
```
