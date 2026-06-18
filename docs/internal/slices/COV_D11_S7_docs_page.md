# COV_D11_S7 — D11 LiteLLM proxy plugin: docs page

> **Deliverable**: D11 LiteLLM `async_pre_call_hook` proxy guardrail plugin
> **Slice**: 7 of 7 (S)
> **Spec set**: [`docs/specs/coverage/D11_litellm_proxy_plugin/`](../../specs/coverage/D11_litellm_proxy_plugin/)

## Scope

Land the operator-facing docs page on the Starlight site explaining how to install + configure the SpendGuard LiteLLM proxy guardrail. References the LOCKED surface: `litellm-guardrail` extra name; `spendguard.integrations.litellm_guardrail.spendguard_guardrail_factory` dotted module path; 9-var env table (SLICE 4 5-var subset + SLICE 4b 4-var binding); proxy_config.yaml stanza; demo gate.

Concretely:
- `docs/site-v2/src/content/docs/docs/integrations/litellm-proxy.mdx` — NEW:
  - H1: "LiteLLM proxy guardrail"
  - H2 Install — `pip install 'spendguard-sdk[litellm-guardrail]'` (canonical extras per SLICE 5 R1 deviation #1)
  - H2 Quick start — minimal proxy_config.yaml stanza using the SLICE 5 canonical dot-path
  - H2 Configuration — 9-var env table + 13-var inline-yaml table per SLICE 4 + 4b + 5
  - H2 Limitations — INV-5 end-of-stream commit + no token-by-token cap (review-standards §7.3 Blocker requirement)
  - H2 Demo — points at `make demo-up DEMO_MODE=litellm_guardrail`
  - H2 Troubleshooting — common ConfigError messages + recovery hints
- `docs/site-v2/astro.config.mjs` — sidebar entry for the new docs page (if needed)
- README adapter integrations table — new row for LiteLLM proxy guardrail per review-standards §7.4
- ≥3 Playwright visual regression tests:
  - Page renders 200
  - Install + Quick start sections visible
  - INV-5 limitations section visible (the LOCKED §7.3 Blocker check — page MUST disclose this)
- Update D11 review-standards.md §7 SLICE-PHASING note: SLICE 7 ships ✓ (closes the docs deferral)

## Files touched

| File | Why |
|------|-----|
| `docs/site-v2/src/content/docs/docs/integrations/litellm-proxy.mdx` | NEW |
| `docs/site-v2/astro.config.mjs` (if sidebar config lives here) | sidebar entry |
| `README.md` (root, if it has an integrations table) | new row |
| `docs/site-v2/tests/visual/litellm-proxy-docs.spec.ts` | NEW Playwright tests |
| `docs/specs/coverage/D11_litellm_proxy_plugin/review-standards.md` | §7 SLICE-PHASING ✓ |

## Test/verification plan

1. `npm run build` (Astro) clean
2. `npx astro check` 0/0/0
3. Playwright: 3 new tests pass; SLICE 2 + 3 existing tests still pass
4. SSR sanity: dist HTML contains LOCKED strings ("litellm-guardrail", "spendguard_guardrail_factory", "INV-5 end-of-stream commit", "no token-by-token cap")
5. Sidebar nav from main docs landing reaches /docs/integrations/litellm-proxy/

## Anti-scope

- No SDK code changes (SLICE 1-5 own the runtime)
- No demo wiring changes (SLICE 6 owns)
- No new hook bodies

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D11_litellm_proxy_plugin/design.md) §7 docs, §1 LOCKED surfaces, [`review-standards.md`](../../specs/coverage/D11_litellm_proxy_plugin/review-standards.md) §7.3 Blocker (INV-5 disclosure), §7.4 README row
- SLICE 5: [`COV_D11_S5_proxy_config_entry.md`](COV_D11_S5_proxy_config_entry.md)
- SLICE 6: [`COV_D11_S6_demo_mode.md`](COV_D11_S6_demo_mode.md)
