# D13 — Review Standards

Slice-specific checklist for the `superpowers:code-reviewer` skill across `COV_60` … `COV_66`. Each slice review consults this file plus [`acceptance.md`](acceptance.md) plus the repo-wide coding standards.

## 1. Threat-model assertions

D13 introduces a meter that **cannot** enforce, parses OAuth bearer tokens at the proxy edge, can return synthetic 429s (denial-of-service against the customer's own CLI workflow), and adds a new sidecar branch that can incorrectly skip a ledger write. Any diff touching `services/egress_proxy/src/subscription{,_meter,_cap_store}.rs`, `services/sidecar/src/decision/transaction.rs`, or the three migrations MUST be reviewed against these assertions; reviewer flags as Blocker if any fails.

| ID | Assertion |
|----|-----------|
| `T1` | Authorization-token parsing never crosses 13 chars into the token body. Every code path that touches `Bearer …` MUST go through `extract_auth_token_prefix` (or an equivalent constant-length extractor). Reviewer greps the diff for `.to_string()` on the raw auth string and rejects. |
| `T2` | No `tracing::*!()` macro call in the diff includes the full Authorization value or any field derived from it beyond the 13-char prefix. Reviewer greps for `auth\|authorization\|bearer` near logging macros and reads each match. |
| `T3` | Classifier defaults to `Byok` on ANY ambiguity (missing header, malformed scheme, prefix mismatch, UA forgery). Subscription detection is opt-in conservative — false negatives are acceptable, false positives are not (they would skip the BYOK ledger and accumulate a real-money debt the operator never sees). |
| `T4` | Sidecar branch `if request.reservation_source == SUBSCRIPTION_METER` does NOT trust the field for downstream tenant identity — `tenant_id` still comes from the authenticated sidecar UDS peer. The reservation-source field only controls write-path forking. |
| `T5` | Hard-cap synthetic 429 response body MUST NOT contain other tenants' usage figures or any cross-tenant identifier. The `message` field is a fixed string; numeric fields come from the requesting tenant's own cap row only. |
| `T6` | `subscription_caps` table has RLS enabled with policy `subscription_caps_tenant_isolation`. No diff may grant `sidecar_runtime` BYPASS RLS. |
| `T7` | Fixture HARs MUST contain only redacted sentinels. Reviewer greps every committed `.har` file for `sk-ant-oat01-[A-Za-z0-9_-]{20,}`, `eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}`, and `sk-(proj-)?[A-Za-z0-9_-]{40,}` — any non-`FAKE_`-prefixed match is a Blocker. |
| `T8` | `PROVENANCE.md` pins the SHA-256 of the redaction script. Reviewer recomputes `sha256sum scripts/redact_har.py` and confirms it matches. |
| `T9` | Soft-cap Slack/PagerDuty payloads MUST NOT include the inbound Authorization value, the inbound request body, or any prompt content. The unit test `soft_cap_slack_payload_redacts_oauth_token` is the enforcement; reviewer reads the assertion. |
| `T10` | The `meter_only_estimate` codepath MUST NOT call `sidecar.RequestDecision`, MUST NOT acquire a ledger lock, and MUST NOT write to `reservations`. Verified by mock-panicking test (`meter_estimate_never_calls_sidecar`). |
| `T11` | Migration 0044 default-value backfill is purely `'byok'`. No migration row may default to `'subscription_meter'` (would mis-tag historical BYOK rows as subscription and break dashboards). |
| `T12` | Hard-cap `Retry-After` value is bounded — must be `min(seconds_until_window_reset, MAX_RETRY_AFTER)` where `MAX_RETRY_AFTER = 86400`. Prevents the CLI from being asked to wait > 24h on a misconfigured window. |
| `T13` | The synthetic 429 body's `error.type = "rate_limit_exceeded"` and `error.code = "spendguard_subscription_cap"` — distinct from any vendor rate-limit code so a downstream log analyser can tell SpendGuard-injected from vendor-injected 429s. Reviewer verifies the constant strings. |
| `T14` | Importer crates with `live` feature flag must remain `live`-OFF by default. `cargo tree` output (per acceptance `A2.5` / `A10.3`) must show no `reqwest`, `hyper-tls`, or HTTP-client dep in default-features build. |

## 2. Cross-tier correctness assertions

`COV_61` (proto + migration), `COV_62` (estimate path), and `COV_64` (hard-cap path) cross multiple subsystems. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `X1` | `ReservationSource` proto enum is **additive** with `RESERVATION_SOURCE_UNSPECIFIED = 0` so existing clients without the field default safely to sidecar's BYOK ledger path. Reviewer rejects any non-zero default. |
| `X2` | Proto enum value names follow `RESERVATION_SOURCE_*` prefix convention (Google proto style; matches existing `RUN_STATE_*` etc. in `common.proto`). |
| `X3` | The sidecar branch on `reservation_source` happens **after** sidecar authenticates the UDS peer and resolves `tenant_id`. No reservation_source check happens before `auth::resolve_peer`. |
| `X4` | `pricing_table` lookup in `meter_only_estimate` uses **retail** prices (the existing public columns), not any volume-discounted operator-private columns. The strategy memo defines meter mode as "best-effort estimate via the public retail table" — operator-specific overrides would mislead the dashboard. |
| `X5` | The Codex / ChatGPT-OAuth routing-table row is **appended** to `ROUTING_TABLE` text-order after the existing OpenAI responses row. Regex is restrictive enough that order doesn't matter, but textual ordering eases code review. |
| `X6` | `audit_outbox.reservation_source` column ordering in the SELECT projections in downstream services (analytics, control plane forwarder) updated to include the new column. Reviewer greps every `SELECT … FROM audit_outbox` for missing column. |
| `X7` | Both demo verifier SQL files use the same `audit_outbox` schema columns that the existing `verify_step_litellm_*.sql` targets use — no schema drift. |
| `X8` | Hard-cap window-reset math is UTC-based. `chrono::Utc::now()` not `chrono::Local::now()`. Reviewer greps the diff for `chrono::Local`. |

## 3. Classifier correctness matrix

`COV_60` modifies `subscription.rs`. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `M1` | Every classification arm requires BOTH an Authorization-prefix match AND a User-Agent match. No single-signal arm exists. |
| `M2` | Classifier is **pure** (no IO, no global mutable state). Verified by signature: `fn classify(cfg: &…, headers: &…, body: &…) -> SubscriptionKind`, all args borrowed. |
| `M3` | `extract_auth_token_prefix` returns at most 13 chars. Reviewer reads the implementation and confirms. |
| `M4` | The Codex JWT sniff (`starts_with("eyJ")`) is documented as a heuristic, not a parse. Reviewer confirms a comment explains "we deliberately do not parse JWTs to avoid jwt-crate dep + side-channel risk". |
| `M5` | `User-Agent` allow-list is anchored to `<tool>/<version>` prefix, not substring contains. `claude-cli/1.4.0` matches; an attacker-controlled UA like `evil; claude-cli/x.y` does NOT match. |
| `M6` | Unknown ProviderKind (e.g. Vertex, Azure) falls through to `Byok`. Subscription detection is not extended to vendors that don't have subscription-tier CLIs. |
| `M7` | The classifier is called **exactly once** per inbound request — verified by integration test that wraps it in `Arc<AtomicUsize>` and asserts count == 1 across the proxy + sidecar path. |

## 4. Meter-vs-ledger fork assertions

`COV_61` + `COV_62` introduce the fork between the meter-only audit row and the BYOK reserve path. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `F1` | The fork happens at **one** call site in `forward.rs::handle_request`. Reviewer rejects diffs that introduce multiple fork sites (rationale: single-fork keeps the proof obligation small). |
| `F2` | For `SubscriptionKind::Byok`, the code path is **byte-identical** to the pre-D13 path. Reviewer diffs the BYOK branch against pre-D13 `forward.rs` and confirms no behaviour change. |
| `F3` | For subscription kinds, the path does NOT instantiate a `SidecarClient::request_decision` request — no sidecar gRPC call at all. The audit row is written via a direct PG insert from the proxy (no sidecar in the loop) OR via a new sidecar method `MeterOnlyAudit` that explicitly does NOT touch ledger tables. Reviewer picks whichever pattern the diff uses and verifies the constraint holds. |
| `F4` | Reasoning for skipping the sidecar reserve path is documented in a code comment referencing design §3 + §4.3. |
| `F5` | `tenant_id` for the meter audit row comes from the same upstream resolution path as BYOK (existing `tenant_id_from_request` or equivalent) — no new resolution mechanism. |
| `F6` | `pricing_version` written into the meter audit row matches what `BYOK` writes for the same call — both read from the same `PricingSnapshot`. |

## 5. Hard-cap UX assertions

`COV_64` ships the synthetic 429 path. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `H1` | Synthetic 429 response shape matches what the **vendor** would return — `error.type = "rate_limit_exceeded"`, `Retry-After` header present. Asserted by `hard_cap_429_body_matches_anthropic_shape`. |
| `H2` | The vendor 429 shape diverges between Anthropic and OpenAI (Anthropic uses `error.type`, OpenAI uses `error.code`). The 429 response is shape-matched to the vendor of the inbound request via `cfg.kind`. Reviewer confirms a per-provider response shape selector. |
| `H3` | Distinct SpendGuard-specific signal in the response: `error.code = "spendguard_subscription_cap"` (or `error.type` for the OpenAI shape) lets a downstream log analyser distinguish SpendGuard 429s from vendor 429s. |
| `H4` | Upstream is **not called** when hard-cap triggers. Asserted by `hard_cap_returns_429_before_upstream_call` (mock panics on upstream invocation). |
| `H5` | Audit row for a hard-cap-triggered call has `decision = STOP_RUN_PROJECTION` and `reason_code = "subscription_cap_exceeded"`. Distinct from BYOK `BUDGET_EXHAUSTED` so dashboards can split the categories. |
| `H6` | Hard-cap does NOT leak the threshold value in the 429 response body. The message is a fixed string; the operator finds the actual threshold via the dashboard, not via the CLI error. |
| `H7` | `Retry-After` is bounded at 24h (`T12`). |

## 6. Migration ordering / inventory assertions

`COV_61` + `COV_63` + `COV_65` add migrations 0044/0045/0046. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `G1` | Migration numbers are contiguous and don't collide with existing 0044+. Reviewer runs `ls services/canonical_ingest/migrations/004*.sql` and confirms only D13 owns 0044-0046. |
| `G2` | `migration_inventory.toml` updated with all three migrations, checksum-pinned per existing convention. |
| `G3` | Down-migrations exist where the existing convention requires them. Reviewer cross-checks with existing 004x migrations for the convention. |
| `G4` | RLS policy for `subscription_caps` references `current_setting('spendguard.tenant_id')` — same mechanism as other RLS-protected tables (no new isolation mechanism). |
| `G5` | Partial index `idx_audit_outbox_subscription_meter` has matching predicate `WHERE reservation_source = 'subscription_meter'` — typed-narrowed for planner. |
| `G6` | CHECK constraints on `reservation_source` and `import_source` enumerate exactly the values from `design.md` §4.3 + §5 — reviewer cross-references both. |

## 7. Importer stub assertions

`COV_65` adds two new crates. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `I1` | Both crates declare `publish = false` in Cargo.toml — they are internal until the live feature lands. |
| `I2` | The `live` feature exists but pulls **no dependencies** in the default build. Reviewer reads `Cargo.toml` and confirms `[features] live = [...]` lists only deps that are themselves `optional = true`. |
| `I3` | `import_record_to_audit_row` is **pure** — no IO, no global state. |
| `I4` | The contract test `import_record_schema_matches_pg_check_constraint` actually round-trips through a real PG instance. Reviewer reads the test setup and confirms it uses `sqlx::postgres::PgPool` with migration 0046 applied, not a mock. |
| `I5` | README for each crate explicitly says "stub — live polling deferred to vendor API release", links back to `design.md` §5. |

## 8. Demo / Makefile assertions

`COV_66` adds demo targets. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `D1` | New Makefile targets follow the existing `demo-verify-*` naming. |
| `D2` | Each verifier SQL uses the same audit_outbox columns that existing `verify_step_*.sql` files use — no schema drift. |
| `D3` | Demo replay harness `replay_har.py` does NOT send the redacted `FAKE_*` tokens to the real vendor — the demo's HTTPS upstream is a docker-compose'd stub, not `api.anthropic.com`. Reviewer reads the compose file. |
| `D4` | `make demo-clean` removes any D13-specific artefacts (test rows in `subscription_caps`, replayed audit rows). |
| `D5` | Demo targets exit cleanly even when a HAR fixture is replayed multiple times — idempotent. |

## 9. Docs assertions

`COV_66` adds two Starlight docs pages. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `C1` | Both pages explicitly state "SpendGuard cannot enforce subscription quotas — only Anthropic/OpenAI can" above the fold. |
| `C2` | Both pages explain the three modes with their UX trade-offs in a table. |
| `C3` | Both pages cross-link to D02's install page as a prerequisite. |
| `C4` | The hard-cap section warns the operator that the 429 will look like a vendor error to the CLI user. |
| `C5` | Both pages mention the importer reconciliation path with a "coming when vendor APIs open" note linked to the importer crate READMEs. |
| `C6` | Embedded code/JSON examples for the 429 response shape are wrapped in `is:raw` (Starlight Astro convention per project memory). |

## 10. R1-R5 escalation criteria

| Round | Blocker count | Action |
|-------|--------------|--------|
| R1 | 0 → MERGE | none |
| R1 | ≥ 1 → dispatch same implementer with findings | typical 2-4 findings on first review (D13 surface is wider than D02; classifier + meter + cap + hard-cap + importer + 3 migrations) |
| R2-R4 | drop to 0 → MERGE | follow normal cadence |
| R5 | ≥ 1 Blocker → Staff+ panel arbitration | panel composition per build plan §1.3 |

**R5 panel summarizer override:** Security Engineer (per design §9 locked decision #6). Rationale: Authorization-token parsing + hard-cap synthetic-429 are both PII-adjacent and DoS-adjacent surfaces; the security framing dominates the architecture framing for arbitration weighting.

## 11. Per-slice review focus

| Slice | Focus areas |
|-------|-------------|
| `COV_60_d13_subscription_classifier` | §1 (T1-T3), §3 (M1-M7) |
| `COV_61_d13_meter_audit_row` | §1 (T4, T11), §2 (X1-X3, X6), §4 (F1-F6), §6 (G1-G2, G6) |
| `COV_62_d13_codex_route_and_estimate` | §2 (X4-X5), §4 (F2, F4, F6) |
| `COV_63_d13_threshold_and_softcap` | §1 (T6, T9), §6 (G4) |
| `COV_64_d13_hardcap_synthetic_429` | §1 (T5, T12-T13), §5 (H1-H7) |
| `COV_65_d13_importer_stubs` | §1 (T14), §7 (I1-I5), §6 (G1-G2 for migration 0046) |
| `COV_66_d13_demo_and_docs` | §8 (D1-D5), §9 (C1-C6) |

Each slice's review pass only consults the focus areas listed (plus repo-wide standards); the reviewer is NOT asked to re-check the whole list for every slice.
