import type { ProviderPlugin as OpenClawProvider } from "openclaw/plugin-sdk/provider-model-shared";

import { OpenClawSpendGuardNotImplementedError } from "./errors.js";
import { validateOptions, type OpenClawSpendGuardOptions } from "./options.js";

export type { OpenClawProvider };

export type OpenClawProviderContext = Record<string, unknown>;

function notImplementedCatalog(feature: string) {
  return async (): Promise<never> => {
    throw new OpenClawSpendGuardNotImplementedError(feature);
  };
}

export function createSpendGuardOpenClawProvider(
  upstream: OpenClawProvider,
  options: OpenClawSpendGuardOptions,
): OpenClawProvider {
  validateOptions(options);

  const wrapped: OpenClawProvider = {
    ...upstream,
    catalog: {
      order: upstream.catalog?.order ?? "simple",
      run: notImplementedCatalog("OpenClaw provider reserve/dispatch wrapper"),
    },
  };

  if (upstream.staticCatalog) {
    wrapped.staticCatalog = {
      order: upstream.staticCatalog.order ?? "simple",
      run: notImplementedCatalog("OpenClaw provider static catalog wrapper"),
    };
  }

  return wrapped;
}
