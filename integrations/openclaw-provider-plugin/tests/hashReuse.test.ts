import { describe, expect, it } from "vitest";

import { flattenOpenClawPrompt } from "../src/flatten.js";
import { OpenClawSpendGuardNotImplementedError } from "../src/index.js";

describe("OpenClaw prompt flattening skeleton", () => {
  it("does not implement hash or flatten behavior in slice 1", () => {
    expect(() => flattenOpenClawPrompt()).toThrow(OpenClawSpendGuardNotImplementedError);
  });
});
