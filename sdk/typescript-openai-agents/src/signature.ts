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
//   - `string` input: TS emits a Python `repr(str)`-style literal for the
//     supported prompt surface, including quote selection and common control
//     escapes.
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
 * Python parity quirk: for string inputs we render a Python `repr(str)`-style
 * literal. For list-of-message inputs both languages serialize to JSON via
 * the canonical path described in module JSDoc. The cross-language fixture
 * (SLICE 3) gates the agreement.
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
 * - `string` → Python-`repr(str)`-style quoted literal.
 * - non-`string` (typically `AgentInputItem[]`) → `JSON.stringify(input)`.
 *
 * The Python wrapper does the same — `str` goes through `repr()`, list
 * inputs go through `json.dumps`. The SLICE 3 cross-language fixture
 * (`sdk/fixtures/cross-language/v1.json#openai_agents`) verifies byte
 * equality.
 */
function renderInputCanonical(input: unknown): string {
  if (typeof input === "string") {
    return pythonReprString(input);
  }
  // For non-string inputs we let JSON.stringify drive the canonical form.
  // `JSON.stringify(undefined)` is `undefined` — coerce to "null" so the
  // hash still resolves.
  const json = JSON.stringify(input);
  return json ?? "null";
}

function pythonReprString(input: string): string {
  const quote = input.includes("'") && !input.includes('"') ? '"' : "'";
  let escaped = "";
  for (const ch of input) {
    const code = ch.codePointAt(0);
    if (code === undefined) continue;
    if (ch === "\\") {
      escaped += "\\\\";
    } else if (ch === "\n") {
      escaped += "\\n";
    } else if (ch === "\t") {
      escaped += "\\t";
    } else if (ch === "\r") {
      escaped += "\\r";
    } else if (ch === quote) {
      escaped += `\\${quote}`;
    } else if (code < 0x20 || code === 0x7f) {
      escaped += `\\x${code.toString(16).padStart(2, "0")}`;
    } else if (!isPythonPrintable(ch, code)) {
      escaped += pythonReprCodePointEscape(code);
    } else {
      escaped += ch;
    }
  }
  return `${quote}${escaped}${quote}`;
}

const PYTHON_NON_PRINTABLE_RE = /[\p{C}\p{Z}]/u;

function isPythonPrintable(ch: string, code: number): boolean {
  // Python str.isprintable() keeps ASCII space but escapes other separators
  // and all control/format/private/surrogate/unassigned code points.
  return code === 0x20 || !PYTHON_NON_PRINTABLE_RE.test(ch);
}

function pythonReprCodePointEscape(code: number): string {
  if (code <= 0xff) {
    return `\\x${code.toString(16).padStart(2, "0")}`;
  }
  if (code <= 0xffff) {
    return `\\u${code.toString(16).padStart(4, "0")}`;
  }
  return `\\U${code.toString(16).padStart(8, "0")}`;
}
