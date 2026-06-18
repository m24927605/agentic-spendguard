# COV_04 — D03 base-URL landing: copy polish + README sync + screenshots

> **Deliverable**: D03 OPENAI_BASE_URL drop-in landing page
> **Slice**: 2 of 2-3 (Slice 2)
> **Spec set**: [`docs/specs/coverage/D03_base_url_landing/`](../../specs/coverage/D03_base_url_landing/)

## Scope

Polish the SLICE 1 skeleton: copy refinement for the H1 (capture `spendguard ... 30 seconds` long-tail), restore per-tool H3 sections on the landing page for SEO (Major #2 R2 follow-up from SLICE 1 review), README cross-link row, and a Playwright screenshot regression baseline.

Concretely:
- `docs/site-v2/src/content/docs/docs/drop-in/index.mdx`:
  - Update H1 to "Drop in SpendGuard in 30 seconds (14 tools, one env var)" per design.md §2.2 SEO intent
  - Restore per-tool H3 sections (14-15 H3s) directly on the landing page with the exact env var copyable to clipboard. Each H3: tool name + 1-2 sentence install instruction + the env var code block.
  - Keep matrix as a quick scan, but the H3 sections are the SEO + AI-citation surface.
  - Keep stubs in `/docs/drop-in/<tool>/` — the H3 anchors on the landing + the stubs co-exist; design.md §2.2 wants both.
- `README.md`:
  - New row under `## 🧰 What works today` table referencing `/docs/drop-in/` with one-line marketing copy.
- `docs/site-v2/screenshots/drop-in-landing-{desktop,mobile}.png` — baseline Playwright screenshots at 1280×800 (desktop) + 375×812 (mobile). Used for regression in subsequent slices.
- `docs/site-v2/tests/visual/drop-in.spec.ts` (or similar) — Playwright test that loads `/docs/drop-in/`, screenshots, compares to baseline. Tolerance: 1% pixel diff.

## Files touched

| File | Why |
|------|-----|
| `docs/site-v2/src/content/docs/docs/drop-in/index.mdx` | H1 + restore H3 sections |
| `README.md` | Cross-link row in adapter integrations table |
| `docs/site-v2/screenshots/drop-in-landing-desktop.png` | Baseline desktop screenshot |
| `docs/site-v2/screenshots/drop-in-landing-mobile.png` | Baseline mobile screenshot |
| `docs/site-v2/tests/visual/drop-in.spec.ts` | Playwright regression test |

## Test/verification plan

1. `cd docs/site-v2 && pnpm build` clean.
2. `pnpm astro check` clean.
3. `grep -c "^### " docs/site-v2/src/content/docs/docs/drop-in/index.mdx` ≥ 14 (one H3 per tool).
4. README grep: matches the new row pointing at `/docs/drop-in/`.
5. Playwright test (if Playwright is wired in docs/site-v2 already; otherwise install it as a devDep): `pnpm exec playwright test docs/site-v2/tests/visual/drop-in.spec.ts` produces screenshots matching baseline (within 1% tolerance).
6. Spot-check the H1 string literally matches design.md §2.2.

## Anti-scope

- No `<DropInPicker />` Astro interactive component — SLICE 3 (optional).
- No per-tool deep-dive content (those live in D33 / D34 / etc.).
- Per-tool H3 sections do NOT duplicate the full recipe pages — they're short anchor-points; full content stays in the stubs / D33+D34.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D03_base_url_landing/design.md) §2.2 SEO H1 + H3 sections, Slice 2 plan
- SLICE 1: [`COV_03_d03_landing_skeleton.md`](COV_03_d03_landing_skeleton.md)
- R1 review follow-ups: Major #2 (restore H3 sections) + Major #3 (H1 SEO copy)
