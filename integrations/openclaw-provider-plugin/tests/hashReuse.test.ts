import { describe, expect, it } from "vitest";

import { flattenOpenClawPrompt } from "../src/flatten.js";

describe("OpenClaw prompt flattening", () => {
  it("flattens structured prompt/message content deterministically", () => {
    expect(
      flattenOpenClawPrompt({
        messages: [
          { role: "system", content: "stay within budget" },
          { role: "user", content: [{ type: "text", text: "hello" }] },
        ],
      }),
    ).toBe("system: stay within budget\nuser: hello");
  });
});
