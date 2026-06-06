# D10 — Review Standards

**Audience:** `superpowers:code-reviewer` skill (per build-plan §1.2, the canonical reviewer for every slice). Backup: R5 panel arbitration (build-plan §1.3).
**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Replaces:** the codex CLI adversarial loop used in earlier hardening phases. R1-R5 = re-invocations of `superpowers:code-reviewer` per build-plan §1.1.

## 1. Per-slice acceptance bar

A slice passes when, **and only when**:

1. The slice's diff matches the file boundary in `implementation.md` §2 (e.g. Slice 1 touches only the `plugins/dify/` scaffold + `requirements.txt` + `pyproject.toml` + README).
2. All hard gates from `acceptance.md` §1 that are runnable at this slice's commit point pass.
3. `superpowers:code-reviewer` returns zero Blockers and zero Majors. Minors may be deferred to a follow-up GitHub issue with explicit rationale captured in the slice's commit message.
4. The slice maintains backwards compatibility per `implementation.md` §3 — no edits to `sdk/python/src/spendguard/integrations/`, no proto changes, no DB schema changes, no Rust changes.

## 2. Slice-specific reviewer checklist

For each slice the reviewer MUST verify each row that applies. Rows marked `Blocker` are non-negotiable; one Blocker fails the slice.

### Slice 1 — Plugin scaffold

| # | Check | Severity |
|---|-------|----------|
| 1.1 | `manifest.yaml` declares `type: model-provider`, `version` is semver-shaped, `name: spendguard`, `author` set, `created_at` ISO-8601. | Blocker |
| 1.2 | `requirements.txt` pins `spendguard-sdk>=0.5.1` (matches the SDK version line shipped 2026-06). | Blocker |
| 1.3 | `requirements.txt` pins `dify-plugin>=0.2.0,<0.3.0`. | Blocker |
| 1.4 | `pyproject.toml` enables editable install for `pytest plugins/dify/`. | Major |
| 1.5 | No outbound network calls in scaffold's import path (no requests on `import`). | Major |
| 1.6 | README declares the plugin daemon container, env vars, install command outline. | Major |

### Slice 2 — Provider manifest + LLM model yaml

| # | Check | Severity |
|---|-------|----------|
| 2.1 | `provider/spendguard.yaml` schema validates against the Dify plugin SDK's published JSON schema (run `dify plugin check`). | Blocker |
| 2.2 | `provider_credential_schema` includes ALL required fields: `upstream_provider`, `upstream_api_key`, `spendguard_budget_id`, `spendguard_window_instance_id`. | Blocker |
| 2.3 | `upstream_provider` options list is `[openai, anthropic, gemini, bedrock]` — Gemini/Bedrock labelled "v1.1+" but selectable so the form does not exclude them prematurely. | Major |
| 2.4 | `upstream_api_key` is `type: secret-input` (NOT `text-input` — secret-input scrubs from logs). | Blocker |
| 2.5 | `models/llm/spendguard.yaml` lists at least one OpenAI model + one Anthropic model. | Major |
| 2.6 | All `supported_model_types` is `[llm]` only. No tools / text-embedding / rerank in v1. | Major |

### Slice 3 — `SpendGuardLLM` skeleton + `_DifyReservation` delegate

| # | Check | Severity |
|---|-------|----------|
| 3.1 | `_DifyReservation` is composition-only (no inheritance from `LargeLanguageModel`). | Blocker |
| 3.2 | `_DifyReservation.__init__` reads `SPENDGUARD_SIDECAR_UDS` and `SPENDGUARD_TENANT_ID`; missing → `SpendGuardConfigError` naming the var. | Blocker |
| 3.3 | `_ensure_client` pattern matches `_LoopBoundCallback._ensure_client` (5s deadline, 1s per attempt, deadline-bounded — `litellm.py` 800-863). | Blocker |
| 3.4 | `reserve` builds `BudgetBinding` and validates it via `_validate_claim_against_binding` (must mirror `litellm.py:149-191`; empty fields rejected). | Blocker |
| 3.5 | `commit_success` passes `estimated_amount_atomic=str(real_amount)` + `provider_reported_amount_atomic=""` (matches existing adapter contract — `litellm.py:550-560`). | Blocker |
| 3.6 | `release_failure` swallows release-RPC errors but logs WARN (TTL sweep backstop). | Blocker |
| 3.7 | `release_failure` classifies `asyncio.CancelledError` → CANCELLED via the same regex pattern as `_classify_failure` (`litellm.py:735-760`). | Major |
| 3.8 | `_DifyReservation` has NO global state (no module-level mutable). | Major |
| 3.9 | `SpendGuardProvider.validate_credentials` issues a 1-token reserve+release roundtrip, not only an upstream-credential probe. INV-4. | Blocker |
| 3.10 | Tests R01-R12 + P01-P06 present. | Major |

