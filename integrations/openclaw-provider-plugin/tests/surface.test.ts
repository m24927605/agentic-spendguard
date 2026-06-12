import { describe, expect, it } from "vitest";

import * as publicApi from "../src/index.js";
import type {
  OpenClawProvider,
  OpenClawProviderContext,
  OpenClawSpendGuardOptions,
} from "../src/index.js";
import { createSpendGuardOpenClawProvider } from "../src/index.js";

type Expect<T extends true> = T;
type IsAssignable<T, U> = T extends U ? true : false;

type _FactoryShape = Expect<
  IsAssignable<
    typeof createSpendGuardOpenClawProvider,
    (upstream: OpenClawProvider, options: OpenClawSpendGuardOptions) => OpenClawProvider
  >
>;

type _ContextShape = Expect<IsAssignable<OpenClawProviderContext, Record<string, unknown>>>;

describe("public barrel skeleton", () => {
  it("exports only the slice-1 runtime surface", () => {
    expect(Object.keys(publicApi).sort()).toEqual([
      "OpenClawSpendGuardConfigError",
      "OpenClawSpendGuardError",
      "OpenClawSpendGuardNotImplementedError",
      "VERSION",
      "createSpendGuardOpenClawProvider",
    ]);
  });
});
