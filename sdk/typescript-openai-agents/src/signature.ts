// `deriveAgentSignature` — stable BLAKE2b-128 fingerprint of an OpenAI
// Agents `ModelRequest` input shape. Feeds into the substrate's
// `deriveUuidFromSignature(...)` to mint deterministic
// `(decisionId, llmCallId)` pairs without burning a UUIDv7 per call.
//
// Why BLAKE2b-128: design.md §5 line "signature = blake2b16(...)". The same
// hash is used by `@spendguard/sdk::computePromptHash` and Python
// `_signature(...)` — cross-language parity gates (review-standards.md §2)
// require byte-equivalence. We delegate to the substrate's
// `deriveUuidFromSignature` for the actual UUID; this module only owns the
// hex-digest computation.
//
// Canonical rendering — the only place this adapter diverges from Python's
// exact bytes — is documented inline. The Python wrapper uses `repr(input)`,
// which is not portable. For:
//   - `string` input: TS emits `'<escaped>'` mirroring Python `repr('s')`.
//   - `AgentInputItem[]` input: both languages emit JSON; the
//     cross-language fixture (SLICE 3) verifies the agreed canonical shape
//     end-to-end. Field ordering is NOT sorted — both languages depend on
//     stable insertion order for the same logical input; the SLICE 3
//     fixture generator (`scripts/dump_openai_agents_signatures.py`)
//     re-renders Python output through the same JSON serializer to keep
//     the comparison meaningful.
//
// The single-place divergence is review-standards.md §2.7 — JSDoc + fixture
// MUST acknowledge it.

import { blake2b } from "@noble/hashes/blake2b";
import { bytesToHex } from "@noble/hashes/utils";

/**
 * Compute the stable hex signature for an OpenAI Agents `ModelRequest`'s
 * `(input, systemInstructions)` pair.
 *
 * @param input - The `ModelRequest.input` field — either a raw string
 *   prompt (older Chat Completions style) or a list of `AgentInputItem`
 *   message objects (Responses API style). Both shapes are supported.
 * @param systemInstructions - The `ModelRequest.systemInstructions` field.
 *   Treated as `""` when `null` or `undefined` so two calls with no system
 *   prompt collapse to the same signature.
 * @returns 32-character lowercase hex string — BLAKE2b output truncated to
 *   16 bytes.
 *
 * @remarks
 * Python parity quirk: for string inputs we render `repr('value')` —
 * Python's `repr()` on a `str` emits `'<escaped>'` with single quotes and
 * `\\` / `\'` escaping. For list-of-message inputs both languages serialize
 * to JSON via the canonical path described in module JSDoc. The
 * cross-language fixture (SLICE 3) gates the agreement.
 */
export function deriveAgentSignature(
  input: unknown,
  systemInstructions: string | null | undefined,
): string {
  const repr = renderInputCanonical(input);
  const sysSegment = systemInstructions == null ? "" : systemInstructions;
  const text = `${repr}|${sysSegment}`;
  return bytesToHex(blake2b(text, { dkLen: 16 }));
}

/**
 * Render the OpenAI Agents `input` field into a stable string form.
 *
 * - `string` → Python-`repr`-style `'<escaped>'`.
 * - non-`string` (typically `AgentInputItem[]`) → `JSON.stringify(input)`.
 *
 * The Python wrapper does the same — `str` goes through `repr()`, list
 * inputs go through `json.dumps`. The SLICE 3 cross-language fixture
 * (`sdk/fixtures/cross-language/v1.json#openai_agents`) verifies byte
 * equality.
 */
function renderInputCanonical(input: unknown): string {
  if (typeof input === "string") {
    // Python-`repr`-style single-quote rendering. `repr` escapes `\\` first
    // (so an existing backslash does not turn the following character into
    // a control-escape), then `'` (the quote character used to wrap the
    // literal). Order matters: do `\\` BEFORE `'` or the `\'` escape would
    // double-escape the backslash.
    const escaped = input.replace(/\\/g, "\\\\").replace(/'/g, "\\'");
    return `'${escaped}'`;
  }
  // For non-string inputs we let JSON.stringify drive the canonical form.
  // `JSON.stringify(undefined)` is `undefined` — coerce to "null" so the
  // hash still resolves.
  const json = JSON.stringify(input);
  return json ?? "null";
}
