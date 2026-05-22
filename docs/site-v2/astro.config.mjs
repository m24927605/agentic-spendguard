// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  // Live URL. Drives the canonical <link>, absolute sitemap.xml URLs,
  // and OG image URLs. Custom domain is preserved via public/CNAME.
  site: 'https://agenticspendguard.dev',
  // Match the previous MkDocs URL shape (`/quickstart/`, not
  // `/quickstart`) so existing inbound links and search-engine
  // entries do not 404 after the cutover.
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
      // Mirror the previous MkDocs nav (`docs/site/mkdocs.yml`) so
      // information architecture survives the migration. Items are
      // grouped exactly as before; URL slugs match too.
      sidebar: [
        { label: 'Quickstart', slug: 'quickstart' },
        {
          label: 'Use cases',
          items: [
            { label: 'Pre-call budget caps', slug: 'use-cases/pre-call-budget-cap' },
            { label: 'Stop a runaway agent', slug: 'use-cases/agent-runaway-protection' },
            { label: 'Reservation pattern', slug: 'use-cases/reservation-pattern' },
          ],
        },
        {
          label: 'Concepts',
          items: [
            { label: '6-layer architecture', slug: 'concepts/architecture' },
            { label: 'Decision lifecycle', slug: 'concepts/decision-lifecycle' },
            { label: 'Audit chain', slug: 'concepts/audit-chain' },
            { label: 'Pricing & USD budget', slug: 'concepts/pricing-and-usd' },
          ],
        },
        {
          label: 'Deployment',
          items: [
            { label: 'Docker compose (POC)', slug: 'deployment/docker-compose' },
            { label: 'Helm chart (k8s)', slug: 'deployment/helm' },
            { label: 'Terraform (AWS)', slug: 'deployment/terraform-aws' },
          ],
        },
        {
          label: 'Authoring contracts',
          items: [
            { label: 'Contract YAML reference', slug: 'contracts/yaml' },
            { label: 'Rule examples', slug: 'contracts/examples' },
          ],
        },
        {
          label: 'Adapter integrations',
          items: [
            { label: 'Pydantic-AI', slug: 'integrations/pydantic-ai' },
            { label: 'LangChain & LangGraph', slug: 'integrations/langchain' },
            { label: 'OpenAI Agents SDK', slug: 'integrations/openai-agents' },
            { label: 'Microsoft AGT', slug: 'integrations/agt' },
          ],
        },
        {
          label: 'Operations',
          items: [
            { label: 'Dashboard', slug: 'operations/dashboard' },
            { label: 'Control plane API', slug: 'operations/control-plane' },
            { label: 'Data classification', slug: 'operations/data-classification' },
            { label: 'Multi-pod deployment', slug: 'operations/multi-pod' },
            { label: 'SLOs', slug: 'operations/slos' },
            {
              label: 'Drills',
              items: [
                { label: 'Approval TTL wave', slug: 'operations/drills/approval-ttl-wave' },
                { label: 'Audit chain forwarder backlog', slug: 'operations/drills/audit-chain-forwarder-backlog' },
                { label: 'Lease lost mid-batch', slug: 'operations/drills/lease-lost-mid-batch' },
                { label: 'Strict signature quarantine spike', slug: 'operations/drills/strict-signature-quarantine-spike' },
              ],
            },
          ],
        },
        {
          label: 'Roadmap',
          items: [
            { label: 'GA hardening slices', slug: 'roadmap/ga-hardening-slices' },
            { label: 'GA hardening progress', slug: 'roadmap/ga-hardening-progress' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Wire protocol (proto)', slug: 'reference/proto' },
            { label: 'Ledger schema', slug: 'reference/ledger-schema' },
            { label: 'Error codes', slug: 'reference/error-codes' },
          ],
        },
        { label: 'POC vs GA gates', slug: 'poc-vs-ga' },
      ],
    }),
  ],

  vite: {
    plugins: [tailwindcss()],
  },
});
