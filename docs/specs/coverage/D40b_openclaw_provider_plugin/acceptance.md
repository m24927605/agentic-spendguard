# D40b - Acceptance Gates

Commands run from repo root unless noted.

## 1. Package gates

| Gate | Command | Pass condition |
|---|---|---|
| A1.1 | package install/typecheck command | exits 0 against the pinned OpenClaw version. |
| A1.2 | package unit test command | exits 0. |
| A1.3 | package build command | produces ESM output only. |
| A1.4 | package size command | `dist/index.js` <= 50 KB minified. |
| A1.5 | hash-reuse grep over `src/` and `dist/` | no local hash implementation tokens. |

## 2. Demo gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `make demo-down` | exit 0 before live demo rerun. |
| A2.2 | `make demo-up DEMO_MODE=openclaw_provider_plugin` | prints `[demo] openclaw_provider_plugin ALL 4 steps PASS (ALLOW + DENY + STREAM + PROVIDER_ERROR)`. |
| A2.3 | `make -C deploy/demo demo-verify-openclaw-provider-plugin` | SQL gate exits 0. |
| A2.4 | runner DENY assertion | counting-stub counter unchanged across DENY. |
| A2.5 | runner provider-error assertion | provider error propagates and failure commit row exists. |

## 3. Docs gates

| Gate | Command | Pass condition |
|---|---|---|
| A3.1 | `rg -n "in-process|not a sandbox" integrations/openclaw-provider-plugin README.md docs/site-v2/src/content/docs/docs/integrations/openclaw-provider-plugin.mdx` | trust-boundary warning present. |
| A3.2 | `rg -n "D40a|base-URL" docs/site-v2/src/content/docs/docs/integrations/openclaw-provider-plugin.mdx` | D40a fallback cross-link present. |
| A3.3 | docs-site build command | exits 0. |
| A3.4 | memory file exists | `project_coverage_d40b_shipped.md` exists after closeout. |

## 4. Ship checklist

- [ ] Every `OB-V*` marker pinned.
- [ ] Live demo run and SQL verify run recorded in commit message.
- [ ] No fail-open branch or env bypass.
- [ ] Reserve-time unit/pricing tuple reused on commits.
- [ ] D40a docs still describe base-URL coverage accurately.
