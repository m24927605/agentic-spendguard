# D33 — AnythingLLM Custom Base URL Recipe — `tests.md`

> Status: Doc + smoke spec; defines what must run green before any slice merges.
> Sibling docs: `design.md`, `implementation.md`, `acceptance.md`, `review-standards.md`.
> Audience: R1-R5 reviewer; CI maintainer; Technical Writer implementer.

---

## 1. Test surface

D33 is a hybrid: documentation page + Docker-based smoke. The pyramid:

| Layer | What it covers | Runs in |
|---|---|---|
| **L1 — Build** | Astro build succeeds; `dist/docs/drop-in/anythingllm/index.html` exists | Slice CI + local |
| **L2 — Parse** | Markdown / frontmatter valid; SQL parses; shell-script `bash -n` | Slice CI + local |
| **L3 — Link** | Internal anchors + upstream docs URL resolve | Slice CI |
| **L4 — Citation** | AnythingLLM "Custom OpenAI Base URL" upstream page contains the literal field names cited on D33 (`Base URL`, `API Key`, `Chat Model Name`) — snapshot captured under `citations/` | Slice CI + R1 reviewer manual check |
| **L5 — Smoke** | `DEMO_MODE=anythingllm_real make demo-up` boots stack, smoke prints OK, verify SQL exits 0 | Slice CI + local |
| **L6 — Cross-deliverable** | D03 row 10 `Verified` column reads `Live` after Slice 1 merge; D03 row 10 link target is the page D33 ships (not the stub) | Slice CI |
| **L7 — Manual smoke** | Reviewer opens the rendered page, copy-pastes Step 4's smoke command, confirms green | R1-R5 reviewer |

L1-L6 are hard gates on every slice. L7 is captured in the review log.

---

## 2. L1 + L2 — Build and parse

### 2.1 Astro build green

```bash
cd docs/site-v2 && npm run build
test -s docs/site-v2/dist/docs/drop-in/anythingllm/index.html
```

Both exit 0. A missing page file means the slug is wrong in the
frontmatter or the file was not saved into the content directory.

### 2.2 Shell + SQL parse

```bash
bash -n deploy/demo/anythingllm_smoke.sh
# SQL is exercised by L5; no separate parse step.
```

`bash -n` exits 0. The smoke script must not contain syntax errors at
slice CI time; a runtime failure is caught at L5.

### 2.3 Compose profile valid

```bash
cd deploy/demo
docker compose --profile anythingllm_real config >/dev/null
```

Exits 0. Catches typos in service / volume references.

---

## 3. L3 + L4 — Link and citation

### 3.1 Internal anchor + outbound link check

```bash
lychee docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md
```

Zero broken links. Specifically the upstream maintainer URL
`https://docs.anythingllm.com/llm-configuration/custom-openai-base-url`
must return 200 (or 301 → 200 within two hops).

### 3.2 Upstream citation verbatim check

The recipe page cites three exact upstream field names — **Base URL**,
**API Key**, **Chat Model Name** — that appear in the AnythingLLM
"Custom OpenAI Base URL" docs page. Slice 1 commits a PDF snapshot
of that upstream page under
`docs/specs/coverage/D33_anythingllm_recipe/citations/anythingllm-custom-openai-base-url.pdf`
and the R1 reviewer cross-checks it against the recipe page by hand.

A scripted check is added as a follow-up gate when AnythingLLM's docs
move to a machine-readable surface; until then the PDF + manual review
is the L4 gate. The slice cannot merge without the PDF in the citations
directory.

### 3.3 D03 row 10 link target check

```bash
test -s docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md
grep -q 'AnythingLLM' docs/site-v2/src/content/docs/docs/drop-in/index.md
# Verify the D03 matrix row 10 still points at /docs/drop-in/anythingllm/
grep -E 'AnythingLLM.*anythingllm' docs/site-v2/src/content/docs/docs/drop-in/index.md \
    | grep -q 'drop-in/anythingllm'
```

All three exit 0. Catches accidental rename of the D33 slug that would
break D03's row 10 link.

---

## 4. L5 — Smoke

### 4.1 Docker-compose bring-up + smoke

```bash
cd /repo
export OPENAI_API_KEY=sk-...        # real key, gpt-4o-mini access
make demo-up DEMO_MODE=anythingllm_real
```

Exits 0. Expected stdout includes:

```
[demo] DEMO_MODE=anythingllm_real → ...
[anythingllm-smoke] step 1: bootstrap account...
[anythingllm-smoke] step 2: configure provider → http://egress-proxy:9000/v1...
[anythingllm-smoke] step 3: create workspace...
[anythingllm-smoke] step 4: chat round-trip via SpendGuard...
[anythingllm-smoke]   chat OK
[anythingllm-smoke] step 5: verify reserve+commit in ledger...
[anythingllm-smoke] OK: reserve+commit verified
```

