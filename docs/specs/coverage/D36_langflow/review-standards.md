# D36 — Review Standards

**Audience:** `superpowers:code-reviewer` skill (per build-plan §1.2, the canonical reviewer for every slice). Backup: R5 panel arbitration (build-plan §1.3).
**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`acceptance.md`](acceptance.md).
**Replaces:** the codex CLI adversarial loop used in earlier hardening phases. R1-R5 = re-invocations of `superpowers:code-reviewer` per build-plan §1.1.

## 1. Per-slice acceptance bar

A slice passes when, **and only when**:

1. The slice's diff matches the file boundary in `implementation.md` §2 (e.g. Slice 1 touches only the `plugins/langflow/src/spendguard_langflow/` skeleton + `pyproject.toml` + README).
2. All hard gates from `acceptance.md` §1 that are runnable at this slice's commit point pass.
3. `superpowers:code-reviewer` returns zero Blockers and zero Majors. Minors may be deferred to a follow-up GitHub issue with explicit rationale captured in the slice's commit message.
4. The slice maintains backwards compatibility per `implementation.md` §3 — no edits to `sdk/python/src/spendguard/integrations/langchain.py`, no proto changes, no DB schema changes, no Rust changes.

## 2. Slice-specific reviewer checklist

For each slice the reviewer MUST verify each row that applies. Rows marked `Blocker` are non-negotiable; one Blocker fails the slice.

### Slice 1 — Component skeleton

| # | Check | Severity |
|---|-------|----------|
| 1.1 | `SpendGuardChatModelWrapper` inherits from `langflow.custom.Component`, NOT from `langflow.custom.CustomComponent` (1.8 favoured the new `Component` base). | Blocker |
| 1.2 | All 8 declared inputs match `implementation.md` Slice 1 name + type + defaults exactly. | Blocker |
| 1.3 | `inner` is `HandleInput(name="inner", input_types=["LanguageModel"], required=True)`. NOT `MessageInput` / `DataInput`. | Blocker |
| 1.4 | The single Output declares `types=["LanguageModel"]` so downstream Langflow nodes accept the wrapped model identically to a raw ChatOpenAI. | Blocker |
| 1.5 | `display_name`, `icon`, `description`, `documentation`, `name` class attrs all present + sensible. | Major |
| 1.6 | No outbound network calls in scaffold's import path (no `requests` on `import`). | Major |
| 1.7 | `build_model` raises `NotImplementedError` (wired in Slice 2) — does NOT silently return None. | Major |
| 1.8 | `pyproject.toml` placeholder pins `spendguard-sdk[langchain]>=0.5.1` and `langflow>=1.8.0,<2.0.0`. | Blocker |
| 1.9 | README declares the component, install command outline, and the Langflow version floor. | Major |

### Slice 2 — Wrap logic

