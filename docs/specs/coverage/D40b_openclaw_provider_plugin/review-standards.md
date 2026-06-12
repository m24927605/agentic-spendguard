# D40b - Review Standards

Use with a Codex sub-agent for every D40b slice.

2026-06-12 user-directed reviewer override: the formal R1-R5 adversarial
review gate for D40/D41 coverage work is an independent Codex sub-agent
reviewer. Do not route reviews through an external Codex service, local
Ollama model, or Claude Code `superpowers:code-reviewer`.

## 1. Precedence (P0)

The LOCKED `design.md` trumps slice docs. OpenClaw API corrections require a dated append-only amendment to `design.md`; do not bury them in implementation comments.

## 2. Fail-closed enforcement (P0)

| Check | Pass condition |
|---|---|
| 2.1 | No catch-and-continue around reserve. |
| 2.2 | DENY and sidecar outage both prevent upstream provider invocation. |
| 2.3 | No `failOpen`, `degradeOnUnavailable`, `SPENDGUARD_DISABLE`, or equivalent bypass in adapter code/docs. |
| 2.4 | Demo proves counting-stub unchanged on DENY. |

## 3. Reserve/commit tuple integrity (P0)

| Check | Pass condition |
|---|---|
| 3.1 | `unitId`, `windowInstanceId`, and `pricing` are required options from day 1. |
| 3.2 | Reserve claims carry those values. |
| 3.3 | Commit reuses reserve-time unit/pricing tuple. |
| 3.4 | `estimatedAmountAtomic` is positive; never zero. |

## 4. OpenClaw API pinning (P0)

| Check | Pass condition |
|---|---|
| 4.1 | Every `OB-V*` marker touched by the slice is pinned with version/source evidence. |
| 4.2 | The wrapper point is before upstream dispatch, not post-observation. |
| 4.3 | Streaming terminal event and provider-error shape are tested against the pinned API. |

## 5. Hash and bundle hygiene (P1)

| Check | Pass condition |
|---|---|
| 5.1 | IDs and idempotency keys delegate to `@spendguard/sdk`. |
| 5.2 | No local hash library or crypto import. |
| 5.3 | Bundle <= 50 KB minified excluding peers. |

## 6. Reviewer prompt template

```text
You are the adversarial code reviewer for slice <SLICE_ID> (round R<N>) of
D40b - OpenClaw provider plugin adapter.

Reviewer tool: Codex sub-agent, per the 2026-06-12 user-directed override above.

Read in order:
1. docs/specs/coverage/D40b_openclaw_provider_plugin/design.md
2. docs/specs/coverage/D40b_openclaw_provider_plugin/review-standards.md
3. docs/slices/<SLICE_ID>.md
4. The diff under review: <DIFF_REF>

Apply review-standards §1-§5. Block on fail-open behavior, unpinned OpenClaw
API assumptions, tuple mismatch, local hashing, or fake demo evidence. Verify
that D40a remains a separate base-URL fallback and that D40b docs carry the
in-process trust-boundary warning.

Output numbered findings with severity, file:line, evidence, and spec ref.
End with exactly one verdict line:
VERDICT: PASS
or
VERDICT: FAIL (<b> blockers, <m> majors)
```
