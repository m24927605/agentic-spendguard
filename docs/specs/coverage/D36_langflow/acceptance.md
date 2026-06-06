# D36 — Acceptance

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`tests.md`](tests.md), [`review-standards.md`](review-standards.md).

D36 ships when **every gate** in §1 is green AND the §2 invariants are unbreakable AND the §3 ship checklist is fully checked. Per build-plan §3 "100% feasible" rule: every gate below is runnable in the repo's current state, no third-party action required (PyPI publish is gated behind tag-only workflow + OIDC, not a hard ship gate).

## 1. Hard gates

### G1 — Package imports cleanly

```bash
cd plugins/langflow && python -c "from spendguard_langflow import SpendGuardChatModelWrapper; print(SpendGuardChatModelWrapper.__name__)"
```

Expected: prints `SpendGuardChatModelWrapper`. No `ImportError`.

### G2 — Component metadata introspection

```bash
cd plugins/langflow && python -c "from spendguard_langflow.component import SpendGuardChatModelWrapper as C; \
  assert C.display_name == 'SpendGuard Budget Gate'; \
  assert C.icon == 'shield'; \
  assert len(C.inputs) == 8; \
  assert len(C.outputs) == 1; \
  assert C.outputs[0].types == ['LanguageModel']; \
  print('OK')"
```

Expected: prints `OK`. Component schema lines up with design.md §2 / implementation.md Slice 1.

### G3 — Unit suite

```bash
cd plugins/langflow && pytest tests/test_component_skeleton.py tests/test_build_model.py tests/test_run_context_autobind.py tests/test_install_script.py -v
```

Expected: 25 tests pass (count from `tests.md` §2; final count may rise during implementation but never fall below 25).

### G4 — Wheel build

```bash
cd plugins/langflow && python -m build --wheel -o /tmp/langflow_wheel/
```

Expected: produces `spendguard_langflow_component-0.1.0-py3-none-any.whl`. Wheel size < 500 KB (pure Python, no vendored assets).

### G5 — Wheel contains metadata YAML

```bash
unzip -l /tmp/langflow_wheel/spendguard_langflow_component-0.1.0-py3-none-any.whl | grep -E 'spendguard_chat_model_wrapper\.yaml'
```

Expected: exactly one match. The metadata YAML must ship inside the wheel so Langflow's component loader can pick it up after `pip install`.

### G6 — Install script roundtrip

```bash
TMPDIR=$(mktemp -d) && cd plugins/langflow && \
  pip install --quiet -e . && \
  spendguard-langflow-install --target $TMPDIR && \
  test -f $TMPDIR/spendguard_chat_model_wrapper.py && \
  test -f $TMPDIR/spendguard_chat_model_wrapper.yaml && echo OK
```

Expected: prints `OK`. Both files land at the target.

### G7 — Existing demos still pass (regression)

```bash
make demo-down
make demo-up DEMO_MODE=decision
make demo-down
make demo-up DEMO_MODE=default
```

Expected: both demos still exit 0. D36 is purely additive — the existing demo modes' compose files were not edited (Slice 4 lives in an overlay file).

### G8 — Langflow demo boots and passes

```bash
make demo-down       # clean slate
make demo-up DEMO_MODE=langflow_real
```

Expected:

- All compose services (Langflow + SpendGuard sidecar + Postgres + ledger + canonical ingest + outbox forwarder) reach healthy.
- Demo driver exits 0.
- stdout contains `[demo] langflow_real ALL 3 steps PASS (ALLOW + DENY + STREAM)`.
- stdout contains `D36_LANGFLOW OK: langflow decisions=N` for `N >= 2`.
- stdout contains the outbox-closure verification line.

### G9 — Demo mode tear-down clean

```bash
make demo-down
```

Expected: no orphaned containers, no orphaned volumes, exit 0.

### G10 — Public docs page renders

```bash
cd docs/site && npm run build
```

Expected: build succeeds. `docs/site/dist/docs/integrations/langflow/index.html` exists. Page contains:

