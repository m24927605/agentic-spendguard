declare module "openclaw/plugin-sdk/provider-model-shared" {
  export type ProviderPluginCatalog = {
    order?: "simple" | "profile" | "paired" | "late" | string;
    run: (ctx: unknown) => Promise<unknown>;
  };

  export type OpenClawStreamFn = (params: unknown) => unknown | Promise<unknown>;

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
    createStreamFn?: (ctx: unknown) => OpenClawStreamFn | null | undefined;
    wrapStreamFn?: (ctx: unknown) => OpenClawStreamFn | null | undefined;
    [key: string]: unknown;
  };
}

declare module "openclaw/plugin-sdk/plugin-entry" {
  import type {
    OpenClawStreamFn,
    ProviderPlugin,
  } from "openclaw/plugin-sdk/provider-model-shared";

  export type ProviderWrapStreamFnContext = {
    streamFn?: OpenClawStreamFn;
    provider?: string;
    modelId?: string;
    model?: unknown;
    agentId?: string;
    config?: unknown;
    agentDir?: string;
    workspaceDir?: string;
    extraParams?: Record<string, unknown>;
    thinkingLevel?: unknown;
    [key: string]: unknown;
  };

  export type OpenClawPluginApi = {
    registerProvider: (provider: ProviderPlugin) => void;
    registerModelCatalogProvider: (provider: unknown) => void;
    [key: string]: unknown;
  };
}
