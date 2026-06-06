# D34 — LobeChat Custom Base URL Recipe — `tests.md`

> Status: Doc + smoke spec; defines what must run green before any slice merges.
> Sibling docs: `design.md`, `implementation.md`, `acceptance.md`, `review-standards.md`.
> Audience: R1-R5 reviewer; CI maintainer; Technical Writer implementer.

---

## 1. Test surface

D34 is a hybrid: documentation page + Docker-based smoke. The pyramid:

| Layer | What it covers | Runs in |
|---|---|---|
| **L1 — Build** | Astro build succeeds; `dist/docs/drop-in/lobechat/index.html` exists | Slice CI + local |
| **L2 — Parse** | Markdown / frontmatter valid; SQL parses; shell-script `bash -n` | Slice CI + local |
| **L3 — Link** | Internal anchors + upstream docs URL resolve | Slice CI |
| **L4 — Citation** | LobeChat "Environment Variables" upstream page contains the literal env var name `OPENAI_PROXY_URL` and field labels (`API Proxy Address`) cited on D34 — snapshot captured under `citations/` | Slice CI + R1 reviewer manual check |
| **L5 — Smoke** | `DEMO_MODE=lobechat_real make demo-up` boots stack, smoke prints OK, verify SQL exits 0 | Slice CI + local |
| **L6 — Cross-deliverable** | D03 row 11 `Verified` column reads `Live` after Slice 1 merge; D03 row 11 link target is the page D34 ships (not the stub) | Slice CI |
| **L7 — Manual smoke** | Reviewer opens the rendered page, copy-pastes Step 4's smoke command, confirms green | R1-R5 reviewer |

L1-L6 are hard gates on every slice. L7 is captured in the review log.

---

## 2. L1 + L2 — Build and parse

### 2.1 Astro build green

```bash
cd docs/site-v2 && npm run build
test -s docs/site-v2/dist/docs/drop-in/lobechat/index.html
```

Both exit 0. A missing page file means the slug is wrong in the
frontmatter or the file was not saved into the content directory.

### 2.2 Shell + SQL parse

```bash
bash -n deploy/demo/lobechat_smoke.sh
# SQL is exercised by L5; no separate parse step.
```

`bash -n` exits 0.

### 2.3 Compose profile valid

```bash
cd deploy/demo
docker compose --profile lobechat_real config >/dev/null
```

Exits 0. Catches typos in service / volume references.

---

## 3. L3 + L4 — Link and citation

### 3.1 Internal anchor + outbound link check

```bash
lychee docs/site-v2/src/content/docs/docs/drop-in/lobechat.md
```

Zero broken links. Specifically the upstream maintainer URL
`https://lobehub.com/docs/self-hosting/environment-variables/model-provider`
must return 200 (or 301 → 200 within two hops). The fragment
`#openai_proxy_url` must resolve to an anchor on that page.

### 3.2 Upstream citation verbatim check

The recipe page cites two exact upstream identifiers — the env var
**`OPENAI_PROXY_URL`** and the client-mode UI field **`API Proxy Address`** —
that appear in the LobeChat self-hosting docs. Slice 1 commits a PDF
snapshot of those upstream pages under
`docs/specs/coverage/D34_lobechat_recipe/citations/lobechat-environment-variables.pdf`
and the R1 reviewer cross-checks it against the recipe page by hand.

A scripted check is added as a follow-up gate when LobeChat's docs move
to a machine-readable surface; until then the PDF + manual review is
the L4 gate. The slice cannot merge without the PDF in the citations
directory.

### 3.3 D03 row 11 link target check

```bash
test -s docs/site-v2/src/content/docs/docs/drop-in/lobechat.md
grep -q 'LobeChat' docs/site-v2/src/content/docs/docs/drop-in/index.md
# Verify the D03 matrix row 11 still points at /docs/drop-in/lobechat/
grep -E 'LobeChat.*lobechat' docs/site-v2/src/content/docs/docs/drop-in/index.md \
    | grep -q 'drop-in/lobechat'
```

All three exit 0. Catches accidental rename of the D34 slug that would
break D03's row 11 link.

---

## 4. L5 — Smoke

### 4.1 Docker-compose bring-up + smoke

```bash
cd /repo
export OPENAI_API_KEY=sk-...        # real key, gpt-4o-mini access
make demo-up DEMO_MODE=lobechat_real
```

Exits 0. Expected stdout includes:

```
[demo] DEMO_MODE=lobechat_real → ...
[lobechat-smoke] step 1: confirm OPENAI_PROXY_URL on the container...
[lobechat-smoke] step 2: chat round-trip via SpendGuard...
[lobechat-smoke]   chat OK
[lobechat-smoke] step 3: verify reserve+commit in ledger...
[lobechat-smoke] OK: reserve+commit verified
```

A failure at any step exits non-zero with a `FAIL:` prefix line.

### 4.2 Verification SQL standalone

```bash
psql "${POSTGRES_URL}" -v ON_ERROR_STOP=1 \
    -f deploy/demo/verify_step_lobechat_real.sql
```

Exits 0 after step 2 of the smoke has run. Asserts:
- ≥ 1 row in `ledger_transactions` with `operation_kind = 'reserve'`
  within the last 10 minutes for tenant
  `00000000-0000-4000-8000-000000000001`.
