// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  site: 'https://agenticspendguard.dev',
  trailingSlash: 'always',
  integrations: [
    starlight({
      title: 'Agentic SpendGuard',
      description:
        'The spend firewall for LLM agents. Budget reserved before the provider is called, KMS-signed audit trail, p50 ≤10ms decision overhead. Works with LiteLLM, OpenAI Agents SDK, LangChain, LangGraph, Pydantic-AI, and Microsoft AGT.',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/m24927605/agentic-spendguard',
        },
      ],
      customCss: ['./src/styles/global.css'],
      // Docs surface lives entirely under /docs/*. The root `/` route is
      // owned by src/pages/index.astro (custom landing, no Starlight chrome).
      sidebar: [
        { label: 'Quickstart', slug: 'docs/quickstart' },
        {
          label: 'Use cases',
          items: [
            { label: 'Pre-call budget caps', slug: 'docs/use-cases/pre-call-budget-cap' },
            { label: 'Stop a runaway agent', slug: 'docs/use-cases/agent-runaway-protection' },
            { label: 'Reservation pattern', slug: 'docs/use-cases/reservation-pattern' },
          ],
        },
        {
          label: 'Concepts',
          items: [
            { label: '6-layer architecture', slug: 'docs/concepts/architecture' },
            { label: 'Decision lifecycle', slug: 'docs/concepts/decision-lifecycle' },
            { label: 'Audit chain', slug: 'docs/concepts/audit-chain' },
            { label: 'Pricing & USD budget', slug: 'docs/concepts/pricing-and-usd' },
          ],
        },
        {
          label: 'Deployment',
          items: [
            { label: 'Docker compose (POC)', slug: 'docs/deployment/docker-compose' },
            { label: 'Helm chart (k8s)', slug: 'docs/deployment/helm' },
            { label: 'Terraform (AWS)', slug: 'docs/deployment/terraform-aws' },
          ],
        },
        {
          label: 'Authoring contracts',
          items: [
            { label: 'Contract YAML reference', slug: 'docs/contracts/yaml' },
            { label: 'Rule examples', slug: 'docs/contracts/examples' },
          ],
        },
        {
          label: 'Adapter integrations',
          items: [
            { label: 'Pydantic-AI', slug: 'docs/integrations/pydantic-ai' },
            { label: 'LangChain & LangGraph', slug: 'docs/integrations/langchain' },
            { label: 'OpenAI Agents SDK', slug: 'docs/integrations/openai-agents' },
            { label: 'Microsoft AGT', slug: 'docs/integrations/agt' },
          ],
        },
        {
          label: 'Drop-in (Pattern 2)',
          items: [
            { label: 'Drop in 14 tools (overview)', slug: 'docs/drop-in' },
            { label: 'LiteLLM (proxy mode)', slug: 'docs/drop-in/litellm' },
            { label: 'Aider', slug: 'docs/drop-in/aider' },
            { label: 'Continue', slug: 'docs/drop-in/continue' },
            { label: 'Cline / Roo Code (BYOK)', slug: 'docs/drop-in/cline-roo-code' },
            { label: 'OpenHands (BYOK)', slug: 'docs/drop-in/openhands' },
            { label: 'Goose', slug: 'docs/drop-in/goose' },
            { label: 'Zed AI', slug: 'docs/drop-in/zed' },
            { label: 'GitHub Copilot CLI (BYOK)', slug: 'docs/drop-in/copilot-cli' },
            { label: 'Tabnine Enterprise', slug: 'docs/drop-in/tabnine' },
            { label: 'AnythingLLM', slug: 'docs/drop-in/anythingllm' },
            { label: 'LobeChat', slug: 'docs/drop-in/lobechat' },
            { label: 'Cody self-hosted Enterprise', slug: 'docs/drop-in/cody' },
            { label: 'Augment (BYOK)', slug: 'docs/drop-in/augment' },
            { label: 'Dify', slug: 'docs/drop-in/dify' },
          ],
        },
        {
          label: 'Operations',
          items: [
            { label: 'Dashboard', slug: 'docs/operations/dashboard' },
            { label: 'Control plane API', slug: 'docs/operations/control-plane' },
            { label: 'Data classification', slug: 'docs/operations/data-classification' },
            { label: 'Multi-pod deployment', slug: 'docs/operations/multi-pod' },
            { label: 'SLOs', slug: 'docs/operations/slos' },
            {
              label: 'Drills',
              items: [
                { label: 'Approval TTL wave', slug: 'docs/operations/drills/approval-ttl-wave' },
                { label: 'Audit chain forwarder backlog', slug: 'docs/operations/drills/audit-chain-forwarder-backlog' },
                { label: 'Lease lost mid-batch', slug: 'docs/operations/drills/lease-lost-mid-batch' },
                { label: 'Strict signature quarantine spike', slug: 'docs/operations/drills/strict-signature-quarantine-spike' },
              ],
            },
          ],
        },
        {
          label: 'Roadmap',
          items: [
            { label: 'GA hardening slices', slug: 'docs/roadmap/ga-hardening-slices' },
            { label: 'GA hardening progress', slug: 'docs/roadmap/ga-hardening-progress' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Wire protocol (proto)', slug: 'docs/reference/proto' },
            { label: 'Ledger schema', slug: 'docs/reference/ledger-schema' },
            { label: 'Error codes', slug: 'docs/reference/error-codes' },
          ],
        },
        { label: 'POC vs GA gates', slug: 'docs/poc-vs-ga' },
        {
          label: 'Specifications',
          items: [
            { label: 'Agent Spend Protocol — Draft 01', slug: 'docs/specs/agent-spend-protocol' },
            { label: 'OTel GenAI extension proposal', slug: 'docs/specs/otel-genai-extension' },
          ],
        },
        {
          label: 'Posts',
          items: [
            { label: 'The Agent Spend Governance Gap', slug: 'docs/posts/agent-spend-governance-gap' },
          ],
        },
      ],
    }),
  ],

  vite: {
    plugins: [tailwindcss()],
  },
});
