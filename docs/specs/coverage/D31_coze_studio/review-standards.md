# D31 — Coze Studio Model Provider — Review Standards

**Audience:** `superpowers:code-reviewer` skill (per build-plan §1.2, the canonical reviewer for every slice). Backup: R5 panel arbitration (build-plan §1.3).
**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Replaces:** the codex CLI adversarial loop used in earlier hardening phases. R1-R5 = re-invocations of `superpowers:code-reviewer` per build-plan §1.1.

## §1. Per-slice acceptance bar

A slice passes when, **and only when**:

1. The slice's diff matches the file boundary in `implementation.md` §1 (e.g. Slice 1 touches only `examples/coze-studio/coze-workspace-config.yaml`, `examples/coze-studio/headers-cheatsheet.md`, `examples/coze-studio/README.md`).
2. All hard gates from `acceptance.md` §1 that are runnable at this slice's commit point pass.
3. `superpowers:code-reviewer` returns zero Blockers and zero Majors. Minors may be deferred to a follow-up GitHub issue with explicit rationale captured in the slice's commit message.
4. The slice maintains backwards compatibility per `implementation.md` §3 — no edits to `sdk/`, `services/`, `crates/`, `proto/`, or existing `deploy/demo/compose.yaml` and Makefile demo branches other than the additive `coze_studio_real` branch.

## §2. Architectural invariants (BLOCK — apply to every slice)

| # | Invariant | How to verify |
|---|-----------|---------------|
| 2.1 | D31 is a *recipe + docs + demo overlay* only. No Rust, no proto, no SDK, no new sidecar endpoint. | `git diff main -- 'services/**' 'crates/**' 'sdk/**' 'proto/**'` empty across every D31 slice. |
| 2.2 | The HTTP companion endpoint `/v1/openai/chat/completions` is reused as-is from D09 SLICE 1. No D31 change to it. | `git diff main -- 'services/sidecar/src/server/http_companion.rs'` empty. |
| 2.3 | Fail-closed default — no `fail_open` flag anywhere in the v1 snippet, smoke, demo, or docs. | `! grep -r "fail_open" examples/coze-studio/ deploy/demo/coze_studio/ docs/site/docs/integrations/coze-studio.md` |
| 2.4 | Coze DB isolation — Coze stack uses a database named `coze_db` (or similar), never `spendguard_ledger`. | `grep "coze_db" examples/coze-studio/docker-compose.coze.yaml deploy/demo/coze_studio/compose.override.yaml` non-empty. |
| 2.5 | Coze image pinned by SHA256 digest, not floating tag. | `grep -E "coze-studio[^[:space:]]*@sha256:" examples/coze-studio/docker-compose.coze.yaml deploy/demo/coze_studio/compose.override.yaml` non-empty. |
| 2.6 | No PII / no secrets in repo. No literal `OPENAI_API_KEY` value; env-var ref only. | `! grep -rE 'sk-[A-Za-z0-9]{20,}' examples/coze-studio/ deploy/demo/coze_studio/` |
| 2.7 | Tenant header required — no "default tenant" silent fallback for missing `X-SpendGuard-Tenant-Id`. | inspect smoke + demo error-case assertions; companion error path verified to return 400 not 200. |
| 2.8 | All audit writes route through the existing decision/transaction path of the sidecar. No direct `INSERT INTO audit_outbox` from any D31-introduced code. | `git grep "INSERT INTO audit_outbox" examples/coze-studio/ deploy/demo/coze_studio/` must be empty. |

## §3. Slice-specific reviewer checklist

### Slice 1 — Coze workspace config snippet

| # | Check | Severity |
|---|-------|----------|
| 3.1.1 | `coze-workspace-config.yaml` is YAML-valid (`yaml.safe_load`). | Blocker |
| 3.1.2 | Snippet declares all three `X-SpendGuard-Tenant-Id` / `X-SpendGuard-Budget-Id` / `X-SpendGuard-Window-Instance-Id` headers. | Blocker |
| 3.1.3 | Snippet `api_key` field uses `${OPENAI_API_KEY}` env-var form, NOT a literal key. | Blocker |
| 3.1.4 | Snippet `base_url` targets the SpendGuard sidecar HTTP companion path `/v1/openai`, not `/v1` (Coze must hit the companion, not be misdirected). | Blocker |
| 3.1.5 | Snippet declares TLS material (CA + client cert + key) — companion is mTLS-only per D09 §3.1. | Blocker |
| 3.1.6 | Snippet declares at least 2 OpenAI models in the `models` list. | Major |
| 3.1.7 | `README.md` covers prereqs, install steps, smoke test, and a Troubleshooting section. | Blocker |
| 3.1.8 | `headers-cheatsheet.md` documents the format of each header (e.g. tenant ID is the Coze workspace ID; budget ID is the SpendGuard budget UUID). | Major |
| 3.1.9 | No `fail_open` field anywhere in snippet / README / cheatsheet. (§3.4 design lock) | Blocker |
| 3.1.10 | No outbound network call when an operator reads the README (no `curl https://...` recipes that probe at read-time). | Minor |

