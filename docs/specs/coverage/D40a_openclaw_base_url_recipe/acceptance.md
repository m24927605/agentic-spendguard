# D40a - Acceptance Gates

Commands run from repo root unless noted.

## 1. Slice 1 - recipe and smoke

| Gate | Command | Pass condition |
|---|---|---|
| A1.1 | `make demo-down` | exit 0 before every demo rerun. |
| A1.2 | `make demo-up DEMO_MODE=openclaw_base_url` | exit 0; locked success line printed. |
| A1.3 | `make -C deploy/demo demo-verify-openclaw-base-url` | SQL gate exits 0 with `COV_D40A_GATE` labels. |
| A1.4 | `rg -n "http://localhost:9000/v1|http://egress-proxy:9000/v1" docs/site-v2/src/content/docs/docs/drop-in/openclaw.mdx examples/openclaw-base-url` | exact proxy URL documented. |
| A1.5 | `rg -n "provider plugin" docs/site-v2/src/content/docs/docs/drop-in/openclaw.mdx` | only appears in anti-scope or D40b cross-link wording; no plugin coverage claim. |
| A1.6 | `git diff --stat -- sdk/fixtures/cross-language` | empty. |

## 2. Slice 2 - docs publish and closeout

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `rg -n "OpenClaw" README.md` | table row exists and says base-URL recipe. |
| A2.2 | `rg -n "D40a|OpenClaw" CHANGELOG.md` | D40a entry exists. |
| A2.3 | repo docs-site build command | exits 0. |
| A2.4 | `test -f /Users/michael.chen/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/project_coverage_d40a_shipped.md` | memory entry exists after closeout. |

## 3. Ship checklist

- [ ] `OA-V1`..`OA-V5` pinned in slice docs.
- [ ] Live demo run included in commit message with the locked success line.
- [ ] SQL verify gate physically run.
- [ ] No OpenClaw plugin claims in D40a docs.
- [ ] README and CHANGELOG updated.
