import { describe, expect, it } from "vitest";

describe("@spendguard/langchain locked surface", () => {
  it("exports VERSION", async () => {
    const mod = await import("../src/index.js");
    expect(typeof mod.VERSION).toBe("string");
    expect(mod.VERSION).toMatch(/^\d+\.\d+\.\d+/);
  });
});
