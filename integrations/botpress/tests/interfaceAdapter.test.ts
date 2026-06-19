// interfaceAdapter.test.ts — `llm` interface <-> internal boundary mapping.
//
// The integration adopts the formal Botpress `llm` interface via
// `.extend(llm, ...)`, so the `generateContent` handler is invoked with the
// interface's rich input and must return the interface's rich output. These
// cases pin the boundary adapter that narrows the interface input down to the
// SpendGuard pipeline's internal shape and widens the internal result back up.
//
//   IA01 string content passes through unchanged.
//   IA02 multipart content flattens to the concatenated text parts (images
//        drop out of the text estimate).
//   IA03 null content (assistant-only-tool-calls) flattens to "".
//   IA04 model { id } maps through; omitted model stays undefined.
//   IA05 the optional caps (systemPrompt/maxTokens/temperature/topP/
//        stopSequences/userId) thread through unchanged.
//   IA06 internal output widens to the interface output, adding the required
//        usage.inputCost / usage.outputCost (0) and preserving botpress.cost.

import { describe, expect, test } from "vitest";
import {
  type InterfaceGenerateContentInput,
  flattenContent,
  toInterfaceOutput,
  toInternalInput,
} from "../src/llm/interfaceAdapter.js";
import type { GenerateContentOutput } from "../src/llm/schemas.js";

function makeInterfaceInput(
  overrides: Partial<InterfaceGenerateContentInput> = {},
): InterfaceGenerateContentInput {
  return {
    model: { id: "gpt-4o-mini" },
    messages: [{ role: "user", type: "text", content: "hello" }],
    temperature: 1,
    topP: 1,
    ...overrides,
  };
}

describe("flattenContent (IA01–IA03)", () => {
  test("IA01 string content passes through", () => {
    expect(flattenContent("plain text")).toBe("plain text");
  });

  test("IA02 multipart content flattens to text parts only", () => {
    const content = [
      { type: "text" as const, text: "describe " },
      { type: "image" as const, url: "https://example.com/cat.png" },
      { type: "text" as const, text: "this image" },
    ];
    expect(flattenContent(content)).toBe("describe this image");
  });

  test("IA03 null content flattens to empty string", () => {
    expect(flattenContent(null)).toBe("");
  });
});

describe("toInternalInput (IA04–IA05)", () => {
  test("IA04 model { id } maps through; omitted model stays undefined", () => {
    expect(toInternalInput(makeInterfaceInput()).model).toEqual({ id: "gpt-4o-mini" });
    // An interface input with `model` absent (not `undefined`, which
    // exactOptionalPropertyTypes forbids for an optional field).
    const noModel: InterfaceGenerateContentInput = {
      messages: [{ role: "user", type: "text", content: "hello" }],
      temperature: 1,
      topP: 1,
    };
    expect(toInternalInput(noModel).model).toBeUndefined();
  });

  test("IA04b messages flatten content + preserve role", () => {
    const internal = toInternalInput(
      makeInterfaceInput({
        messages: [
          { role: "user", type: "text", content: "hi" },
          {
            role: "assistant",
            type: "multipart",
            content: [{ type: "text", text: "part-a" }],
          },
        ],
      }),
    );
    expect(internal.messages).toEqual([
      { role: "user", content: "hi" },
      { role: "assistant", content: "part-a" },
    ]);
  });

  test("IA05 optional caps thread through unchanged", () => {
    const internal = toInternalInput(
      makeInterfaceInput({
        systemPrompt: "you are a budget guard",
        maxTokens: 256,
        temperature: 0.3,
        topP: 0.9,
        stopSequences: ["STOP"],
        userId: "u-1",
      }),
    );
    expect(internal.systemPrompt).toBe("you are a budget guard");
    expect(internal.maxTokens).toBe(256);
    expect(internal.temperature).toBe(0.3);
    expect(internal.topP).toBe(0.9);
    expect(internal.stopSequences).toEqual(["STOP"]);
    expect(internal.userId).toBe("u-1");
  });
});

describe("toInterfaceOutput (IA06)", () => {
  test("IA06 widens internal output to interface output with cost fields", () => {
    const internal: GenerateContentOutput = {
      id: "prov-resp-1",
      provider: "openai",
      model: "gpt-4o-mini",
      choices: [
        { role: "assistant", type: "text", content: "hi there", index: 0, stopReason: "stop" },
      ],
      usage: { inputTokens: 11, outputTokens: 7 },
      botpress: { cost: 0.42 },
    };
    const out = toInterfaceOutput(internal);
    expect(out.id).toBe("prov-resp-1");
    expect(out.provider).toBe("openai");
    expect(out.choices[0]).toEqual({
      role: "assistant",
      type: "text",
      content: "hi there",
      index: 0,
      stopReason: "stop",
    });
    // Interface requires per-token cost fields the pipeline does not compute.
    expect(out.usage).toEqual({
      inputTokens: 11,
      inputCost: 0,
      outputTokens: 7,
      outputCost: 0,
    });
    // Advisory aggregate cost is preserved.
    expect(out.botpress.cost).toBe(0.42);
  });
});
