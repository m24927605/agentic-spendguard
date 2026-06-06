# D30 — Anthropic claude-agent-sdk Egress-Proxy Install Recipe — `tests.md`

> Status: Doc + smoke-test spec; defines what must run green before any slice merges.
> Sibling docs: `design.md`, `implementation.md`, `acceptance.md`, `review-standards.md`.
> Audience: R1-R5 reviewer; CI maintainer; Technical Writer implementer.

---

## 1. Test surface

D30 mixes documentation and a thin end-to-end smoke. There is no SpendGuard code change; verification is at the integration boundary. The pyramid is:

| Layer | What it covers | Runs in |
|---|---|---|
| **L1 — Build** | mkdocs site builds; example pyproject + package.json parse | Slice CI + local |
| **L2 — Parse** | Markdown valid; YAML / TOML frontmatter and config valid | Slice CI + local |
| **L3 — Link** | Internal page links + external citations resolve | Slice CI |
| **L4 — Smoke (Python)** | `DEMO_MODE=agent_real_claude_agent_sdk_egress` runs green end-to-end | Slice CI (Slice 1+) |
| **L5 — Smoke (TS)** | `node examples/claude-agent-sdk-egress/typescript/smoke.mjs` runs green | Slice CI (Slice 2+) |
| **L6 — Audit assertion** | `verify_step_claude_agent_sdk_egress.sql` passes after each smoke | Slice CI (Slice 1+) |
| **L7 — Manual smoke** | Reviewer reads the rendered page on desktop, runs the Python smoke locally | R1-R5 reviewer |

L1-L3 are hard gates on every slice. L4 + L6 are hard from Slice 1; L5 + L6 are hard from Slice 2. L7 is captured in the review log per `review-standards.md` §3.

## 2. L1 + L2 — Build and parse

### 2.1 mkdocs build green

```bash
cd docs/site && pip install -r requirements.txt && mkdocs build --strict
```

Exits 0. `--strict` promotes warnings to errors so a missing nav entry or broken intra-site link breaks the build immediately.

### 2.2 Page route present

```bash
test -s docs/site/site/integrations/claude-agent-sdk/index.html
```

Returns 0 after build. A missing file means the page was not added to `docs/site/mkdocs.yml`'s `nav`; both fail the slice.

### 2.3 Example metadata files parse

```bash
python -c "import tomllib; tomllib.loads(open('examples/claude-agent-sdk-egress/python/pyproject.toml').read())"
node -e "JSON.parse(require('fs').readFileSync('examples/claude-agent-sdk-egress/typescript/package.json'))"
```

Both exit 0. The TS check is Slice-2-only.

## 3. L3 — Link

### 3.1 Internal anchor + cross-page

`lychee` runs against the doc page and the two example READMEs:

```bash
lychee docs/site/docs/integrations/claude-agent-sdk.md \
       examples/claude-agent-sdk-egress/README.md \
       examples/claude-agent-sdk-egress/python/pyproject.toml
```

Exits 0; all internal `./install/` / `./drop-in/` etc. links resolve against the built `site/` tree; all external links return 200 (3xx redirects up to 2 hops allowed).

### 3.2 Cited upstream pages

Per `design.md` D-1 there is exactly one upstream citation surface to check:

| Cited page | Literal string verified |
|---|---|
| `https://docs.claude.com/en/docs/claude-code/sdk/overview` | "PreToolUse" (confirms the hook name our warning references) |
| `https://pypi.org/project/claude-agent-sdk/` | "claude-agent-sdk" (confirms the PyPI package name) |
| `https://www.npmjs.com/package/@anthropic-ai/claude-agent-sdk` | "@anthropic-ai/claude-agent-sdk" |

A simple curl-then-grep step in CI. Any 4xx / 5xx / missing-string blocks the slice.

## 4. L4 — Python smoke

### 4.1 Local invocation

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cd deploy/demo && DEMO_MODE=agent_real_claude_agent_sdk_egress make demo-up
```

PASS criteria:

- `demo-up` exits 0.
- The Python smoke script (`run_demo.py` arm per `implementation.md` §7) prints `[smoke] PASS`.
- The verify SQL (§5 below) exits 0.

A non-zero exit at any step fails the slice. The smoke is allowed up to 2 transient-network retries on the live `api.anthropic.com` call, after which it is a hard fail.

### 4.2 CI invocation

The `python-smoke` job in `.github/workflows/d30-claude-agent-sdk-smoke.yml` (added Slice 2; preview-only in Slice 1):

```yaml
- run: cd deploy/demo && make demo-up
  env:
    DEMO_MODE: agent_real_claude_agent_sdk_egress
    ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY_CI }}