### Slice 4 — OpenAI upstream

| # | Check | Severity |
|---|-------|----------|
| 4.1 | OpenAI client is constructed per-call from `credentials["upstream_api_key"]`; NOT cached at module level (multi-workspace safety). | Blocker |
| 4.2 | `model` field passed to OpenAI strips the `spendguard/` prefix before sending. | Blocker |
| 4.3 | Real usage from `response.usage.completion_tokens` feeds `commit_success`. NOT estimator. INV-5. | Blocker |
| 4.4 | DENY path: `respx` HTTP mock records ZERO outbound to `api.openai.com`. INV-1. | Blocker |
| 4.5 | `openai.AuthenticationError` translates to Dify `InvokeAuthorizationError`; `openai.RateLimitError` → `InvokeRateLimitError`; `openai.APIError` → `InvokeError`. | Major |
| 4.6 | `upstream_base_url` honoured. | Major |
| 4.7 | `gemini` / `bedrock` upstream selection raises `InvokeError("not supported in this plugin version")` — does NOT silently fall through. | Blocker |
| 4.8 | No logging of `upstream_api_key`, even partially redacted. INV-6. | Blocker |
| 4.9 | Tests O01-O08 present. | Major |

### Slice 5 — Anthropic upstream + `get_num_tokens`

| # | Check | Severity |
|---|-------|----------|
| 5.1 | Message-format adapter: Dify `system` role goes into Anthropic's top-level `system` field, NOT into `messages`. | Blocker |
| 5.2 | Real usage from `response.usage.input_tokens` + `output_tokens` feeds commit. NOT estimator. | Blocker |
| 5.3 | DENY path: `respx` HTTP mock records ZERO outbound to `api.anthropic.com`. INV-1. | Blocker |
| 5.4 | `get_num_tokens` dispatches to sidecar `count_tokens` UDS RPC, NOT a bundled tokenizer. | Blocker |
| 5.5 | Anthropic `529 Overloaded` translates to Dify `InvokeServerUnavailableError`. | Major |
| 5.6 | Unknown model (not in `models/llm/spendguard.yaml`) raises `InvokeBadRequestError`. | Major |
| 5.7 | Tests A01-A07 present. | Major |

### Slice 6 — Streaming path

| # | Check | Severity |
|---|-------|----------|
| 6.1 | Reservation taken ONCE at the top of `_stream_generate`; commit fires after the stream completes — no per-chunk reserve/commit. | Blocker |
| 6.2 | Cancellation (caller closes the iterator) routes to `release_failure(CANCELLED)`; reservation does NOT leak. INV-7-adjacent. | Blocker |
| 6.3 | Upstream exception mid-stream → `release_failure(FAILURE)` + Dify `InvokeError` translation. | Blocker |
| 6.4 | When upstream omits `usage` frame, estimator-snapshot commit fires + WARN log carries substring `falling back to estimator` (matches `litellm.py:602-607`). INV-5 secondary path. | Major |
| 6.5 | Anthropic's `message_delta` event with `usage.output_tokens` correctly accumulated. | Major |
| 6.6 | Streaming first-chunk is yielded BEFORE the commit RPC (perf + correctness — Dify clients expect a chunk before stream-end). | Blocker |
| 6.7 | Tests S01-S06 present. | Major |

### Slice 7 — Demo mode

| # | Check | Severity |
|---|-------|----------|
| 7.1 | `DEMO_MODE=dify_plugin_real` branch wires the new `dify_plugin/compose.override.yaml` correctly — `dify-api`, `dify-worker`, `dify-plugin-daemon` services all present. | Blocker |
| 7.2 | Compose service `dify-plugin-daemon` mounts the sidecar UDS (read+write) and `plugins/dify/` (read-only). | Blocker |
| 7.3 | Demo driver step 2 (DENY) asserts **upstream stub counter unchanged**. INV-1. | Blocker |
| 7.4 | Demo driver step 1 (ALLOW) verifies reservation row precedes upstream stub hit (strict order). INV-2. | Blocker |
| 7.5 | `verify_step_dify_plugin.sql` includes ALL 6 assertions from `tests.md` §4 (including the `stub_hits` no-hit-on-deny check). | Blocker |
| 7.6 | Outbox-closure check runs after the demo per existing `Makefile` pattern. | Major |
| 7.7 | Driver writes the success line `dify_plugin_real ALL 3 steps PASS (ALLOW + DENY + STREAM)` exactly. | Major |
| 7.8 | No regressions in adjacent demo modes (`decision`, `default`, `litellm_real`, `litellm_deny`, `litellm_direct`) — their compose / Makefile branches unchanged. | Blocker |
| 7.9 | Dify uses a separate database name (`dify_db`) on the shared Postgres instance, not `spendguard_ledger`. | Blocker |
| 7.10 | Dify images pinned by digest, not by floating tag. | Major |

