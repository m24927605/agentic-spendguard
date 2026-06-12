import { describe, expect, it } from "vitest";

import { extractOpenClawUsage } from "../src/usage.js";
import { OpenClawSpendGuardNotImplementedError } from "../src/index.js";

describe("OpenClaw usage skeleton", () => {
  it("does not implement streaming usage extraction in slice 1", () => {
    expect(() => extractOpenClawUsage()).toThrow(OpenClawSpendGuardNotImplementedError);
  });
});
