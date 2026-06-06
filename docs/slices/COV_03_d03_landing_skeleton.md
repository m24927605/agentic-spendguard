# COV_03 — D03 base-URL landing page: page skeleton + matrix

> **Deliverable**: D03 OPENAI_BASE_URL drop-in landing page
> **Slice**: 1 of 2-3 (Slice 1)
> **Spec set**: [`docs/specs/coverage/D03_base_url_landing/`](../specs/coverage/D03_base_url_landing/)

## Scope

Ship the landing page itself: a single Starlight markdown page at `docs/site-v2/src/content/docs/docs/drop-in/index.mdx` listing the 14 supported Pattern-2 tools, each with the exact env var or config line the tool's docs specify, plus sidebar wiring so the page appears in the site navigation.

Concretely:
- New page: `docs/site-v2/src/content/docs/docs/drop-in/index.mdx` — title "Drop-in coverage: 14 tools, one env var", lede 1-paragraph, matrix table with columns: Tool | Provider field | Exact value | Upstream source link | Verified status (`Spec` initially; deeper-dive slices may flip to `Live`).
- 14 stub pages for the per-tool deep-dives at `docs/site-v2/src/content/docs/docs/drop-in/{litellm,aider,continue,cline,roo-code,openhands,goose,zed,copilot-cli,tabnine,anythingllm,lobechat,cody,augment,dify}.mdx`. Each stub: title, "Coming soon" body, frontmatter `pagefind: false`. (Deep-dives ship in D33 / D34 / etc.)
- Sidebar config: `docs/site-v2/astro.config.mjs` — add `Drop-in` section with the 15 pages (index + 14).
- `citations/` directory under `docs/specs/coverage/D03_base_url_landing/` with one citation-source-link file per tool listing the upstream URL we sourced the env var from. (PDF snapshots come in SLICE 2.)

## Files touched

| File | Why |
|------|-----|
| `docs/site-v2/src/content/docs/docs/drop-in/index.mdx` | Landing matrix |
| `docs/site-v2/src/content/docs/docs/drop-in/{14 tools}.mdx` | Per-tool stub deep-dive pages |
| `docs/site-v2/astro.config.mjs` | Sidebar wiring |
| `docs/specs/coverage/D03_base_url_landing/citations/upstream-sources.md` | Citation source list |

## Test/verification plan

1. `cd docs/site-v2 && pnpm build` succeeds.
2. `cd docs/site-v2 && pnpm astro check` passes — type-check + MDX parse.
3. `grep -c "^| \`" docs/site-v2/src/content/docs/docs/drop-in/index.mdx` returns ≥ 14 (one row per tool in the matrix).
4. Manual review: each row's "Exact value" column matches the upstream source listed in `upstream-sources.md`.
5. `pnpm exec pagefind` (if pagefind is wired) doesn't error on stubs.

## Anti-scope

- No copy polish — SLICE 2.
- No README sync — SLICE 2.
- No screenshots — SLICE 2.
- No interactive `<DropInPicker />` Astro component — Slice 3 (optional).
- Per-tool deep-dive content — separate deliverables (D33 AnythingLLM, D34 LobeChat, etc.).

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D03_base_url_landing/design.md) Slice 1 section
- Build plan: [`framework-coverage-build-plan-2026-06.md`](../strategy/framework-coverage-build-plan-2026-06.md) §1.5
- Review standards: [`review-standards.md`](../specs/coverage/D03_base_url_landing/review-standards.md)