| # | Check | Severity |
|---|-------|----------|
| 2.1 | `build_model` imports `SpendGuardChatModel` + `RunContext` + `run_context` + `_RUN_CONTEXT` from `spendguard.integrations.langchain`. NO reimplementation of the reservation lifecycle in this package. | Blocker |
| 2.2 | `SpendGuardClient` is constructed per `build_model()` invocation; NOT cached at module level (multi-flow / multi-tenant safety, INV-4). | Blocker |
| 2.3 | `client.connect()` + `client.handshake()` both awaited before `SpendGuardChatModel(...)` construction. | Blocker |
| 2.4 | Empty `sidecar_uds_path` + env unset → `ValueError` whose message names BOTH the canvas input and the env var name. | Blocker |
| 2.5 | `unit_ref.unit_id` derived as `f"{model_family}.{unit_token_kind}"`. `token_kind` and `model_family` correctly forwarded into the UnitRef. | Major |
| 2.6 | Default estimator uses `max(50, chars // chars_per_token)` floor matching the docstring example in `langchain.py:118-135`. | Major |
| 2.7 | `install_autobind` only enters `run_context` if `_RUN_CONTEXT.get() is None`. Caller-bound contexts MUST win. INV-3. | Blocker |
| 2.8 | `install_autobind` patches `_agenerate` via `object.__setattr__` (Pydantic v2 BaseModel field-assignment dance). | Major |
| 2.9 | `functools.wraps(original_agen)` preserves signature + docstring. | Minor |
| 2.10 | `build_model()` called from a running event loop → clear `RuntimeError`, no deadlock. | Blocker |
| 2.11 | No logging of `tenant_id`, `budget_id`, `window_instance_id` verbatim. Sidecar UDS path may be logged (it's a filesystem path, not a secret). INV-6. | Blocker |
| 2.12 | Tests B01-B09 + A01-A05 present. | Major |

### Slice 3 — Metadata YAML + PyPI packaging

| # | Check | Severity |
|---|-------|----------|
| 3.1 | `metadata/spendguard_chat_model_wrapper.yaml` schema validates against Langflow 1.8 component metadata spec (verified via Langflow's own `langflow components verify` CLI when available, else manual cross-check). | Blocker |
| 3.2 | Metadata YAML version field matches `_version.py` and `pyproject.toml` version field. | Blocker |
| 3.3 | `pyproject.toml` `dependencies` is exactly `["spendguard-sdk[langchain]>=0.5.1", "langflow>=1.8.0,<2.0.0"]` (no extras creep). | Blocker |
| 3.4 | `pyproject.toml` declares `[project.scripts] spendguard-langflow-install = "spendguard_langflow._install:main"`. | Blocker |
| 3.5 | Wheel built via `python -m build --wheel` includes BOTH `component.py` AND `metadata/spendguard_chat_model_wrapper.yaml` (verified via `unzip -l`). | Blocker |
| 3.6 | `_install.py` refuses targets matching system paths (`/usr/*`, `/bin/*`, `/etc/*`, `/System/*`). INV-8. | Blocker |
| 3.7 | `_install.py` `--force` overwrite is opt-in only; default is "refuse and prompt". | Major |
| 3.8 | `_install.py` auto-creates parent dirs when the target subdirectory does not exist. | Major |
| 3.9 | `License` field is `Apache-2.0` matching `spendguard-sdk`. | Major |
| 3.10 | Tests I01-I05 present. | Major |

### Slice 4 — Demo mode

| # | Check | Severity |
|---|-------|----------|
| 4.1 | `DEMO_MODE=langflow_real` branch wires the new `langflow/compose.override.yaml` correctly — `langflow` service present. | Blocker |
| 4.2 | Compose service `langflow` mounts the sidecar UDS (read+write) AND the plugin components directory (read-only) AND the flow seed directory (read-only). | Blocker |
| 4.3 | `LANGFLOW_COMPONENTS_PATH=/app/components_seed` env set on the Langflow container so the wrapper is auto-loaded on boot. | Blocker |
| 4.4 | Demo driver step 2 (DENY) asserts **upstream stub counter unchanged**. INV-1. | Blocker |
| 4.5 | Demo driver step 1 (ALLOW) verifies reservation row precedes upstream stub hit (strict order). INV-2. | Blocker |
| 4.6 | `verify_step_langflow.sql` includes ALL 6 assertions from `tests.md` §3 (including the `stub_hits` no-hit-on-deny check). | Blocker |
| 4.7 | Outbox-closure check runs after the demo per existing `Makefile` pattern. | Major |
| 4.8 | Driver writes the success line `langflow_real ALL 3 steps PASS (ALLOW + DENY + STREAM)` exactly. | Major |
| 4.9 | No regressions in adjacent demo modes (`decision`, `default`, `litellm_real`, `litellm_deny`, `dify_plugin_real`) — their compose / Makefile branches unchanged. | Blocker |
| 4.10 | Langflow uses a separate database name (`langflow_db`) on the shared Postgres instance, not `spendguard_ledger`. | Blocker |
| 4.11 | Langflow image pinned by digest, not by floating tag. | Major |
| 4.12 | The flow seed JSON `flow_chat_openai_wrapped.json` references the canonical demo budget/window IDs (matches `canonical-seed-init` output). | Major |

### Slice 5 — Docs + publish workflow

| # | Check | Severity |
|---|-------|----------|
| 5.1 | New page `docs/site/docs/integrations/langflow.md` renders via `cd docs/site && npm run build`. | Blocker |
| 5.2 | Decision matrix lists 3 paths (Langflow component / egress proxy / Langflow global-provider config deferred) with explicit "when to use" rows. | Major |
| 5.3 | "Limitations" section explicitly states: no embeddings gate, no token-by-token mid-stream cap, global-provider config interception deferred to v1.1, no Langflow Cloud marketplace push. | Blocker |
| 5.4 | README adapter integrations table gains exactly one row with the `pip install spendguard-langflow-component` command. | Major |
| 5.5 | `langflow-component-publish.yml` workflow lints clean (`actionlint`). | Blocker |
| 5.6 | Workflow uses PyPI Trusted Publisher (OIDC), NOT API-token-based auth. | Blocker |
| 5.7 | Workflow runs only on `langflow-component-v*` tag pushes; not on every push. | Blocker |
| 5.8 | Canvas screenshot present in the docs page showing the `SpendGuardChatModelWrapper` card wired to a `ChatOpenAI` node. | Major |
| 5.9 | Docs page includes a "Coexists with other SpendGuard integrations?" Q&A noting Langflow + LangChain SDK integration share the same SDK code path and audit shape. | Minor |

## 3. Cross-cutting reviewer focus areas (every slice)

| Area | What to check | Severity if missed |
|------|---------------|--------------------|
| Backwards compatibility | Did the slice mutate `sdk/python/src/spendguard/integrations/langchain.py` or any file under `sdk/python/src/`? Did it edit existing demo modes' compose files? | Blocker |
| Type hints | All new public functions carry full hints. `from __future__ import annotations` used at top of each module. | Major |
| Logging | All `log.warning` / `log.info` callsites carry the `spendguard:` prefix matching the rest of the SDK. | Minor |
| Error messages | `ValueError` strings name the offending env var or canvas input. `RuntimeError` for unsupported call patterns name the version-bug remediation. | Major |
| Secret leakage | NO logging of `tenant_id`, `budget_id`, `window_instance_id` verbatim. NO logging of any env var name containing `KEY`/`SECRET`/`PASSWORD`/`TOKEN`. INV-6. | Blocker |
| Test isolation | Unit tests do NOT require Docker, do NOT require a running sidecar, do NOT make outbound HTTP. Use fake sidecar fixture + `FakeListChatModel`. | Blocker |
| Async / sync mixing | `build_model` is sync (Langflow contract). Inner async coordination uses `asyncio.run()` with explicit "no running loop" guard, NOT nested event loop hacks. | Blocker |
| Drop handles | Any new asyncio task / subprocess / fixture cleans up in `finally` or fixture teardown. | Major |
| Dependency surface | No new runtime dependency added beyond `spendguard-sdk[langchain]` and `langflow`. | Major |
| Symlink reuse | `_fake_sidecar.py` is a symlink to `sdk/python/tests/integrations/_fake_sidecar.py`, NOT a copy. | Major |
| Reuse of SDK contract | Auto-bind monkey-patches `SpendGuardChatModel._agenerate` only on the returned instance, NOT class-globally. INV-7 protection. | Blocker |

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

Likely D36 triggers:

- **Slice 2 auto-bind monkey-patch:** if Langflow flow execution sometimes invokes `_agenerate` from threads outside the event loop (e.g. background-task tools), the contextvar-based `_RUN_CONTEXT` won't propagate. Panel decides whether to thread-bind the run context or push for an explicit `run_id` canvas input.
- **Slice 2 sync/async bridge:** Langflow's `Component.build()` is sync but our build needs `await client.connect()`. If `asyncio.run()` inside a sync method conflicts with Langflow's executor (some Langflow paths already run inside a loop), panel decides between `nest_asyncio`, a thread executor, or pushing Langflow to expose an `async build_async()`.
- **Slice 3 metadata YAML schema:** Langflow 1.8 docs are sparse on the metadata YAML schema. If the in-the-wild schema differs from the design.md assumptions, panel decides whether to skip metadata YAML (rely on class-level introspection only) or wait on a Langflow docs PR.
- **Slice 4 demo footprint:** Langflow image is ~1 GB; if CI cell flake rate exceeds 10%, panel decides whether to mock Langflow's flow runner in the demo (regression in coverage) or accept the longer CI time.
- **Slice 5 PyPI Trusted Publisher setup:** if PyPI maintainer doesn't have the project preregistered for OIDC, panel decides whether to ship API-token auth as a v1 fallback (and migrate to Trusted Publisher in v1.0.1) or block on the manual PyPI registration step.

## 6. Slice-merge order is fixed

Per dependency in `implementation.md` §2: **Slice 1 → 2 → 3 → 4 → 5**, never reorder.

- Slice 2 depends on Slice 1's component skeleton + input declarations.
- Slice 3 depends on Slices 1 + 2 (component class is the artifact being packaged).
- Slice 4 depends on Slice 3 (demo needs the wheel-installable package OR the source mount, both of which are Slice 3 artifacts).
- Slice 5 depends on Slice 4 (docs reference the demo flow; publish workflow packages what Slice 3 produced).

## 7. Final reviewer override

If the reviewer believes the spec itself is wrong (e.g. composition vs subclassing of `SpendGuardChatModel`, auto-bind necessity, separate PyPI package vs SDK extra, metadata YAML format), flag it as a Blocker on the relevant slice with rationale referencing `design.md` §5 "Key decisions" — do not silently deviate. Spec changes route through Staff+ panel per build-plan §1.3.