```

Slice 1 ships the Makefile arm and the dispatcher arm but does not require the CI workflow to exist yet (the Makefile change is locally testable). Slice 2 adds the CI gate.

## 5. L5 — TS smoke

### 5.1 Local invocation

```bash
cd examples/claude-agent-sdk-egress/typescript
npm ci
export ANTHROPIC_API_KEY=sk-ant-...
export HTTPS_PROXY=http://localhost:9000
export NODE_EXTRA_CA_CERTS="$HOME/.local/share/spendguard/ca/spendguard-root.crt"
node smoke.mjs
```

PASS criteria identical to L4: smoke prints `[smoke] PASS` and `verify_audit.py` exits 0.

### 5.2 CI invocation

The `typescript-smoke` job in `.github/workflows/d30-claude-agent-sdk-smoke.yml` brings up a `DEMO_MODE=proxy` stack (no demo container), then runs the TS smoke from the host runner with `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS` pointed at the compose-exposed proxy + the CA the proxy advertises on `/internal/ca.crt`.

## 6. L6 — Audit-chain assertion (`verify_step_claude_agent_sdk_egress.sql`)

The SQL file lives at `deploy/demo/verify_step_claude_agent_sdk_egress.sql`. It asserts inside a `DO $$ … $$` block, raising `EXCEPTION` on any miss:

1. Exactly one row in `audit_outbox` with `event_type = 'RESERVE_RESPONSE'`, `provider = 'anthropic'`, `model LIKE 'claude-%'`, written in the last 5 minutes.
2. Exactly one row with `event_type = 'COMMIT_OUTCOME'`, same `request_id` as (1), `provider = 'anthropic'`, `committed_input_tokens > 0`, `committed_output_tokens > 0`.
3. No `RESERVE_RESPONSE` row with `decision = 'STOP'` for the same `request_id` (the canonical prompt is well under any sane cap; STOP would imply mis-configuration).

The SQL mirrors `deploy/demo/verify_step8.sql`'s shape. A failing assertion exits non-zero and the Makefile target fails the slice.

## 7. L7 — Manual smoke

Captured in the R1 review log per `review-standards.md` §3.6. Includes:

- Read the rendered `claude-agent-sdk` page end-to-end on desktop (1280 px). Confirm the PreToolUse warning is above the fold and visually distinct (callout or admonition block).
- Read the page on mobile (375 px). Confirm code blocks scroll horizontally rather than overflow.
- Run the Python smoke locally per §4.1 once. Record exit code and the audit-chain query output in the review log.
- For Slice 2, run the TS smoke per §5.1 once. Record exit code and any Node-version-specific output.
- Confirm the troubleshooting section's three bullets each map to a real error the reviewer can reproduce by disabling the suggested env var.

## 8. Demo regression

D30 adds one new demo mode (`agent_real_claude_agent_sdk_egress`). The existing demo modes (`agent`, `decision`, `proxy`, `cost_advisor`, `approval`, `multi_provider_usd`, `litellm_real`, `agent_real_openai_agents_proxy`, etc.) MUST continue to pass at slice merge time.

Slice 1 runs:

```bash
cd deploy/demo
for MODE in decision proxy agent_real_openai_agents_proxy; do
    DEMO_MODE=$MODE make demo-up && DEMO_MODE=$MODE make demo-down
done
```

All three exit 0. A regression on any of these three is a P0 block; the new mode change must not have touched their wiring. Other demo modes are validated by their own CI workflows and are not D30's regression surface.

## 9. CI workflow summary

Added by Slice 2 — `.github/workflows/d30-claude-agent-sdk-smoke.yml`:

| Job | Steps | Hard gate |
|---|---|---|
| `mkdocs-build` | `mkdocs build --strict` from `docs/site` | Yes |
| `link-check` | lychee per §3.1 | Yes |
| `python-smoke` | `make demo-up` with the new DEMO_MODE | Yes (Slice 2+) |
| `typescript-smoke` | `DEMO_MODE=proxy` + `node smoke.mjs` | Yes (Slice 2+) |
| `audit-verify` | `psql -f verify_step_claude_agent_sdk_egress.sql` | Yes |

Trigger: `pull_request` on `main`, paths-filter on:

- `docs/site/docs/integrations/claude-agent-sdk.md`
- `examples/claude-agent-sdk-egress/**`
- `deploy/demo/verify_step_claude_agent_sdk_egress.sql`
- `deploy/demo/Makefile`
- `deploy/demo/demo/run_demo.py`
- `.github/workflows/d30-claude-agent-sdk-smoke.yml`

Concurrency: runs in parallel with the rest of repo CI; the new DEMO_MODE is mutually exclusive with no other workflow so there is no shared-state collision.

## 10. Flake budget

Each smoke is allowed 2 retries on a single transient `api.anthropic.com` network error per CI run. A third failure within the same run is a hard fail. This matches the existing `agent_real_openai_agents_proxy` flake budget (it also hits a real provider).

Citation link-check retries: 2 with exponential backoff per the existing repo CI convention.

No flake budget on the SQL verification — the database is local to compose, no external dependency.
