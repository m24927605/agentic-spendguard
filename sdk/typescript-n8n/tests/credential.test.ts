// D37 unit tests — SpendGuardApi credential schema.
// Covers C-01..C-08 per tests.md §3.2.

import { describe, expect, it } from "vitest";
import { SpendGuardApi } from "../credentials/SpendGuardApi.credentials";

describe("SpendGuardApi credential", () => {
  const cred = new SpendGuardApi();

  it("C-01 name === 'spendGuardApi'", () => {
    expect(cred.name).toBe("spendGuardApi");
  });

  it("C-02 displayName === 'SpendGuard API'", () => {
    expect(cred.displayName).toBe("SpendGuard API");
  });

  it("C-03 documentationUrl points to integrations page", () => {
    expect(cred.documentationUrl).toContain("integrations/n8n");
  });

  it("C-04 properties in canonical order", () => {
    const order = cred.properties.map((p) => p.name);
    expect(order).toEqual([
      "tenantId",
      "socketPath",
      "budgetId",
      "windowInstanceId",
      "runtimeKind",
    ]);
  });

  it("C-05 tenantId / budgetId / windowInstanceId are required", () => {
    const required = cred.properties.filter((p) => p.required).map((p) => p.name);
    expect(required.sort()).toEqual(["budgetId", "tenantId", "windowInstanceId"].sort());
  });

  it("C-06 socketPath default is /var/run/spendguard/sidecar.sock", () => {
    const socket = cred.properties.find((p) => p.name === "socketPath");
    expect(socket?.default).toBe("/var/run/spendguard/sidecar.sock");
  });

  it("C-07 runtimeKind default is 'n8n'", () => {
    const rk = cred.properties.find((p) => p.name === "runtimeKind");
    expect(rk?.default).toBe("n8n");
  });

  it("C-08 no test function (lazy handshake)", () => {
    expect((cred as unknown as Record<string, unknown>).test).toBeUndefined();
    // Also assert via the typed surface — `ICredentialType.test` is optional.
    expect("test" in cred ? cred.test : undefined).toBeUndefined();
  });
});
