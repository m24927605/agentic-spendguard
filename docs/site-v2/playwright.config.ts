import { defineConfig, devices } from '@playwright/test';

/**
 * Visual regression config for the docs site. Spins up Astro preview
 * (built artifacts from `npm run build`) on port 4321 and runs the
 * drop-in landing screenshot baseline against it.
 *
 * Snapshot policy (D03 SLICE 2):
 * - 1% pixel diff tolerance (`maxDiffPixelRatio: 0.01`).
 * - 1280x800 desktop + 375x812 mobile baselines.
 * - Baselines live under `tests/visual/drop-in.spec.ts-snapshots/` next to
 *   the spec (Playwright default `snapshotPathTemplate`).
 *
 * Update baselines with:
 *   npx playwright test --update-snapshots
 */
export default defineConfig({
  testDir: './tests/visual',
  fullyParallel: true,
  reporter: 'list',
  use: {
    baseURL: 'http://127.0.0.1:4321',
  },
  expect: {
    toHaveScreenshot: {
      maxDiffPixelRatio: 0.01,
    },
  },
  projects: [
    {
      name: 'desktop-chromium',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1280, height: 800 },
      },
    },
    {
      name: 'mobile-chromium',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 375, height: 812 },
        deviceScaleFactor: 2,
        isMobile: true,
        hasTouch: true,
      },
    },
  ],
  webServer: {
    command: 'npm run preview -- --host 127.0.0.1 --port 4321',
    url: 'http://127.0.0.1:4321/docs/drop-in/',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
