# COV_05 (D03) — D03 base-URL landing: DropInPicker Astro component

> **Deliverable**: D03 docs landing for drop-in install
> **Slice**: 3 of 7 (M)
> **Spec set**: [`docs/specs/coverage/D03_base_url_landing/`](../specs/coverage/D03_base_url_landing/)

## Scope

Convert the SLICE 1+2 `.md` landing at `docs/drop-in/` to `.mdx` and host a `<DropInPicker />` Astro component. The component lets readers filter the BYOK tool table by:
- "Has OpenAI-compatible BASE_URL" (filters to tools where simple env override works)
- "Needs forward HTTPS proxy + CA" (filters to closed-binary CLIs that need D02 install)
- "Search by tool name"

Per design §3 + §5: lands on `feat/astro-starlight-redesign` (PR #81) — the same Starlight site as SLICE 1+2.

Concretely:
- Convert `src/content/docs/docs/drop-in/index.md` → `index.mdx`
- NEW `src/components/DropInPicker.astro`:
  - Imports the drop-in tool list as JSON (or fronts a tools.json data file)
  - 3 filter controls: pattern radio (base_url / proxy / search), search input
  - Renders a filterable table with: Tool | Pattern | Install command | Verification snippet
  - SSR-rendered so it works without JS; client-side filtering progressively enhances
- NEW `src/data/dropin_tools.json` — single source of truth for the 14-tool table:
  - { "name": "Claude Code", "pattern": "proxy_and_ca", "install_cmd": "spendguard install", "env_var": "CLAUDE_PROXY" }
  - (entries should align with D02 SLICE 5's tools table — sync these so docs and CLI agree)
- Update `src/content/docs/docs/drop-in/index.mdx` to import + render `<DropInPicker />`
- ≥5 Playwright visual regression tests:
  - Default view (no filter)
  - Filter to "OpenAI-compatible BASE_URL" only
  - Filter to "Proxy + CA" only
  - Search "claude"
  - Mobile viewport (responsive shrink)

## Files touched

| File | Why |
|------|-----|
| `src/content/docs/docs/drop-in/index.md` → `.mdx` | Rename + content conversion |
| `src/components/DropInPicker.astro` | NEW — filterable table component |
| `src/data/dropin_tools.json` | NEW — 14-tool source of truth |
| `tests/visual/drop-in-picker.spec.ts` | NEW Playwright tests |

## Test/verification plan

1. `pnpm run build` (Astro/Starlight) clean
2. `pnpm run typecheck` clean
3. Playwright visual regression: 5 new tests pass; existing landing tests still pass
4. SSR works: view-source contains the full tool table even with JS disabled
5. Lighthouse a11y score ≥ 95 on the rendered page

## Anti-scope

- No D33/D34 per-tool recipe pages (those are separate deliverables)
- No backend search index (client-side filter only)
- No CLI sync automation (sync dropin_tools.json with D02 tools table manually for v0.1)

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D03_base_url_landing/design.md) §3 layout, §5 slice 3 row, §6 Astro component pattern
- SLICE 1: [`COV_03_d03_landing_skeleton.md`](COV_03_d03_landing_skeleton.md)
- SLICE 2: [`COV_04_d03_copy_polish.md`](COV_04_d03_copy_polish.md)
- D02 SLICE 5 14-tool table at `services/cli/src/tools/mod.rs::TOOL_OVERRIDES`
