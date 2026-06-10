# `@spendguard/ag-ui` — Third-Party License Notices

This package (`@spendguard/ag-ui`) is itself licensed under Apache License
2.0 — see the repository root `LICENSE` file
(<https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>) for the
full Apache-2.0 text.

This document lists every direct runtime + peer + dev dependency and its
license. Transitive deps that flow in via these direct edges follow their own
license; this notice is required only for the direct edges below.

---

## Runtime dependencies

**None.** The published tarball contains only the package's own compiled
output (`dist/index.js` + `dist/index.d.ts`) and no third-party runtime
dependencies are bundled or resolved — the builders, serializer, and SSE
helper are dependency-free by design (D39 design.md §10 / implementation.md
§2: a non-empty `dependencies` block is a review blocker).

---

## Optional peer dependencies (resolved in the consumer's tree, if at all)

### `@ag-ui/core`

- License: **MIT**
- Project: <https://github.com/ag-ui-protocol/ag-ui>
- Declared: `peerDependencies: { "@ag-ui/core": ">=0.0.27 <0.1.0" }` with
  `peerDependenciesMeta: { "@ag-ui/core": { "optional": true } }`.
- Use: **types-only, opt-in, never imported by this package.** Consumers who
  want AG-UI's own `CustomEvent` typing or zod validation install it
  themselves; nothing in `src/` or `dist/` references it. The package's own
  CI pins it exactly as a devDependency for the compat test suite only.
- Full MIT text:
  <https://github.com/ag-ui-protocol/ag-ui/blob/main/LICENSE>

---

## Build/Dev-only dependencies

Dev-only dependencies (`tsup`, `vitest`, `@vitest/coverage-v8`, `typescript`,
`@biomejs/biome`, `@types/node`, and the exact-pinned `@ag-ui/core` compat
fixture) are NOT shipped to consumers — they live under `"devDependencies"`
and are stripped from the published tarball. They do not require notice
attribution because they do not appear in the consumer's runtime tree.

---

## Apache-2.0 compliance summary

Per Apache-2.0 §4(c), this `LICENSE_NOTICES.md` file is the NOTICE document
for redistribution. Consumers that re-distribute `@spendguard/ag-ui` in
binary form (e.g. as part of a larger application's deploy artifact) should
preserve this file. Consumers that depend on `@spendguard/ag-ui` via npm and
ship it as-installed are already compliant — npm's install tree preserves
this file under `node_modules/@spendguard/ag-ui/LICENSE_NOTICES.md`.
