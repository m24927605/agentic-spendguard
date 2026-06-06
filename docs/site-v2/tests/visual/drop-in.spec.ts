import { test, expect } from '@playwright/test';

/**
 * D03 SLICE 2 — drop-in landing visual regression baseline.
 *
 * Loads `/docs/drop-in/`, waits for the page to render fully, and
 * compares the viewport screenshot against the committed baseline.
 * Tolerance is set globally in `playwright.config.ts` to 1% pixel diff
 * (`maxDiffPixelRatio: 0.01`).
 *
 * Baselines are captured per-project (desktop-chromium @ 1280x800,
 * mobile-chromium @ 375x812) — viewport, not full-page, because the
 * full-page height drifts between runs as the Pagefind index loads
 * and table-of-contents hydrates. The viewport-locked baseline still
 * exercises the hero / "How Pattern 2 works" callout / start-the-proxy
 * code block, which is the surface a first-time reader sees in the
 * 30-second window the landing is optimized for.
 *
 * To refresh after intentional copy changes:
 *
 *   npx playwright test --update-snapshots
 */
test.describe('Drop-in landing visual regression', () => {
  test('matches the committed baseline screenshot', async ({ page }) => {
    await page.goto('/docs/drop-in/');

    // Wait for the H1 to confirm the page rendered, then for any
    // late-loading assets (Pagefind script registration, syntax
    // highlighting hydration) to settle.
    await expect(
      page.getByRole('heading', { level: 1, name: /drop in spendguard in 30 seconds/i }),
    ).toBeVisible();
    await page.waitForLoadState('networkidle');

    await expect(page).toHaveScreenshot('drop-in-landing.png');
  });
});
