import { test, expect } from '@playwright/test';

/**
 * COV_D11 SLICE 7 — LiteLLM proxy guardrail docs page.
 *
 * Verifies the operator-facing docs page renders and that the LOCKED
 * §7.3 Blocker disclosure (INV-5 end-of-stream commit + no
 * token-by-token cap) is visible in the SSR'd output — not just lurking
 * in a `<meta>` tag or a `display: none` block.
 *
 * Per review-standards §7.3, the page MUST disclose:
 *   - "INV-5 end-of-stream commit"
 *   - "no token-by-token cap"
 *
 * Both as literal user-visible text. The grep-on-dist gate in the slice
 * verify script catches drift in the static output; these Playwright
 * tests catch drift in the rendered DOM (e.g. a future MDX directive
 * that hides the limitations block behind a collapsed `<details>`).
 */

const LITELLM_PROXY_DOCS = '/docs/integrations/litellm-proxy/';

test.describe('LiteLLM proxy guardrail docs page', () => {
  test('page renders 200 with the H1 visible', async ({ page }) => {
    const response = await page.goto(LITELLM_PROXY_DOCS);
    expect(response?.status()).toBe(200);
    await expect(
      page.getByRole('heading', { level: 1, name: /litellm proxy guardrail/i }),
    ).toBeVisible();
  });

  test('Install + Quick start sections render as H2 headings', async ({ page }) => {
    await page.goto(LITELLM_PROXY_DOCS);
    await expect(
      page.getByRole('heading', { level: 2, name: /^install$/i }),
    ).toBeVisible();
    await expect(
      page.getByRole('heading', { level: 2, name: /^quick start$/i }),
    ).toBeVisible();
    // The canonical extras name MUST appear in the Install section.
    // Pinned because SLICE 5 R1 deviation #1 locked it as
    // `litellm-guardrail` (not `litellm-proxy`). Shiki splits the code
    // block into per-token spans so `getByText` with an exact match
    // does not work; use regex against the rendered code block element
    // instead.
    await expect(
      page.locator('pre').filter({ hasText: /spendguard-sdk\[litellm-guardrail\]/ }).first(),
    ).toBeVisible();
  });

  test('Limitations section discloses INV-5 + no token-by-token cap (review-standards §7.3 Blocker)', async ({ page }) => {
    await page.goto(LITELLM_PROXY_DOCS);
    await expect(
      page.getByRole('heading', { level: 2, name: /^limitations$/i }),
    ).toBeVisible();
    // Both LOCKED disclosure strings MUST be present as user-visible
    // text. If a future refactor moves the limitations block behind a
    // collapsed `<details>` or strips one of the phrases, this test
    // fails — that is the intended behaviour.
    await expect(
      page.getByText(/INV-5 end-of-stream commit/),
    ).toBeVisible();
    await expect(
      page.getByText(/no token-by-token cap/),
    ).toBeVisible();
  });

  test('Configuration env-var table lists the two required vars', async ({ page }) => {
    await page.goto(LITELLM_PROXY_DOCS);
    await expect(
      page.getByRole('heading', { level: 2, name: /^configuration$/i }),
    ).toBeVisible();
    // Spot-check the two mandatory env vars are present in a rendered
    // `<table>`. The env-var name renders as `<code>SPENDGUARD_…</code>`
    // inside a `<td>` so we look for the code element by exact text.
    const tenantCode = page.locator('table code', { hasText: /^SPENDGUARD_TENANT_ID$/ }).first();
    const addressCode = page.locator('table code', { hasText: /^SPENDGUARD_SIDECAR_ADDRESS$/ }).first();
    await expect(tenantCode).toBeVisible();
    await expect(addressCode).toBeVisible();
  });
});