### Slice 2 — HTTP companion smoke

| # | Check | Severity |
|---|-------|----------|
| 3.2.1 | `docker-compose.coze.yaml` pins Coze Studio image by SHA256 digest. | Blocker |
| 3.2.2 | `smoke.sh` requires `OPENAI_API_KEY` env-var and exits with explicit error if unset. | Blocker |
| 3.2.3 | Smoke asserts HTTP 200 from the companion AND the response body is OpenAI-shaped (`.choices[0].message.content` accessible). | Blocker |
| 3.2.4 | Smoke asserts a sidecar audit row exists with `decision_context->>'integration' = 'coze_studio'`. INV-2. | Blocker |
| 3.2.5 | Smoke has a negative case — missing `X-SpendGuard-Tenant-Id` → 400 with `MISSING_TENANT` code. INV-4. | Blocker |
| 3.2.6 | Smoke tears down all containers + volumes on success AND on failure (use `trap`). | Major |
| 3.2.7 | Smoke does NOT drive Coze Studio's UI (UI flow covered in Slice 3 demo). Comment in script states this. | Minor |
| 3.2.8 | Smoke does NOT add any test data to the long-lived `spendguard_ledger` DB used by other demo modes (isolated DB or cleaned up). | Blocker |
| 3.2.9 | mTLS handshake uses real client cert + key, NOT `-k` / `--insecure`. | Blocker |
| 3.2.10 | D09 SLICE 1 endpoint presence verified before smoke runs (early exit with clear error if companion not built). | Major |

### Slice 3 — Demo mode

| # | Check | Severity |
|---|-------|----------|
| 3.3.1 | `DEMO_MODE=coze_studio_real` Makefile branch follows the existing pattern (echo line + `OPENAI_API_KEY` check + `$(COMPOSE)` invocation). | Major |
| 3.3.2 | `compose.override.yaml` overlays additional services (`coze-studio`, `coze-postgres`, `coze-redis`, `coze-seed`); does NOT mutate existing services in `compose.yaml`. | Blocker |
| 3.3.3 | Coze containers use a separate database name (`coze_db`) on the shared Postgres instance, NOT `spendguard_ledger`. INV-8. | Blocker |
| 3.3.4 | Demo driver `client.py` step 1 (ALLOW) verifies reservation row exists BEFORE the upstream OpenAI hit (ordering check). INV-2. | Blocker |
| 3.3.5 | Demo driver step 2 (DENY) asserts upstream stub counter unchanged across the DENY. INV-1. | Blocker |
| 3.3.6 | Demo driver step 3 (STREAMING) asserts SSE chunks reach Coze AND end-of-stream commit row exists with `decision_context->>'stream' = 'true'`. INV-5. | Blocker |
| 3.3.7 | `verify_step_coze_studio_real.sql` includes ALL 7 assertions from `implementation.md` §2 Slice 3 (count + DENY + commit + streaming + stub-hits + canonical-events + chain-verify). | Blocker |
| 3.3.8 | Driver writes the success line `[demo] coze_studio_real ALL 3 steps PASS (ALLOW + DENY + STREAM)` verbatim on success. | Major |
| 3.3.9 | Driver exits with code 9 on gate failure (matches existing demo driver convention). | Minor |
| 3.3.10 | No regressions in adjacent demo modes (`decision`, `litellm_real`, `litellm_deny`, `cost_advisor`, `approval`) — their compose / Makefile branches unchanged. INV-7. | Blocker |
| 3.3.11 | Outbox-closure check runs after the demo per existing `Makefile` pattern. | Major |
| 3.3.12 | Coze image pin (§3.2.1) carried over to demo overlay. INV-9. | Blocker |
| 3.3.13 | Tenant header injection in seeded Coze workspace config matches the snippet shape from Slice 1. | Major |

### Slice 4 — Docs page

| # | Check | Severity |
|---|-------|----------|
| 3.4.1 | New page `docs/site/docs/integrations/coze-studio.md` renders via `cd docs/site && npm run build`. | Blocker |
| 3.4.2 | Page includes the verbatim workspace config snippet from `examples/coze-studio/coze-workspace-config.yaml`. | Major |
| 3.4.3 | Decision matrix lists 2+ install paths (D31 base-URL vs D02 egress-proxy) with explicit "when to use" rows. | Major |
| 3.4.4 | "Limitations" section explicitly states: Coze Cloud unsupported, Anthropic/Gemini/Bedrock deferred to v1.1, no mid-stream cap, no native plugin SDK in v1. | Blocker |
| 3.4.5 | "Troubleshooting" section covers: mTLS chain mismatch, tenant header missing, workspace ID extraction. | Major |
| 3.4.6 | Decision matrix row for "Coze + terminal CLIs + other apps on same pod" steers to D02. | Minor |
| 3.4.7 | README adapter integrations table gains exactly one row referencing Coze Studio with link to `examples/coze-studio/README.md`. | Major |
| 3.4.8 | No fictional or unimplemented features mentioned (no claim of streaming-mid-stream cap, no claim of Anthropic v1). | Blocker |
| 3.4.9 | No PII / no real credentials in any sample. | Blocker |

