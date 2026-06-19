// Package version constant. Mirrors the `version` field in package.json.
// Kept as a typed literal so consumers can `as const`-pin against it.
//
// SLICE 8 bump: drop the `-pre` tag for the first public release. The
// `scripts/version-check.sh` gate enforces strict equality with
// `package.json#version`.
export const VERSION = "0.2.2" as const;
