# D32 — Review Standards

**Audience:** `superpowers:code-reviewer` skill (per build-plan §1.2, the canonical reviewer for every slice). Backup: R5 panel arbitration (build-plan §1.3).
**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Replaces:** the codex CLI adversarial loop used in earlier hardening phases. R1-R5 = re-invocations of `superpowers:code-reviewer` per build-plan §1.1.

## 1. Per-slice acceptance bar

A slice passes when, **and only when**:

1. The slice's diff matches the file boundary in `implementation.md` §2 (e.g. Slice 1 touches only `integrations/botpress/` scaffold + `package.json` + `tsconfig.json` + `tsup.config.ts` + `biome.json` + `botpress.integration.yaml` + `README.md` + `src/version.ts`).
2. All hard gates from `acceptance.md` §1 that are runnable at this slice's commit point pass.
3. `superpowers:code-reviewer` returns zero Blockers and zero Majors. Minors may be deferred to a follow-up GitHub issue with explicit rationale captured in the slice's commit message.
4. The slice maintains backwards compatibility per `implementation.md` §3 — no edits to `sdk/typescript/`, `sdk/python/`, no proto changes, no DB schema changes, no Rust changes, no D09 HTTP companion mutations.

## 2. Slice-specific reviewer checklist

For each slice the reviewer MUST verify each row that applies. Rows marked `Blocker` are non-negotiable; one Blocker fails the slice.

### Slice 1 — Integration scaffold

| # | Check | Severity |
|---|-------|----------|
| 1.1 | `botpress.integration.yaml` declares `name: spendguard`, `version` is semver-shaped, `description` non-empty, `icon` referenced. | Blocker |
| 1.2 | `package.json` declares `peerDependencies` `@spendguard/sdk@^0.1.0` AND `@botpress/sdk@^0.7.0,<0.8.0`. | Blocker |
| 1.3 | `package.json` declares `"type": "module"`, `"engines": { "node": ">=20.10.0" }`. | Blocker |
| 1.4 | `tsup.config.ts` externalises `@spendguard/sdk` and `@botpress/sdk` (peer-deps not bundled). | Blocker |
| 1.5 | `vitest.config.ts` configured with `environment: "node"` (NOT jsdom — Botpress runs server-side). | Major |
| 1.6 | `biome.json` extends the D05 base or matches the repo standard (no new lint deviations). | Major |
| 1.7 | No outbound network calls in scaffold's import path (no `fetch` on `import`). | Major |
| 1.8 | README declares the integration, env vars, install command outline. | Major |

### Slice 2 — Hooks registration skeleton + config + validateConfiguration

| # | Check | Severity |
|---|-------|----------|
| 2.1 | `src/config.ts` Zod schema validates against the documented field list (sidecarUrl, spendguardBudgetId, spendguardWindowInstanceId, upstreamProvider, tenantId, optional TLS paths). | Blocker |
| 2.2 | `upstreamProvider` Zod enum is `["openai", "anthropic", "bedrock"]` (no `cohere`, no `gemini` in v1). | Blocker |
| 2.3 | `src/index.ts` registers BOTH `beforeAiGeneration` and `afterAiGeneration` hooks (not just one). | Blocker |
| 2.4 | `register` field on `new Integration(...)` is wired to `validateConfiguration`. | Blocker |
| 2.5 | `SpendGuardReservation` is composition-only (no inheritance from any `@botpress/sdk` base). | Blocker |
| 2.6 | `SpendGuardReservation.__init__` reads sidecar URL + budget IDs from passed `Configuration`; missing → `SpendGuardConfigError` naming the field. | Blocker |
| 2.7 | `validateConfiguration` issues a 1-token reserve+release roundtrip, NOT only a credential probe. INV-4. | Blocker |
| 2.8 | No global state (no module-level mutable in `reservation.ts` / `index.ts`). | Blocker |

### Slice 3 — Reserve/commit wiring

