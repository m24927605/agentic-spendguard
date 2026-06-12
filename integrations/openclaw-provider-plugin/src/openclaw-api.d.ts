declare module "openclaw/plugin-sdk/provider-model-shared" {
  export type ProviderPluginCatalog = {
    order?: "simple" | "profile" | "paired" | "late" | string;
    run: (ctx: unknown) => Promise<unknown>;
  };

  export type ProviderPlugin = {
    id: string;
    pluginId?: string;
    label: string;
    docsPath?: string;
    aliases?: string[];
    hookAliases?: string[];
    envVars?: string[];
    auth: unknown[];
    catalog?: ProviderPluginCatalog;
    staticCatalog?: ProviderPluginCatalog;
    [key: string]: unknown;
  };
}

declare module "openclaw/plugin-sdk/plugin-entry" {
  import type { ProviderPlugin } from "openclaw/plugin-sdk/provider-model-shared";

  export type ProviderWrapStreamFnContext = {
    streamFn?: unknown;
    provider?: string;
    modelId?: string;
    [key: string]: unknown;
  };

  export type OpenClawPluginApi = {
    registerProvider: (provider: ProviderPlugin) => void;
    registerModelCatalogProvider: (provider: unknown) => void;
    [key: string]: unknown;
  };
}
