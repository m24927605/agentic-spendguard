import { describe, expect, it } from "vitest";

import { OpenClawSpendGuardNotImplementedError } from "../src/index.js";

describe("OpenClaw typed errors", () => {
  it("exports the slice-1 fail-closed placeholder error", () => {
    const error: Error = new OpenClawSpendGuardNotImplementedError("fail-closed reserve path");

    expect(error.name).toBe("OpenClawSpendGuardNotImplementedError");
    expect(error.message).toContain("COV_D40B_01_plugin_package_init");
  });
});
