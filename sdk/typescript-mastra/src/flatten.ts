// src/flatten.ts — deterministic step-text flatten (implementation.md §3.2).
//
// Walks the hook-provided step messages and concatenates TEXT PARTS ONLY,
// "\n"-joined — the same flatten discipline as D06 `flattenPromptText` and
// D04 `measureContentChars`. Images / tool-call payloads / reasoning /
// binary parts are skipped.
//
// Determinism matters: identity derivation (§6.3) depends on byte-identical
// output for byte-identical input.
//
// [VERIFY-AT-IMPL: V1] PINNED (COV_D38_02, @mastra/core 1.41.0): the
// `processInputStep` hook delivers `messages: MastraDBMessage[]` —
// `{ id, role, createdAt, content: { format: 2, parts: MastraMessagePart[] } }`
// where text parts are `{ type: "text", text: string }`. The walker below
// matches that shape but is written defensively against `unknown` so a
// Mastra minor bump cannot throw from inside the gate (implementation.md
// §3.2 requirement).

/** True for a `{ type: "text", text: string }` part. */
function isTextPart(part: unknown): part is { type: "text"; text: string } {
  return (
    part !== null &&
    typeof part === "object" &&
    (part as { type?: unknown }).type === "text" &&
    typeof (part as { text?: unknown }).text === "string"
  );
}

/** Collect text-typed parts from an unknown parts array into `out`. */
function collectTextParts(parts: unknown, out: string[]): void {
  if (!Array.isArray(parts)) {
    return;
  }
  for (const part of parts) {
    if (isTextPart(part)) {
      out.push(part.text);
    }
  }
}

/**
 * Flatten step messages to a single deterministic text blob.
 *
 * Per message:
 *   - string `content` → pushed verbatim (defensive: V1-era MastraDBMessage
 *     always carries an object content, but MastraMessageV1 / CoreMessage
 *     string contents must not break the gate);
 *   - object `content` with `parts` → text parts only;
 *   - array `content` → treated as a parts array (CoreMessage shape);
 *   - anything else → skipped.
 *
 * Joined with "\n". Non-array input flattens to "".
 */
export function flattenStepText(messages: unknown): string {
  const out: string[] = [];
  if (!Array.isArray(messages)) {
    return "";
  }
  for (const msg of messages) {
    if (msg === null || typeof msg !== "object") {
      continue;
    }
    const content = (msg as { content?: unknown }).content;
    if (typeof content === "string") {
      out.push(content);
      continue;
    }
    if (Array.isArray(content)) {
      collectTextParts(content, out);
      continue;
    }
    if (content !== null && typeof content === "object") {
      collectTextParts((content as { parts?: unknown }).parts, out);
    }
  }
  return out.join("\n");
}