- "Install (PyPI)" section with `pip install spendguard-langflow-component && spendguard-langflow-install --target $LANGFLOW_COMPONENTS_PATH` command.
- "Install (vendor-drop)" section with the manual copy snippet for air-gapped setups.
- Canvas screenshot showing the `SpendGuardChatModelWrapper` card wired to a `ChatOpenAI` node.
- Decision matrix comparing the Langflow component path vs the egress-proxy path vs a Langflow-side global-provider config (deferred, mentioned for awareness).
- "Limitations" section explicitly listing: no embeddings gate, no token-by-token mid-stream cap, global-provider config (v1.8+) interception deferred to v1.1, no Langflow Cloud marketplace push.

### G11 — README index entry present

```bash
grep -F "Langflow custom component" README.md
```

Expected: exactly one row in the adapter integrations table that includes the `pip install spendguard-langflow-component` command.

### G12 — Publish workflow lints clean

```bash
gh workflow view langflow-component-publish.yml --yaml | yq '.jobs' >/dev/null
actionlint .github/workflows/langflow-component-publish.yml
```

Expected: no lint errors. Workflow runs only on `langflow-component-v*` tags; not on PR CI. PyPI Trusted Publisher OIDC step is conditional on the tag pattern only.

### G13 — No proto / no DB-schema / no Rust changes (purely additive)

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.(proto|rs)$' && echo FAIL || echo OK
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '\.sql$' | grep -v -E '^deploy/demo/(verify_step_langflow\.sql|langflow/.*\.sql)$' && echo FAIL || echo OK
```

Expected: both lines print `OK`. The only new SQL files are the demo verify gate and any Langflow seed SQL — both in the demo allow-list.

### G14 — No `spendguard-sdk` mutations

```bash
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '^sdk/python/src/spendguard/integrations/langchain\.py$' && echo FAIL || echo OK
git diff --name-only $(git merge-base main HEAD)..HEAD | grep -E '^sdk/python/src/spendguard/' && echo FAIL || echo OK
```

Expected: both lines print `OK`. D36 lives entirely under `plugins/langflow/` + `deploy/demo/` + `docs/site/` + `.github/workflows/` + `README.md`. The SDK is untouched.

## 2. Invariants (must never regress)

| ID | Invariant | How verified |
|----|-----------|--------------|
| INV-1 | **DENY never hits the upstream provider.** Counting stub MUST register zero hits across all DENY decisions in the demo. Inner `_agenerate` MUST NOT fire when SDK raises `DecisionDenied`. | `verify_step_langflow.sql` `stub_hits` assertion + B09 + demo step 2 |
| INV-2 | **Pre-call reservation precedes inner LLM call.** `request_decision` fires before the wrapped `BaseChatModel._agenerate`. | B08 strict-ordering `asyncio.Event` |
| INV-3 | **Auto-bind never clobbers caller's `run_context`.** When the caller has wrapped in `run_context(...)`, the SDK-recorded `run_id` is the caller's, not the auto-bound one. | A02 |
| INV-4 | **`build_model` is idempotent across canvas re-renders.** Calling `build_model()` twice yields two distinct `SpendGuardChatModel` instances; neither references the other's client. | Implicit in B01 + B02; reviewer checks for module-level state. |
| INV-5 | **Streaming end-of-stream commit fires when upstream provides usage.** Same contract as existing `langchain.py` integration. | demo step 3 + SQL `stream=true` assertion |
| INV-6 | **No mutation of operator credentials in logs.** No log line contains `tenant_id`, `budget_id`, or sidecar UDS path verbatim (path-shape OK, secret-shape not). | Reviewer checklist + manual log scrape in demo |
| INV-7 | **No edits to `sdk/python/src/spendguard/integrations/langchain.py`.** D36 is a packaging layer. | G14 |
| INV-8 | **Install script refuses to write outside operator-controlled paths.** Targeting `/usr`, `/etc`, `/System`, `/bin` → refused. | I04 |
| INV-9 | **No Rust/proto/SDK drift.** D36 is purely additive at the plugin tree. | G13 + G14 |

## 3. Ship checklist

```
[ ] G1 package imports clean
[ ] G2 component metadata introspection clean
[ ] G3 unit suite (>=25 tests) passes
[ ] G4 wheel builds, < 500 KB
[ ] G5 wheel contains metadata YAML
[ ] G6 install script roundtrip OK
[ ] G7 existing demo modes unbroken (decision + default)
[ ] G8 `make demo-up DEMO_MODE=langflow_real` exits 0 + success lines printed
[ ] G9 `make demo-down` clean
[ ] G10 docs/site build succeeds + new langflow page renders + 5 sections present
[ ] G11 README adapter row landed
[ ] G12 publish workflow lint clean
[ ] G13 no proto / Rust / SQL drift outside allow-list
[ ] G14 no spendguard-sdk mutations
[ ] INV-1 .. INV-9 all green
[ ] All 5 slices merged in order S1 → S5 with R1-R5 review loop per build-plan §1.1
[ ] Memory write-back entry `project_coverage_D36_shipped.md` drafted per build-plan §8
```

## 4. Definition of done (per build-plan §7)

- All 5 slices merged into main.
- Acceptance gates G1..G14 + invariants INV-1..INV-9 green.
- README adapter row landed.
- `docs/site/docs/integrations/langflow.md` live.
- `Makefile` `DEMO_MODE=langflow_real` branch live.
- `langflow-component-publish.yml` workflow exists (PyPI publish is a tag-driven follow-up, not a ship gate).
- Memory entry written per build-plan §8.

## 5. Out-of-scope explicit declarations

D36 does NOT close any of:

- Embeddings / RAG retrieval gating — out of scope; budget gate fires only at LLM call boundary.
- Tool-call cost gating on Langflow canvas tool nodes — future component slot, not v1.
- Token-by-token cap mid-stream — end-of-stream only.
- Global model-provider config interception (Langflow v1.8+) — explicitly deferred to v1.1; per-node wrap is v1 surface.
- Langflow Cloud (DataStax-hosted) marketplace push automation. PyPI is the install surface; Cloud push is follow-up.
- Per-flow budget IDs read from flow metadata — v1 reads from canvas component inputs only.

These are documented in `docs/site/docs/integrations/langflow.md` "Limitations" section so operator expectation matches the ship surface.

## 6. Risk register

| Risk | Mitigation |
|------|-----------|
| Langflow SDK 1.8 → 1.9 breaking change between spec-write and ship | Pin `langflow>=1.8.0,<2.0.0` in `pyproject.toml`. CI matrix includes the latest 1.8.x patch; 1.9.x release will require a follow-up issue, not a v1 blocker. |
| Self-hosted Langflow image (`langflowai/langflow:1.8`) compose drift | Pin image digest in the overlay compose. CI fails on digest mismatch. |
| Langflow's `Component` base class internal API changes break the auto-bind monkey-patch | Auto-bind only patches the returned `SpendGuardChatModel._agenerate` (SDK-side surface), NOT Langflow internals. Risk localized. |
| `inner` HandleInput type name changes from `LanguageModel` to something else | Reviewer cross-checks against shipping Langflow 1.8 source on each spec rev; pinned in test C03. |
| Operator misconfigures sidecar UDS path | Build raises `ValueError` with explicit message naming both canvas input and env-var fallback. B05 pins. |
| Demo stack adds ~1 GB Langflow image to CI cell | Compose layer cache; `DEMO_MODE=langflow_real` opt-in only, not part of default `make demo-up`. |
| Caller writes code-driven Langflow flow that already binds `run_context` | INV-3 — caller's context always wins. A02 pins. |
| PyPI publishing OIDC trust gets misconfigured | Workflow gate `gh workflow view` lint; tag-only trigger; manual approval not required because PyPI Trusted Publisher already enforces the workflow identity. |