- ≥ 1 row in `ledger_transactions` with `operation_kind = 'commit_estimated'`
  in the same window for the same tenant.

A missing row raises a `RAISE EXCEPTION` and exits non-zero. The SQL
mirrors the assertion pattern in `verify_step_anythingllm_real.sql` and
`verify_step_litellm_real.sql`.

### 4.3 Idempotency / re-run

```bash
make demo-down DEMO_MODE=lobechat_real
make demo-up DEMO_MODE=lobechat_real
```

Second run is green. LobeChat server mode boots stateless when no
volume is mounted; the smoke's chat path is independent of any prior
conversation.

### 4.4 Flake budget

L5 has a known dependency on the real OpenAI API. The smoke has a
30-second client timeout per call. Two retries on the chat call only
(steps 0-1 are local + LobeChat internal and must pass first-try). A
third failure fails the slice.

### 4.5 Negative test — env var must be set

```bash
# Boot without OPENAI_PROXY_URL: smoke must fail-closed.
docker run -d --name lobe-noproxy -p 3211:3210 \
    -e OPENAI_API_KEY=sk-bogus \
    -e ACCESS_CODE=test \
    lobehub/lobe-chat:1.40.0
sleep 5
# Manually POST a chat → expect LobeChat to call api.openai.com directly.
# Verify ledger has NO new row in the last 30 seconds.
```

Optional reviewer-run scenario. Documented for the recipe's "gotchas"
section as the failure mode the env var prevents. Not run in CI.

---

## 5. L6 — Cross-deliverable

### 5.1 D03 row 11 `Verified` reads `Live`

After Slice 1 merge:

```bash
grep -E '^\|.*LobeChat.*Live' docs/site-v2/src/content/docs/docs/drop-in/index.md
```

Exits 0 — confirms row 11 reads `Live`. Per D03 `design.md` §3.2,
row 11 is already `Live` at D03 ship time conditionally on D34 smoke
green; D34's slice closes that conditional.

### 5.2 D03 stub page no longer ships

```bash
# The page exists, but the content is no longer the stub from D03 SLICE 1.
! grep -q 'Recipe in progress' docs/site-v2/src/content/docs/docs/drop-in/lobechat.md
```

Exits 0. Catches a regression where the stub somehow got merged back over
the real recipe (e.g. a bad rebase).

### 5.3 D33 + D34 do not collide in compose

```bash
cd deploy/demo
docker compose --profile anythingllm_real --profile lobechat_real config >/dev/null
```

Exits 0. Catches a regression where D33 and D34 share a port or volume
name (D33 binds `127.0.0.1:3001`, D34 binds `127.0.0.1:3210` — must stay
disjoint; D33's `anythingllm-storage` volume must not collide with any
D34 volume).

---

## 6. L7 — Manual smoke

Captured in the R1 review log per `review-standards.md` §3. Includes:

- Reviewer opens the rendered page at 1280 px and confirms the env-var
  snippet in Step 1 is copy-pasteable as-is.
- Reviewer copy-pastes the Step 4 "Verify end-to-end" command into a
  clean clone and confirms it prints `OK: reserve+commit verified`
  within 90 seconds wall time.
- Reviewer confirms the LobeChat screenshot embedded in Slice 2 (if
  shipped) matches the live UI of `lobehub/lobe-chat:1.40.0`.

---

## 7. Demo regression — lobechat_real

D34 adds one new demo mode. The repo's existing demo regression workflow
(`make demo-up` matrix in CI) gains one entry: `DEMO_MODE=lobechat_real`.
The mode requires `OPENAI_API_KEY` as a CI secret. If the secret is not
available in a CI run (e.g. fork PR), the mode is skipped with a CI
log line `[demo] DEMO_MODE=lobechat_real skipped: OPENAI_API_KEY not set`.

The skip path does not fail the slice; it defers the L5 gate to the
next CI run with the secret available (typically the merge-to-main run).

---

## 8. CI workflow

Slice 1 adds one job to the existing `.github/workflows/docs-drop-in.yml`
(created by D03):

| Job | Steps | Hard gate |
|---|---|---|
| `d34-page-build` | Already covered by D03's `build` job (same Astro build runs both) | Yes |
| `d34-citation-snapshot` | `test -f docs/specs/coverage/D34_lobechat_recipe/citations/lobechat-environment-variables.pdf` | Yes |
| `d34-d03-parity` | `tests.md` §3.3 + §5.1 | Yes |
| `d34-smoke` | New job. Boots compose with `OPENAI_API_KEY` secret; runs `make demo-up DEMO_MODE=lobechat_real`; asserts stdout contains `OK: reserve+commit verified` | Yes (skipped if secret missing) |
| `d33-d34-coexistence` | `tests.md` §5.3 — both profiles compose-config together | Yes |

The smoke job runs only on PRs that touch:
- `deploy/demo/lobechat_smoke.sh`
- `deploy/demo/verify_step_lobechat_real.sql`
- `deploy/demo/compose.yaml`
- `deploy/demo/Makefile`
- `docs/site-v2/src/content/docs/docs/drop-in/lobechat.md`
- `docs/specs/coverage/D34_lobechat_recipe/**`

Concurrency: the smoke job runs serially with other demo-mode jobs to
avoid Docker resource contention on the runner. Estimated wall time:
~3 minutes (image pull + bring-up + smoke + tear-down).