A failure at any step exits non-zero with a `FAIL:` prefix line.

### 4.2 Verification SQL standalone

```bash
psql "${POSTGRES_URL}" -v ON_ERROR_STOP=1 \
    -f deploy/demo/verify_step_anythingllm_real.sql
```

Exits 0 after step 4 of the smoke has run. Asserts:
- ≥ 1 row in `ledger_transactions` with `operation_kind = 'reserve'`
  within the last 10 minutes for tenant
  `00000000-0000-4000-8000-000000000001`.
- ≥ 1 row in `ledger_transactions` with `operation_kind = 'commit_estimated'`
  in the same window for the same tenant.

A missing row raises a `RAISE EXCEPTION` and exits non-zero. The SQL
mirrors the assertion pattern in `verify_step_litellm_real.sql`.

### 4.3 Idempotency / re-run

```bash
make demo-down DEMO_MODE=anythingllm_real
make demo-up DEMO_MODE=anythingllm_real
```

Second run is green. The `STORAGE_DIR` named volume is recreated on
each `demo-down`; the smoke's bootstrap step (`/api/setup-account`)
tolerates re-invocation (it 200s on first call and 4xx on second; the
script `|| true`s the call).

### 4.4 Flake budget

L5 has a known dependency on the real OpenAI API. The smoke has a 30-second
client timeout per call. Two retries on the chat call only (steps 1-3
are local + AnythingLLM internal and must pass first-try). A third
failure fails the slice.

---

## 5. L6 — Cross-deliverable

### 5.1 D03 row 10 `Verified` promotion

After Slice 1 merge:

```bash
grep -E '^\|.*AnythingLLM.*Live' docs/site-v2/src/content/docs/docs/drop-in/index.md
```

Exits 0 — confirms row 10 reads `Live` (not `Spec`). The promotion
ships in Slice 1's diff and is part of the slice's commit set.

### 5.2 D03 stub page no longer ships

```bash
# The page exists, but the content is no longer the stub from D03 SLICE 1.
! grep -q 'Recipe in progress' docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md
```

Exits 0. Catches a regression where the stub somehow got merged back over
the real recipe (e.g. a bad rebase).

---

## 6. L7 — Manual smoke

Captured in the R1 review log per `review-standards.md` §3. Includes:

- Reviewer opens the rendered page at 1280 px and confirms the four-field
  table is readable.
- Reviewer copy-pastes the Step 4 "Verify end-to-end" command into a
  clean clone and confirms it prints `OK: reserve+commit verified`
  within 90 seconds wall time.
- Reviewer confirms the AnythingLLM screenshots embedded in Slice 2 (if
  shipped) match the live UI of `mintplexlabs/anythingllm:1.8.4`.

---

## 7. Demo regression — anythingllm_real

D33 adds one new demo mode. The repo's existing demo regression workflow
(`make demo-up` matrix in CI) gains one entry: `DEMO_MODE=anythingllm_real`.
The mode requires `OPENAI_API_KEY` as a CI secret. If the secret is not
available in a CI run (e.g. fork PR), the mode is skipped with a CI
log line `[demo] DEMO_MODE=anythingllm_real skipped: OPENAI_API_KEY not set`.

The skip path does not fail the slice; it defers the L5 gate to the next
CI run with the secret available (typically the merge-to-main run).

---

## 8. CI workflow

Slice 1 adds one job to the existing `.github/workflows/docs-drop-in.yml`
(created by D03):

| Job | Steps | Hard gate |
|---|---|---|
| `d33-page-build` | Already covered by D03's `build` job (same Astro build runs both) | Yes |
| `d33-citation-snapshot` | `test -f docs/specs/coverage/D33_anythingllm_recipe/citations/anythingllm-custom-openai-base-url.pdf` | Yes |
| `d33-d03-parity` | `tests.md` §3.3 + §5.1 | Yes |
| `d33-smoke` | New job. Boots compose with `OPENAI_API_KEY` secret; runs `make demo-up DEMO_MODE=anythingllm_real`; asserts stdout contains `OK: reserve+commit verified` | Yes (skipped if secret missing) |

The smoke job runs only on PRs that touch:
- `deploy/demo/anythingllm_smoke.sh`
- `deploy/demo/verify_step_anythingllm_real.sql`
- `deploy/demo/compose.yaml`
- `deploy/demo/Makefile`
- `docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md`
- `docs/specs/coverage/D33_anythingllm_recipe/**`

Concurrency: the smoke job runs serially with other demo-mode jobs to
avoid Docker resource contention on the runner. Estimated wall time:
~3 minutes (image pull + bring-up + smoke + tear-down).
