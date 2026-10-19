import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/browser',
  fullyParallel: false,
  workers: 1,
  timeout: 30000,
  reporter: 'list',
  use: {
    baseURL: 'http://127.0.0.1:4319',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    ...(process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {}),
  },
  projects: [
    { name: 'desktop', use: { viewport: { width: 1280, height: 900 } } },
    { name: 'mobile', use: { viewport: { width: 390, height: 844 }, isMobile: true } },
    { name: 'small', use: { viewport: { width: 320, height: 760 }, isMobile: true } },
  ],
  webServer: {
    command: 'node tests/fixture-server.mjs',
    url: 'http://127.0.0.1:4319/health',
    reuseExistingServer: false,
    timeout: 30000,
  },
});
