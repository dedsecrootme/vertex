import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';

const base = 'http://127.0.0.1:3108';
const exercise = process.argv.includes('--exercise-control');
async function monitor() {
  const response = await fetch(base + '/api/admin/monitor', { signal: AbortSignal.timeout(10000) });
  assert.equal(response.status, 200);
  return response.json();
}
async function waitForState(running) {
  const end = Date.now() + 180000;
  while (Date.now() < end) {
    const data = await monitor();
    const active = ['starting', 'running', 'paused-memory'].includes(data.supervisor?.status);
    if (running ? data.running && data.runtime?.fresh : !data.running && !active) return data;
    await delay(1000);
  }
  throw new Error(`Crawler did not reach running=${running}`);
}
await mkdir('test-results', { recursive: true });
const browser = await chromium.launch(process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
let stoppedByTest = false;
try {
  const page = await browser.newPage({ viewport: { width: 1366, height: 950 } });
  await page.goto(base + '/panel');
  await expect(page.locator('#content')).toBeVisible();
  await expect(page.locator('#crawlerControls')).toBeVisible();
  if (exercise) {
    let data = await monitor();
    if (!data.running) {
      await page.getByRole('button', { name: 'Démarrer', exact: true }).click();
      await waitForState(true);
      await page.getByRole('button', { name: 'Actualiser', exact: true }).click();
    }
    page.once('dialog', dialog => dialog.accept());
    await page.getByRole('button', { name: 'Arrêter', exact: true }).click();
    stoppedByTest = true;
    await waitForState(false);
    await page.getByRole('button', { name: 'Actualiser', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Démarrer', exact: true })).toBeEnabled({ timeout: 20000 });
    await page.getByRole('button', { name: 'Démarrer', exact: true }).click();
    data = await waitForState(true);
    stoppedByTest = false;
    assert.equal(data.control.available, true);
  }
  for (const [name, width, height] of [['desktop', 1366, 950], ['mobile', 390, 844], ['small', 320, 760]]) {
    await page.setViewportSize({ width, height });
    await page.getByRole('button', { name: 'Actualiser', exact: true }).click();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.screenshot({ path: `test-results/live-panel-${name}.png`, fullPage: true });
  }
  const data = await monitor();
  console.log(JSON.stringify({ running: data.running, engine: data.database.version, stats: data.stats,
    workers: data.runtime?.online, configured: data.runtime?.data?.configured, control: data.control.available }));
} finally {
  if (stoppedByTest) {
    const response = await fetch(base + '/api/admin/crawler', { method: 'POST', headers: { 'Content-Type': 'application/json', Origin: base }, body: JSON.stringify({ action: 'start', workers: 64 }) });
    if (!response.ok) console.error('Crawler restart refused; check the local panel.');
  }
  await browser.close();
}
