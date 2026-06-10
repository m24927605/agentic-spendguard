// SSE encode helper — design.md §7, LOCKED framing:
//
//     encodeSse(e) === "data: " + canonicalEventJson(e) + "\n\n"
//
// [VERIFY-AT-IMPL resolved 2026-06-10 — design.md §7 SSE frame shape]
// Confirmed against the pinned-generation @ag-ui/client@0.0.56 SSE parser:
// it splits the stream on blank lines (`\n\n`), consumes ONLY lines starting
// with `data:` (strips the prefix plus one optional leading space, joins
// multi-data lines, JSON.parses), and ignores `event:`/`id:` fields
// entirely. Data-only frames are therefore exactly what the AG-UI reference
// client consumes; the framing below does not move.
//
// That is the whole transport story in v0.1.0 — anything richer (event ids,
// retry fields, AG-UI client wiring) belongs to the host app.
import { canonicalEventJson } from "./canonical.js";
import type { SpendGuardAgUiEvent } from "./events.js";

export function encodeSse(event: SpendGuardAgUiEvent): string {
  return `data: ${canonicalEventJson(event)}\n\n`;
}

export type AgUiEmit = (event: SpendGuardAgUiEvent) => void | Promise<void>;
