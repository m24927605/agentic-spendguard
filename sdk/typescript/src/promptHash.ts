// SpendGuard SDK — prompt_hash normalization + HMAC-SHA256 (SLICE 6 /
// COV_S05_06).
//
// Cross-language byte-equivalence with:
//   - Python `sdk/python/src/spendguard/prompt_hash.py::compute`
//   - Rust `services/sidecar/src/prompt_hash.rs::compute`
//
// Cost Advisor P0.5 rules dedupe retried LLM calls by `(run_id, prompt_hash)`.
// The hash MUST be deterministic across the TS adapter, the Python adapter,
// and the Rust sidecar — drift here breaks rule dedup. This is a P0 invariant
// (review-standards §1.5 cross-language gate).
//
// Privacy: prompt_hash is a **tenant-salted HMAC**, not plain SHA-256.
// HMAC-SHA256 with `tenant_id` as the key defeats cross-tenant correlation
// and raises the bar against dictionary attacks on common prompts.
//
// Tenant canonicalization: `tenant_id` is normalized to canonical
// lowercase-hyphenated UUID form before HMAC keying. Otherwise the same
// tenant calling with "ABC-..." vs "abc-..." would compute different HMAC
// keys and split same-tenant dedup. Non-UUID inputs fall back to the raw
// string verbatim (degraded but stable).
//
// Normalization (v1):
//   1. UTF-8 only.
//   2. Trim leading + trailing ASCII whitespace (space, tab, newline, FF, CR).
//   3. NO Unicode normalization (NFC) in v1.
//   4. Output: 64-char lowercase hex HMAC-SHA256.
//
// Spec refs:
//   - design.md §4.8 (LOCKED surface)
//   - implementation.md §7 (`src/promptHash.ts`)
//   - review-standards.md §1.5 P0 cross-language gate
//   - tests.md §5.3 (cross-language fixture matrix)

import { createHmac } from "node:crypto";

// Mirror Rust `char::is_ascii_whitespace` set AND Python's `_ASCII_WHITESPACE`.
// The set is: space, tab, line-feed, form-feed, carriage-return.
const ASCII_WHITESPACE = new Set([" ", "\t", "\n", "\f", "\r"]);

// Canonical-form UUID regex (RFC 4122). Accepts upper and lowercase hex; the
// canonicalizer lowercases. Non-UUID strings are passed through verbatim.
const UUID_RE = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

/**
 * Canonicalize a tenant identifier.
 *
 * If the input is a valid UUID (RFC 4122 canonical form), it is lowercased.
 * Otherwise the input is returned verbatim. Mirrors Python
 * `_canonicalize_tenant` semantics: `str(uuid.UUID(tenant_id))` raises on
 * invalid input → Python returns the raw string; in TS we test the regex
 * and return the lowercased form on success.
 */
function canonicalizeTenant(tenantId: string): string {
  if (UUID_RE.test(tenantId)) return tenantId.toLowerCase();
  return tenantId;
}

/**
 * Trim leading + trailing ASCII whitespace from a string.
 *
 * Mirrors Python `str.strip(_ASCII_WHITESPACE)` semantics. Does NOT use
 * `String.prototype.trim` because that strips Unicode whitespace too, which
 * would diverge from Python's ASCII-only behavior.
 */
function stripAscii(s: string): string {
  let i = 0;
  while (i < s.length && ASCII_WHITESPACE.has(s.charAt(i))) i++;
  let j = s.length;
  while (j > i && ASCII_WHITESPACE.has(s.charAt(j - 1))) j--;
  return s.slice(i, j);
}

/**
 * Compute the lowercase hex HMAC-SHA256 of a normalized prompt with the
 * tenant key.
 *
 * Determinism: `computePromptHash(s, t) === computePromptHash(s, t)` for any
 * (s, t).
 *
 * Cross-language: byte-for-byte identical to:
 *   - Python `spendguard.prompt_hash.compute(s, t)`
 *   - Rust `services::sidecar::prompt_hash::compute(s, t)`
 *
 * Cross-tenant: two tenants asking the same prompt produce different
 * hashes.
 *
 * @param promptText The raw prompt text. Leading/trailing ASCII whitespace
 *   is stripped before hashing.
 * @param tenantId The tenant identifier. If a canonical UUID, lowercased
 *   before keying the HMAC; otherwise passed through verbatim.
 * @returns 64-char lowercase hex string.
 */
export function computePromptHash(promptText: string, tenantId: string): string {
  const key = canonicalizeTenant(tenantId);
  const trimmed = stripAscii(promptText);
  return createHmac("sha256", key).update(trimmed, "utf8").digest("hex");
}
