// `n8n-nodes-spendguard` — public package barrel.
//
// The n8n loader reads `package.json` `n8n.nodes[]` + `n8n.credentials[]`
// to discover the runtime classes; this barrel exists primarily so the
// package's `main` (CJS-only per design.md §6.6) loads cleanly when
// vendored as a dependency. Re-exports the helper modules for tests and
// downstream tooling.

export { resolveRunIdentity } from "./runIdentity";
export type { RunIdSource, RunIdentity } from "./runIdentity";
export { acquireClient } from "./clientPool";
export { mapToNodeApiError } from "./errors";
export { VERSION } from "./version";
