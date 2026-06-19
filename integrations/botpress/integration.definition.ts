// Botpress integration definition for @spendguard/botpress-integration.
//
// SpendGuard is an LLM-provider integration: Botpress invokes the `llm`
// interface's `generateContent` action whenever a bot needs an LLM completion,
// and `listLanguageModels` to populate the model picker. This is the ONLY
// supported gate point — the first-party OpenAI integration hardcodes its
// endpoint and ignores HTTPS_PROXY, so a network-layer interposer is not an
// option. SpendGuard implements `generateContent` itself: reserve budget with
// the sidecar, forward to the configured upstream provider, commit real usage.
//
// We adopt the FORMAL Botpress `llm` interface via `.extend(llm, ...)` so
// Botpress Studio recognises this integration as an LLM provider (and shows it
// in the model picker / agent LLM selector). The `generateContent` /
// `listLanguageModels` actions + their input/output schemas now come from the
// vendored interface package at `bp_modules/llm` (pulled via `bp add llm`,
// committed for CI since `bp add` needs Botpress Cloud auth). The only entity
// the interface requires us to supply is `modelRef` — its action schemas
// reference `modelRef` via `z.ref("modelRef")`, so we bind our concrete
// `modelRef` schema into the interface with the extension builder below.
//
// `bp build` reads this file (default `integration.definition.ts`) and the
// entry point `src/index.ts`, then generates the `.botpress/` typings the
// runtime in `src/index.ts` imports as `import * as bp from '.botpress'`. The
// generated action input/output types are derived from the interface schemas,
// so the action implementations in `src/llm/*` must conform to them exactly.
import { IntegrationDefinition, z } from "@botpress/sdk";
import llm from "./bp_modules/llm";
import { ConfigurationObjectSchema } from "./src/config";
import { LanguageModelIdSchema } from "./src/llm/schemas";
import { VERSION } from "./src/version";

export default new IntegrationDefinition({
  name: "spendguard",
  version: VERSION,
  title: "SpendGuard",
  description:
    "Fail-closed LLM budget guardrail. Reserves spend with the SpendGuard sidecar before forwarding to the upstream provider (OpenAI / Anthropic / Bedrock), then commits real usage. DENY / DEGRADE / config errors fail the call with a Botpress RuntimeError.",
  readme: "README.md",
  configuration: {
    schema: ConfigurationObjectSchema,
  },
  // The `llm` interface's action schemas reference the `modelRef` entity via
  // `z.ref("modelRef")`. We declare a concrete `modelRef` whose `id` is our
  // provider-qualified language-model id; `.extend(llm, ...)` below binds it
  // into the interface. Mirrors the first-party OpenAI integration:
  // `entities: { modelRef: { schema: z.object({ id: <languageModelId> }) } }`.
  entities: {
    modelRef: {
      schema: z.object({
        id: LanguageModelIdSchema,
      }),
    },
  },
}).extend(llm, ({ entities: { modelRef } }) => ({
  entities: { modelRef },
}));
