import { describe, expect, it } from "vitest";

import { prepareOpenClawIdentity } from "../src/identity.js";
import { OpenClawSpendGuardNotImplementedError } from "../src/index.js";

describe("OpenClaw identity skeleton", () => {
  it("does not implement identity derivation in slice 1", () => {
    expect(() => prepareOpenClawIdentity()).toThrow(OpenClawSpendGuardNotImplementedError);
  });
});
