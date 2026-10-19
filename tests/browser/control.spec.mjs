import { test, expect } from '@playwright/test';

async function mockPanel(page) {
  const response = await page.request.get('/api/admin/monitor', { headers: { Authorization: 'Bearer browser-fixture-token-not-for-production-123456' } });
  const state = await response.json();
  state.control = { available: true, busy: false };
  state.running = false;
  state.supervisor = { status: 'stopped' };
  await page.route('**/api/admin/monitor', route => route.fulfill({ json: { ...state, at: Math.floor(Date.now() / 1000) } }));
  return state;
}

test('private control validates limits, starts and stops only after confirmation', async ({ page }) => {
  const state = await mockPanel(page);
  const commands = [];
  await page.route('**/api/admin/crawler', route => {
    const payload = route.request().postDataJSON();
    commands.push(payload);
    state.running = payload.action === 'start';
    state.supervisor = { status: state.running ? 'starting' : 'stopped', updatedAt: new Date().toISOString() };
    return route.fulfill({ status: 202, json: { accepted: true } });
  });
  await page.goto('/panel');
  await expect(page.locator('#crawlerControls')).toBeVisible();
  await page.locator('#workerLimit').fill('65');
  await page.locator('#startCrawler').click();
  expect(commands).toHaveLength(0);
  await page.locator('#workerLimit').fill('8');
  await page.locator('#startCrawler').click();
  await expect(page.locator('#startCrawler')).toBeDisabled();
  expect(commands).toEqual([{ action: 'start', workers: 8 }]);
  page.once('dialog', dialog => dialog.dismiss());
  await page.locator('#stopCrawler').click();
  expect(commands).toHaveLength(1);
  page.once('dialog', dialog => dialog.accept());
  await page.locator('#stopCrawler').click();
  await expect(page.locator('#commandStatus')).toHaveText('Arrêt propre demandé');
  expect(commands.at(-1)).toEqual({ action: 'stop' });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('command failures remain visible and never pretend a successful stop', async ({ page }) => {
  const state = await mockPanel(page);
  state.running = true;
  await page.route('**/api/admin/crawler', route => route.fulfill({ status: 503, json: { error: 'Lanceur indisponible' } }));
  await page.goto('/panel');
  page.once('dialog', dialog => dialog.accept());
  await page.locator('#stopCrawler').click();
  await expect(page.locator('#commandStatus')).toHaveText('Lanceur indisponible');
  await expect(page.locator('#startCrawler')).toBeDisabled();
});