| # | Check | Severity |
|---|-------|----------|
| 3.1 | `SpendGuardReservation.reserve` builds `BudgetBinding` and validates required fields (rejects empty `budget_id`). | Blocker |
| 3.2 | `SpendGuardReservation.reserve` uses D05's `deriveIdempotencyKey` for the LLM_CALL_PRE idempotency key — no re-derivation. | Blocker |
| 3.3 | `SpendGuardReservation.reserve` uses D05's `computePromptHash` for the prompt hash field — no re-derivation. | Blocker |
| 3.4 | `commitSuccess` passes `estimated_amount_atomic = String(realUsage.inputTokens + realUsage.outputTokens)` + `provider_reported_amount_atomic = ""` (matches existing TS adapter contract). | Blocker |
| 3.5 | `releaseFailure` swallows release-RPC errors but logs WARN (TTL sweep backstop). | Blocker |
| 3.6 | `releaseFailure` classifies `AbortError` / `CancelledError` → CANCELLED via the same regex pattern as `_classify_failure` (`litellm.py:735-760`). | Major |
| 3.7 | `beforeAiGeneration` hook on DENY throws Botpress `RuntimeError` with `code: "BUDGET_DENIED"`. | Blocker |
| 3.8 | `beforeAiGeneration` hook on DEGRADE throws `RuntimeError` `code: "BUDGET_DEGRADED"`. Default fail-closed. | Blocker |
| 3.9 | `afterAiGeneration` reads `event.payload.usage` (`{ inputTokens, outputTokens }`) — Botpress's normalised shape, NOT provider-specific. | Blocker |
| 3.10 | When `event.payload.usage` is missing, estimator-snapshot fallback commits + WARN log carries substring `falling back to estimator`. INV-5 secondary. | Major |
| 3.11 | `afterAiGeneration` when `data._spendguardHandle` is undefined (before-hook didn't run) returns `{ data }` without RPC — no phantom commit. | Blocker |
| 3.12 | `data._spendguardHandle` cleared after successful commit (no leak across hooks). INV-10. | Blocker |
| 3.13 | No logging of `configuration.sidecarUrl`, `configuration.tlsKeyPath`, or any TLS material. INV-6. | Blocker |
| 3.14 | DENY path: mock sidecar history shows ZERO `/v1/trace` records (proxy for upstream HTTP). INV-1. | Blocker |
| 3.15 | Tests R01-R11 + B01-B07 + A01-A08 + AD01-AD06 + L01-L05 present. | Major |

### Slice 4 — Tests against self-hosted Botpress v12

| # | Check | Severity |
|---|-------|----------|
| 4.1 | `tests/integration-v12.test.ts` uses testcontainers-node with `botpress/server:v12.30.x` pinned by **digest** (NOT floating tag). | Blocker |
| 4.2 | Test boots a REAL Botpress runtime (not a mocked one) and exercises the actual hook dispatch path. | Blocker |
| 4.3 | Mock sidecar (`_mockSidecar.ts`) is a real msw HTTP server, not an in-memory stub. | Major |
| 4.4 | I02 (DENY) asserts mock upstream records ZERO hits via timestamped event log. INV-1. | Blocker |
| 4.5 | I01 (ALLOW ordering) asserts `/v1/decision` POST precedes mock upstream HTTP via strict-order event log. INV-2. | Blocker |
| 4.6 | I04 (`validateConfiguration` probe) verified end-to-end via POSTing config to Botpress admin API. INV-4. | Blocker |
| 4.7 | `vitest.integration.config.ts` excluded from default `pnpm test` to keep unit tier fast. | Major |
| 4.8 | CI job for integration suite (`botpress-integration-ci.yml`) gated by path filter `integrations/botpress/**`. | Major |
| 4.9 | Docker layer cache step present in CI workflow to amortise Botpress image pull. | Minor |
| 4.10 | Tests I01-I04 present and pass. | Blocker |

### Slice 5 — Demo mode + docs + publish workflow

| # | Check | Severity |
|---|-------|----------|
| 5.1 | `DEMO_MODE=botpress_real` branch wires `botpress/compose.botpress.yaml` overlay — `botpress-server` + `botpress-seed` services both present. | Blocker |
| 5.2 | Compose service `botpress-server` mounts `integrations/botpress/dist/` read-only at the integrations path. | Blocker |
| 5.3 | Compose service `botpress-server` configured with sidecar HTTP companion URL + CA trust. | Blocker |
| 5.4 | Botpress image pinned by digest, not by floating tag. | Blocker |
| 5.5 | Demo driver step 2 (DENY) asserts **upstream stub counter unchanged**. INV-1. | Blocker |
| 5.6 | Demo driver step 1 (ALLOW) verifies sidecar `/v1/decision` row precedes upstream stub hit (strict order). INV-2. | Blocker |
| 5.7 | `verify_step_botpress.sql` includes ALL 6 assertions from `tests.md` §4 (including the `stub_hits` no-hit-on-deny check). | Blocker |
| 5.8 | Outbox-closure check runs after the demo per existing `Makefile` pattern. | Major |
| 5.9 | Driver writes the success line `botpress_real ALL 3 steps PASS (ALLOW + DENY + STREAM)` exactly. | Major |
| 5.10 | No regressions in adjacent demo modes (`decision`, `default`, `litellm_real`, `litellm_deny`, `litellm_direct`, `dify_plugin_real`) — their compose / Makefile branches unchanged. | Blocker |
| 5.11 | New page `docs/site/docs/integrations/botpress.md` renders via `cd docs/site && npm run build`. | Blocker |
| 5.12 | Decision matrix lists at least 2 paths (Botpress integration / egress proxy) with explicit "when to use" rows. | Major |
| 5.13 | "Limitations" section explicitly states: no workflow-node gating beyond AI hook, no token-by-token cap, no channel plugin coverage. | Blocker |
| 5.14 | README adapter integrations table gains exactly one row with the `botpress integrations push` command. | Major |
| 5.15 | `botpress-integration-publish.yml` workflow lints clean (`actionlint`). | Blocker |
| 5.16 | Workflow's marketplace-push step is conditional on `secrets.BOTPRESS_MARKETPLACE_TOKEN` so a missing secret on PR CI is not a failure. | Major |
| 5.17 | Workflow runs only on `botpress-integration-v*` tag pushes; not on every push. | Blocker |
| 5.18 | Workflow publishes via npm OIDC Trusted Publisher with `--provenance` (mirrors D05 pattern). | Blocker |

## 3. Cross-cutting reviewer focus areas (every slice)

| Area | What to check | Severity if missed |
|------|---------------|--------------------|
| Backwards compatibility | Did the slice mutate `sdk/typescript/`, `sdk/python/`, or D09 SLICE 1's HTTP companion? Did it edit existing demo modes' compose files? | Blocker |
| Type safety | All exports carry full TS types. No `any` in public surface. Zod schemas exported for consumer use. | Major |
| ESM purity | No `require()` calls. No CJS shims. `import` paths carry `.js` suffix as per ESM resolution. | Blocker |
| Logging | All `console.warn` / `console.info` callsites carry the `spendguard:botpress` prefix matching the rest of the SDK. | Minor |
| Error messages | `SpendGuardConfigError` strings name the offending config field. `RuntimeError` codes include `BUDGET_DENIED` / `BUDGET_DEGRADED` / `BUDGET_CONFIG`. | Major |
| Secret leakage | NO logging of `configuration.sidecarUrl`, `tlsKeyPath`, or any env var name containing `KEY`/`SECRET`/`PASSWORD`/`TOKEN`. INV-6. | Blocker |
| Test isolation | Unit tests do NOT require Docker, do NOT require a running sidecar, do NOT make outbound HTTP. | Blocker |
| Async cleanliness | Hook functions are `async`; mock sidecar resources clean up in `afterEach` / `afterAll`. | Major |
| Re-entrancy | Per-hook-call `SpendGuardReservation` instance OR per-call state isolation; no shared mutable in hook handlers. | Blocker |
| Dependency surface | No new runtime dependency added beyond `@spendguard/sdk` and `@botpress/sdk` (peers). Dev-deps: vitest, msw, testcontainers, tsup, biome only. | Major |
| Bundle size | `dist/index.js` < 100 KB (peer-deps externalised). | Major |
| D09 contract reuse | All sidecar calls go through D09 SLICE 1's published `/v1/decision`, `/v1/trace` shapes — no new endpoints. | Blocker |

## 4. R1-R5 review loop reminders (per build-plan §1.1)

| Round | Reviewer action | Implementer action on findings |
|-------|----------------|--------------------------------|
| R1 | Run `superpowers:code-reviewer` on slice diff + this checklist. | Address every Blocker + Major. Defer Minors with rationale in commit message. |
| R2 | Re-run reviewer on the post-fix diff. | Same as R1. |
| R3 | Re-run. By R3, Blockers should be at zero. | If R3 still has Blockers, escalate to R4 with structural changes — do not patch around. |
| R4 | Last "self-contained" round. | Significant structural changes may invalidate earlier findings; reviewer re-evaluates the whole slice diff, not just deltas. |
| R5 | Final round before panel. | If R5 has any Blocker, escalate to Staff+ panel arbitration per build-plan §1.3. |
| Panel | 5 panelists per build-plan §1.3. Summarizer Software Architect. | Implementer follows ruling (merge-with-residuals / block / rework). |

## 5. Panel-arbitration likely triggers (so the implementer knows)

Likely D32 triggers:

- **Slice 3 hook re-entrancy:** Botpress's hook concurrency model under high conversation throughput. If the per-call `_spendguardHandle` stashing on `data` leaks across hooks, panel decides whether to switch to a `WeakMap<HookInput, Handle>` or push for a Botpress SDK upgrade exposing per-call context.
- **Slice 3 usage-shape drift:** if Botpress 0.7.x patch versions ship varying `usage` field shapes for Bedrock vs OpenAI vs Anthropic, panel decides whether to ship per-provider extractors or unify behind sidecar `count_tokens` for both pre-call and post-call.
- **Slice 4 Botpress v12 boot time in CI:** if the Botpress container takes > 60s to start in testcontainers, panel decides whether to mock the runtime (regression in integration coverage) or accept the longer CI time + caching strategy.
- **Slice 5 marketplace push:** if Botpress Cloud marketplace policy requires a code-signing chain we don't yet have, panel decides whether to ship sideload-only for v1 and defer marketplace to v1.1.
- **D09 SLICE 1 not landed yet:** if D32 starts before the HTTP companion lands, panel decides whether to absorb the companion endpoint extraction into D32 SLICE 1 (mirror D09 §6 clause) or block D32 on D09 SLICE 1's merge.

## 6. Slice-merge order is fixed

Per dependency in `implementation.md` §2: **Slice 1 → 2 → 3 → 4 → 5**, never reorder.

- Slice 2 depends on Slice 1's scaffold (package.json + tsup + biome + vitest wiring).
- Slice 3 depends on Slice 2's `SpendGuardReservation` skeleton + config schema + hook registration.
- Slice 4 depends on Slice 3 (integration tests require the full reserve/commit path).
- Slice 5 depends on Slice 4 (demo and docs reference the working integration).

## 7. Final reviewer override

If the reviewer believes the spec itself is wrong (e.g. composition vs inheritance, HTTP companion reuse vs custom transport, hook registration shape, demo mode footprint), flag it as a Blocker on the relevant slice with rationale referencing `design.md` §5 "Key decisions" — do not silently deviate. Spec changes route through Staff+ panel per build-plan §1.3.