## §4. Cross-cutting reviewer focus areas (every slice)

| Area | What to check | Severity if missed |
|------|---------------|--------------------|
| Backwards compatibility | Did the slice mutate `sdk/`, `services/`, `crates/`, `proto/`, or existing demo modes' compose files? | Blocker |
| Recipe hygiene | Snippets / cheatsheets use placeholder values (`<COZE_WORKSPACE_ID>`, `${OPENAI_API_KEY}`), never real credentials. | Blocker |
| Image pin discipline | Every external image pinned by SHA256 digest, not floating tag. | Blocker |
| Mtls verification | TLS connections use `--cacert` + `--cert` + `--key`; never `-k` / `--insecure`. | Blocker |
| Secret leakage | No literal `sk-*` keys, no `master_key`, no `OPENAI_API_KEY` value. Env-var refs only. | Blocker |
| Test isolation | Slice 1 lint runs without Docker / network. Slice 2 smoke isolates to its compose stack. Slice 3 demo isolates Coze DB. | Major |
| Documentation accuracy | Every claim in docs is implemented or in anti-scope; nothing promised but absent. | Blocker |
| D09 SLICE 1 reuse | The companion endpoint is referenced by URL path, never re-implemented in D31. | Blocker |
| Coze upstream PR | D31 does NOT open or pin an upstream PR to Coze; recipe lives in our repo only. | Major |
| Anti-scope discipline | No Coze plugin SDK code, no Anthropic / Gemini / Bedrock provider slots, no Coze Cloud automation. | Blocker |

## §5. R1-R5 review loop reminders (per build-plan §1.1)

| Round | Reviewer action | Implementer action on findings |
|-------|----------------|--------------------------------|
| R1 | Run `superpowers:code-reviewer` on slice diff + this checklist. | Address every Blocker + Major. Defer Minors with rationale in commit message. |
| R2 | Re-run reviewer on the post-fix diff. | Same as R1. |
| R3 | Re-run. By R3, Blockers should be at zero. | If R3 still has Blockers, escalate to R4 with structural changes — do not patch around. |
| R4 | Last "self-contained" round. | Significant structural changes may invalidate earlier findings; reviewer re-evaluates the whole slice diff, not just deltas. |
| R5 | Final round before panel. | If R5 has any Blocker, escalate to Staff+ panel arbitration per build-plan §1.3. |
| Panel | 5 panelists per build-plan §1.3. Summarizer Software Architect. | Implementer follows ruling (merge-with-residuals / block / rework). |

## §6. Panel-arbitration likely triggers (so the implementer knows)

Likely D31 triggers:

- **Slice 1 schema lock:** if Coze Studio's workspace YAML schema does not match what we expect (Coze publishes v2 with a breaking change between spec-write and ship), panel decides whether to pin to a specific Coze version range, ship two snippets, or block until Coze stabilises.
- **Slice 2 D09 SLICE 1 dependency:** if D09 SLICE 1 has landed but with a different companion endpoint path or different header contract than D31 assumes, panel decides whether D31 adjusts (path/header alignment slice) or D09 SLICE 2 adjusts (companion-side compatibility shim).
- **Slice 3 demo footprint:** Coze Studio's full stack pulls ~1.5 GB. If CI cell timing exceeds 8 min, panel decides whether to gate the demo behind a tagged-build matrix cell only (current plan) or strip Coze down further.
- **Slice 4 decision-matrix complexity:** D02 + D03 + D10 + D31 + D33 + D34 + D37 all overlap. Panel decides whether D31 docs page repeats the decision matrix from D03 verbatim or links out, and whether the matrix needs a separate document at `docs/site/docs/integrations/no-code-decision-matrix.md`.

## §7. Slice-merge order is fixed

Per dependency in `implementation.md` §1: **Slice 1 → 2 → 3 → 4**, never reorder.

- Slice 1 has no dependency on D09 SLICE 1 — it's snippets + docs only and can ship even before D09.
- Slice 2 strictly requires D09 SLICE 1 on main (G3 enforces).
- Slice 3 depends on Slice 1 (snippet) + Slice 2 (compose harness) + D09 SLICE 1 + existing canonical-ingest + outbox-forwarder pipeline.
- Slice 4 depends on Slice 3 (demo working) and Slice 1 (snippet) — docs reference both.

## §8. Final reviewer override

If the reviewer believes the spec itself is wrong (e.g. Pattern 2 base-URL vs Pattern 3 plugin SDK choice, OpenAI-only v1 scope, D09 SLICE 1 hard dependency, fail-closed-only default), flag it as a Blocker on the relevant slice with rationale referencing `design.md` §3 "Key architectural decisions" — do not silently deviate. Spec changes route through Staff+ panel per build-plan §1.3.
