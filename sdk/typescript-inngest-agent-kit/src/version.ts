// Package version constant. Mirrors the `version` field in package.json.
// Kept as a typed literal so consumers can `as const`-pin against it.
export const VERSION = "0.1.0" as const;
