// COV_D38_03 — `extractUsage` shape tests (tests.md TP-24..TP-26 support).
//
// Pins the V4 resolution (see src/usage.ts header): flat camelCase
// `inputTokens`/`outputTokens` exposed DIRECTLY at `processOutputStep`
// (`args.usage`) and via the stripped `finish` chunk's
// `payload.output.usage` at `processLLMResponse`; snake_case accepted for
// D04/D06 cross-shape parity; `undefined` (NOT zeros) when absent.

import { describe, expect, it } from "vitest";
import { extractUsage } from "../src/usage.js";

describe("COV_D38_03 extractUsage (V4-pinned shapes)", () => {
  it("processOutputStep shape: flat camelCase args.usage", () => {
    expect(extractUsage({ usage: { inputTokens: 7, outputTokens: 3, totalTokens: 10 } })).toEqual({
      inputTokens: 7,
      outputTokens: 3,
    });
  });

  it("processOutputStep shape: snake_case args.usage (D04/D06 discipline)", () => {
    expect(extractUsage({ usage: { input_tokens: 11, output_tokens: 4 } })).toEqual({
      inputTokens: 11,
      outputTokens: 4,
    });
  });

  it("processLLMResponse shape: finish chunk payload.output.usage", () => {
    const args = {
      chunks: [
        { type: "text-delta", payload: { text: "hi" } },
        { type: "finish", payload: { output: { usage: { inputTokens: 10, outputTokens: 5 } } } },
      ],
    };
    expect(extractUsage(args)).toEqual({ inputTokens: 10, outputTokens: 5 });
  });

  it("processLLMResponse shape: snake_case usage inside the finish chunk", () => {
    const args = {
      chunks: [
        { type: "finish", payload: { output: { usage: { input_tokens: 2, output_tokens: 8 } } } },
      ],
    };
    expect(extractUsage(args)).toEqual({ inputTokens: 2, outputTokens: 8 });
  });

  it("finish chunk with payload.usage (no output envelope) is accepted", () => {
    expect(
      extractUsage({
        chunks: [{ type: "finish", payload: { usage: { inputTokens: 1, outputTokens: 1 } } }],
      }),
    ).toEqual({ inputTokens: 1, outputTokens: 1 });
  });

  it("providerEventId rides the response-metadata chunk's payload.id", () => {
    const args = {
      chunks: [
        { type: "response-metadata", payload: { id: "prov-evt-42", modelId: "m" } },
        { type: "finish", payload: { output: { usage: { inputTokens: 3, outputTokens: 2 } } } },
      ],
    };
    expect(extractUsage(args)).toEqual({
      inputTokens: 3,
      outputTokens: 2,
      providerEventId: "prov-evt-42",
    });
  });

  it("zero-token usage is VALID usage (not coerced to absent)", () => {
    expect(extractUsage({ usage: { inputTokens: 0, outputTokens: 0 } })).toEqual({
      inputTokens: 0,
      outputTokens: 0,
    });
  });

  it("absent usage → undefined, NOT zeros (§6.6 fallback selector)", () => {
    expect(extractUsage(undefined)).toBeUndefined();
    expect(extractUsage(null)).toBeUndefined();
    expect(extractUsage("not-an-object")).toBeUndefined();
    expect(extractUsage({})).toBeUndefined();
    expect(extractUsage({ usage: null })).toBeUndefined();
    expect(extractUsage({ chunks: [] })).toBeUndefined();
    expect(extractUsage({ chunks: [{ type: "text-delta", payload: {} }] })).toBeUndefined();
    expect(extractUsage({ chunks: [{ type: "finish", payload: {} }] })).toBeUndefined();
    expect(extractUsage({ chunks: "nope" })).toBeUndefined();
    // COV_D38_04 floor top-up: finish chunk with a missing / null payload
    // (usageFromChunkPayload's non-object guard).
    expect(extractUsage({ chunks: [{ type: "finish" }] })).toBeUndefined();
    expect(extractUsage({ chunks: [{ type: "finish", payload: null }] })).toBeUndefined();
    // response-metadata with unusable payloads never fabricates an id.
    expect(
      extractUsage({
        chunks: [
          { type: "response-metadata" },
          { type: "response-metadata", payload: { id: "" } },
          { type: "response-metadata", payload: { id: 42 } },
          { type: "finish", payload: { output: { usage: { inputTokens: 1, outputTokens: 2 } } } },
        ],
      }),
    ).toEqual({ inputTokens: 1, outputTokens: 2 });
  });

  it("partial / non-numeric usage is treated as absent (no fabricated zeros)", () => {
    // normalizeUsage can legally produce { inputTokens: undefined, ... }.
    expect(extractUsage({ usage: { inputTokens: undefined, outputTokens: 5 } })).toBeUndefined();
    expect(extractUsage({ usage: { inputTokens: 5 } })).toBeUndefined();
    expect(extractUsage({ usage: { inputTokens: "5", outputTokens: "3" } })).toBeUndefined();
    expect(extractUsage({ usage: { inputTokens: Number.NaN, outputTokens: 3 } })).toBeUndefined();
    expect(extractUsage({ usage: { inputTokens: -1, outputTokens: 3 } })).toBeUndefined();
  });

  it("non-object chunk entries are tolerated; last finish wins", () => {
    const args = {
      chunks: [
        null,
        42,
        { type: "finish", payload: { output: { usage: { inputTokens: 1, outputTokens: 1 } } } },
        { type: "finish", payload: { output: { usage: { inputTokens: 9, outputTokens: 9 } } } },
      ],
    };
    expect(extractUsage(args)).toEqual({ inputTokens: 9, outputTokens: 9 });
  });
});