### Slice 8 — Docs + publish workflow

| # | Check | Severity |
|---|-------|----------|
| 8.1 | New page `docs/site/docs/integrations/dify.md` renders via `cd docs/site && npm run build`. | Blocker |
| 8.2 | Decision matrix lists 3 paths (Dify plugin / egress proxy / LiteLLM-routed) with explicit "when to use" rows. | Major |
| 8.3 | "Limitations" section explicitly states: no workflow-step gating, no token-by-token cap, Gemini/Bedrock deferred. | Blocker |
| 8.4 | README adapter integrations table gains exactly one row with the `dify plugin install spendguard.difypkg` command. | Major |
| 8.5 | `dify-plugin-publish.yml` workflow lints clean (`actionlint`). | Blocker |
| 8.6 | Workflow's marketplace-push step is conditional on `secrets.DIFY_MARKETPLACE_TOKEN` so a missing secret on PR CI is not a failure. | Major |
| 8.7 | Workflow runs only on `dify-plugin-v*` tag pushes; not on every push. | Blocker |
| 8.8 | Publish workflow signs the `.difypkg` artefact (cosign or sigstore). | Major |

## 3. Cross-cutting reviewer focus areas (every slice)

| Area | What to check | Severity if missed |
|------|---------------|--------------------|
| Backwards compatibility | Did the slice mutate `sdk/python/src/spendguard/integrations/`? Did it edit existing demo modes' compose files? Did it edit existing SDK extras? | Blocker |
| Type hints | All new public functions carry full hints. `from __future__ import annotations` used at top of each module. | Major |
| Logging | All `log.warning` / `log.info` callsites carry the `spendguard:` prefix matching the rest of the SDK. | Minor |
| Error messages | `SpendGuardConfigError` strings name the offending env var or credential field. `InvokeError` translations include the upstream provider name in the message. | Major |
| Secret leakage | NO logging of `upstream_api_key`, `master_key`, or any env var name containing `KEY`/`SECRET`/`PASSWORD`/`TOKEN`. INV-6. | Blocker |
| Test isolation | Unit tests do NOT require Docker, do NOT require a running sidecar, do NOT make outbound HTTP. | Blocker |
| Async / sync mixing | Plugin daemon SDK calls `_invoke` synchronously; the reservation code MUST bridge to async via a daemon-scoped event loop, NOT `asyncio.run()` per call. | Blocker |
| Drop handles | Any new asyncio task / subprocess / fixture cleans up in `finally` or fixture teardown. | Major |
| Dependency surface | No new runtime dependency added beyond `spendguard-sdk`, `openai`, `anthropic`, `dify-plugin`. | Major |
| Image surface | Plugin daemon container image base is `langgenius/dify-plugin-sdk-python:0.2-slim` or equivalent; no large base unless justified. | Major |

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

Likely D10 triggers:

- **Slice 3 sync/async bridge:** Dify SDK's `_invoke` is sync but SpendGuard's reservation lifecycle is async. If the daemon-scoped event-loop bridge is brittle (e.g. deadlock under concurrent calls), panel decides whether to thread-pool the bridge or push for an async-native Dify SDK upgrade.
- **Slice 6 streaming-fallback path:** if upstream usage detection drifts across provider versions, panel decides whether to ship per-provider usage extractors or unify behind sidecar `count_tokens` for both pre-call and post-stream.
- **Slice 7 demo footprint:** Dify images are 2 GB; if CI cell flake rate exceeds 10%, panel decides whether to mock Dify core in the demo (regression in coverage) or accept the longer CI time.
- **Slice 8 marketplace push:** if Dify marketplace policy requires a code-signing chain we don't yet have, panel decides whether to ship sideload-only for v1 and defer marketplace to v1.1.

## 6. Slice-merge order is fixed

Per dependency in `implementation.md` §2: **Slice 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8**, never reorder.

- Slice 3 depends on Slice 2's manifest fields.
- Slices 4, 5 each depend on Slice 3's `_DifyReservation` API surface.
- Slice 6 depends on Slices 4 + 5 (upstream clients) and reuses their reconcilers.
- Slice 7 depends on Slices 4 + 5 (real upstream forwarding) and on Slice 2's manifest.
- Slice 8 depends on Slices 7 (demo mode working) and 2 (manifest) — docs reference the install command surface that Slice 2 defines.

## 7. Final reviewer override

If the reviewer believes the spec itself is wrong (e.g. composition vs inheritance, manifest schema choices, demo mode footprint, plugin daemon image base), flag it as a Blocker on the relevant slice with rationale referencing `design.md` §5 "Key decisions" — do not silently deviate. Spec changes route through Staff+ panel per build-plan §1.3.
