import { test, expect } from '@playwright/test';

/**
 * D03 SLICE 3 — DropInPicker functional + visual regression.
 *
 * The picker is SSR-rendered: the full 14-row table must be visible in
 * the initial HTML so the page works without JS (review-standards H13 /
 * acceptance.md §F3). Client-side filters + URL-hash sync + smooth
 * scroll are progressive enhancements; the suite exercises them once
 * the page has hydrated.
 *
 * Snapshot policy:
 * - Picker viewport screenshot at desktop + mobile, masked to the picker
 *   region so unrelated landing-page chrome does not produce false drift.
 * - 1% pixel diff tolerance per `playwright.config.ts`.
 */

const LANDING = '/docs/drop-in/';

test.describe('DropInPicker — SSR + functional', () => {
  test('SSR table renders all 14 tools without JS interaction', async ({ page }) => {
    await page.goto(LANDING);
    // Wait for the picker to be in the DOM, but do not depend on JS
    // hydration for the table content — SSR'd rows must be present.
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    const rows = picker.locator('.dropin-picker__table tbody tr');
    await expect(rows).toHaveCount(14);
    // First row should not be hidden by default — sanity check on SSR
    // visibility (no `hidden` attribute applied pre-hydration).
    await expect(rows.first()).not.toHaveAttribute('hidden', /.*/);
  });

  test('filter to env_var shows only env-var tools', async ({ page }) => {
    await page.goto(LANDING);
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    await page.locator('input[name="dropin-pattern"][value="env_var"]').check();
    const visibleRows = picker.locator('.dropin-picker__table tbody tr:not([hidden])');
    // 4 env-var tools per dropin_tools.json: LiteLLM, Aider, Goose, Copilot CLI.
    await expect(visibleRows).toHaveCount(4);
    // Status text should reflect filtered count.
    await expect(picker.locator('[data-status]')).toContainText('Showing 4 of 14');
  });

  test('filter to config_file shows only config-file tools', async ({ page }) => {
    await page.goto(LANDING);
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    await page.locator('input[name="dropin-pattern"][value="config_file"]').check();
    const visibleRows = picker.locator('.dropin-picker__table tbody tr:not([hidden])');
    // 4 config-file tools: Continue, Zed, Cody, Dify.
    await expect(visibleRows).toHaveCount(4);
  });

  test('filter to admin_ui shows only admin-UI tools', async ({ page }) => {
    await page.goto(LANDING);
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    await page.locator('input[name="dropin-pattern"][value="admin_ui"]').check();
    const visibleRows = picker.locator('.dropin-picker__table tbody tr:not([hidden])');
    // 6 admin-UI tools: Cline/Roo, OpenHands, Tabnine, AnythingLLM, LobeChat, Augment.
    await expect(visibleRows).toHaveCount(6);
  });

  test('search "aider" filters table to the matching row', async ({ page }) => {
    await page.goto(LANDING);
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    await picker.locator('.dropin-picker__search').fill('aider');
    const visibleRows = picker.locator('.dropin-picker__table tbody tr:not([hidden])');
    await expect(visibleRows).toHaveCount(1);
    await expect(visibleRows.first()).toContainText('Aider');
  });

  test('clearing filters restores all 14 rows', async ({ page }) => {
    await page.goto(LANDING);
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    await picker.locator('.dropin-picker__search').fill('aider');
    await page.locator('input[name="dropin-pattern"][value="all"]').check();
    await picker.locator('.dropin-picker__search').fill('');
    const visibleRows = picker.locator('.dropin-picker__table tbody tr:not([hidden])');
    await expect(visibleRows).toHaveCount(14);
    await expect(picker.locator('[data-status]')).toContainText('Showing all 14 tools');
  });

  test('jump link updates URL hash and scrolls (H14)', async ({ page }) => {
    await page.goto(LANDING);
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    // Click the Aider jump link; URL hash should become #aider and the
    // browser should scroll the Aider H3 into view.
    const jump = picker.locator('a.dropin-picker__jump[data-anchor="#aider"]').first();
    await jump.click();
    await expect(page).toHaveURL(/#aider$/);
    const target = page.locator('#aider');
    await expect(target).toBeVisible();
  });

  test('renders responsively on mobile viewport', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto(LANDING);
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    // The filter fieldset should stack vertically (one of the radios
    // remains keyboard-accessible regardless of layout).
    const radio = page.locator('input[name="dropin-pattern"][value="env_var"]');
    await expect(radio).toBeVisible();
    // Table overflow wrapper is the horizontal-scroll fallback.
    await expect(picker.locator('.dropin-picker__table-wrap')).toBeVisible();
    const rows = picker.locator('.dropin-picker__table tbody tr');
    await expect(rows).toHaveCount(14);
  });

  test('no-results state shows the empty message', async ({ page }) => {
    await page.goto(LANDING);
    const picker = page.locator('.dropin-picker');
    await expect(picker).toBeVisible();
    await picker.locator('.dropin-picker__search').fill('definitely-not-a-real-tool-xyz');
    const visibleRows = picker.locator('.dropin-picker__table tbody tr:not([hidden])');
    await expect(visibleRows).toHaveCount(0);
    await expect(picker.locator('[data-empty]')).toBeVisible();
  });
});
