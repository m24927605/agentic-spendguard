# Staff Escalation r5 — Auto-Instrument Egress Proxy spec, ID-derivation root cause

Per `auto-instrument-egress-proxy-spec.md` v5 §14.1, codex r5 RED triggered Staff escalation. 4 parallel sub-agents reviewed the contested root cause and returned in ~5 minutes.

## Original contested finding (codex r5 P1-r5.1 + P1-r5.2 + P2-r5.A)

v5 spec text claimed:
- `signature = sha256(canonicalized_body)[..16]`
- `derive_uuid_from_signature` returns "UUID v5"
- canonicalization = "sort JSON keys, strip whitespace"
- `step_id = f"{run_id}:proxy-call:{signature}"`

But actual production code:
- All 3 SDKs use **blake2b**, not sha256 (langchain.py:149, openai_agents.py:125, ids.py:131,167)
- `derive_uuid_from_signature` is **blake2b(scope|signature, 16) masked to UUIDv4 shape** — NOT RFC 4122 v5
- 3 SDKs use 3 different canonicalization approaches; no unified spec
- step_id discriminator `:proxy-call:` decouples proxy from wrapper-mode cost_advisor grouping

## Staff opinions (verbatim summary, full text on disk)

### Staff #1 — distributed systems angle
**Verdict (a) accept + rewrite.** Cross-mode (SDK vs proxy) ID-space convergence is the load-bearing property motivating the v5 fix. blake2b-16, RFC 8785 JCS canonicalization, ported to shared Rust crate with cross-language fixture tests.

### Staff #2 — security angle
**Verdict (c) split.** Accept canonicalization + naming fix; reject "leaks bits" as overstated for v0.1 same-machine trust boundary. Use blake2b (not FIPS but matches SDKs and content-addressing-not-crypto). Audit-outbox oracle attack on step_id IS real but defer HMAC mitigation to v0.2. Rename helper to drop misleading "UUID v5" label.

### Staff #3 — infrastructure angle
**Verdict (a) new shared crate `services/ids/`.** Pattern matches existing `spendguard-signing` / `spendguard-policy`. Slice 2 does the port (not slice 4b — removes critical-path blocker). blake2b-128. Cross-language byte-equivalence fixture test is the load-bearing contract. Owner = same team as Python SDK ids.py. Operability fix: explicit error (no silent `repr()` fallback).

### Staff #4 — ledger / audit-invariant angle
**Verdict (b) unified discriminator `:call:`, NOT `:proxy-call:`.** Transport-distinguished step_id breaks cost_advisor rules for mixed/migrating deployments — same logical agent splits into 2 half-populated buckets. Transport visibility belongs in CloudEvent `source` / `producer_id` (already distinct: `egress-proxy://...` vs `sidecar://...`), NOT in step_id string. No schema changes needed.

## Synthesis (Claude's read, 3-of-4 majority on each axis)

| Axis | Consensus |
|---|---|
| Hash function | **blake2b-128** (4-of-4) |
| UUID flavor | **v4-shape (blake2b-masked)**, NOT RFC 4122 v5 (3-of-4 explicit) |
| Canonicalization | **`serde_json` deterministic sorted-keys** (3-of-4); JCS is aspirational but `BTreeMap` ordering is the practical v0.1 floor |
| Port location | **NEW shared crate `services/ids/`** (2 explicit + 2 implicit) |
| Step_id discriminator | **Unified `:call:`** (matches `pydantic_ai.py:439`); transport visibility in CloudEvent `source` |
| Port lands in slice | **Slice 2 (was Slice 4b)** — leaf crate; unblocks critical path |
| Additional | Cross-language byte-equiv fixture tests; explicit serialize error (no `repr()` fallback); §8 audit-oracle row deferred to v0.2 HMAC; §11 blake2b-not-FIPS note |

## Decision

Take all 4 Staff recommendations modulo merge:

1. **Hash**: blake2b-128 everywhere — replace `sha256` in spec §4.1 step 5, §4.1.5, §7.
2. **UUID helper naming**: rename "UUID v5" → "deterministic UUIDv4-shape (blake2b-masked)" in spec + add to slice 2 acceptance.
3. **Canonicalization**: `serde_json::to_vec` with sorted-key wrapper (BTreeMap). JCS is documented as v0.2 hardening goal; not blocking v0.1.
4. **Port location**: new shared crate `services/ids/` (Cargo.toml mirrors `services/policy/`). Lands in **Slice 2** with cross-language fixture tests committed to `services/ids/tests/fixtures/`.
5. **Step_id discriminator**: **`:call:`** (drop `:proxy-call:`). Transport visibility via CloudEvent `source` already differentiates `egress-proxy://...` from `sidecar://...`. cost_advisor agent grouping converges across modes.
6. **Operability**: Rust port returns typed `Err(IdsError::Unserializable)` instead of Python's `repr()` fallback. Counter: `egress_proxy_unserializable_total`.
7. **Security followup tracked**: audit-outbox oracle attack on `step_id[:16]` → deferred §13.11 (HMAC salt v0.2).
8. **FIPS note**: §11 + slice 8 README acknowledge blake2b non-FIPS; offer `--hash-algo=sha256` build flag as future option.

## Codex r6 expectations

This decision resolves codex r5's 2 P1 + 1 P2-critical:
- P1-r5.1 (sha256→blake2b mismatch): fixed in §4.1 + §4.1.5
- P1-r5.2 (UUIDv5 mis-label): fixed via naming change
- P2-r5.A (canonicalization undefined): fixed via `serde_json` sorted-keys spec + JCS-as-future

Spec slice ordering shifts: ids crate moves to Slice 2 (5 LOC of acceptance), reducing Slice 4b from "largest slice" risk flagged in codex r2 P2-r2.E.

## Authority

Decision-maker: Claude (3-of-4 majority threshold met per §14.1 Step 3). No user tiebreaker needed.

Apply v6 patch; run codex r6. If r6 GREEN: merge slice 1; proceed to slice 2.

If r6 RED: per §14.1, slice 1 remains under Staff deliberation (r7) but slices 2-11 may proceed referencing v6 spec.
