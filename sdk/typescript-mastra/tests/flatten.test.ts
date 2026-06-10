// COV_D38_04 — `flattenStepText` shape tests (tests.md §1 coverage floor:
// flatten.ts 100 % stmt / ≥ 90 % branch).
//
// The flatten is the §6.3 identity input: byte-identical input MUST yield
// byte-identical output (determinism), and a Mastra minor bump must never
// throw from inside the gate (implementation.md §3.2 — the walker is
// written defensively against `unknown`). Each defensive branch is pinned
// here so the gate's behavior on malformed shapes is contractual, not
// accidental.

import { describe, expect, it } from "vitest";
import { flattenStepText } from "../src/flatten.js";

describe("COV_D38_04 flattenStepText (§6.3 identity input)", () => {
  it("V1 MastraDBMessage shape: text parts only, \\n-joined across parts and messages", () => {
    const messages = [
      {
        id: "m1",
        role: "user",
        content: { format: 2, parts: [{ type: "text", text: "hello" }] },
      },
      {
        id: "m2",
        role: "assistant",
        content: {
          format: 2,
          parts: [
            { type: "text", text: "part one" },
            { type: "reasoning", text: "SKIPPED" },
            { type: "text", text: "part two" },
          ],
        },
      },
    ];
    expect(flattenStepText(messages)).toBe("hello\npart one\npart two");
  });

  it("string content is pushed verbatim (MastraMessageV1 / CoreMessage string form)", () => {
    expect(
      flattenStepText([
        { role: "user", content: "plain string" },
        { role: "assistant", content: "second" },
      ]),
    ).toBe("plain string\nsecond");
  });

  it("array content is treated as a parts array (CoreMessage shape)", () => {
    expect(
      flattenStepText([
        {
          role: "user",
          content: [
            { type: "text", text: "from array" },
            { type: "image", image: "data:..." },
          ],
        },
      ]),
    ).toBe("from array");
  });

  it("non-text / malformed parts are skipped, never thrown on", () => {
    expect(
      flattenStepText([
        {
          role: "user",
          content: {
            parts: [
              { type: "text", text: "kept" },
              { type: "text", text: 42 }, // non-string text → skipped
              { type: "tool-call", toolName: "t" },
              null,
              "bare-string-part",
            ],
          },
        },
      ]),
    ).toBe("kept");
  });

  it("defensive shapes flatten without throwing: non-array input, null/primitive messages, missing/odd content", () => {
    // Non-array messages → "".
    expect(flattenStepText(undefined)).toBe("");
    expect(flattenStepText(null)).toBe("");
    expect(flattenStepText("not-an-array")).toBe("");
    expect(flattenStepText({})).toBe("");
    // Null / primitive message entries are skipped.
    expect(flattenStepText([null, 42, "loose-string"])).toBe("");
    // Message without content / numeric content / content without parts /
    // non-array parts — all skipped.
    expect(
      flattenStepText([
        { role: "user" },
        { role: "user", content: 7 },
        { role: "user", content: {} },
        { role: "user", content: { parts: "nope" } },
      ]),
    ).toBe("");
    // Empty array → "".
    expect(flattenStepText([])).toBe("");
  });

  it("determinism: same input twice → byte-identical output", () => {
    const messages = [
      { role: "user", content: { parts: [{ type: "text", text: "déjà vu \u{1F4B8}" }] } },
    ];
    const first = flattenStepText(messages);
    expect(flattenStepText(messages)).toBe(first);
    expect(first).toBe("déjà vu \u{1F4B8}");
  });
});
