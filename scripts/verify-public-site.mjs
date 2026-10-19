import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const base = process.argv[2] || 'https://vertex-ded-sec.vercel.app';
assert.equal(new URL(base).protocol, 'https:');
const output = fileURLToPath(new URL('../test-results/', import.meta.url));
await mkdir(output, { recursive: true });
const browser = await chromium.launch(process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
try {
  for (const [name, width, height] of [['desktop', 1440, 1000], ['mobile', 390, 844], ['small', 320, 760]]) {
    const page = await browser.newPage({ viewport: { width, height } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    page.on('response', response => { if (response.status() >= 400) errors.push(`${response.status()} ${response.url()}`); });
    const response = await page.goto(base, { waitUntil: 'networkidle' });
    assert.equal(response.status(), 200);
    assert.match(await page.locator('body').innerText(), /by ded_sec/);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    const broken = await page.locator('img').evaluateAll(async images => {
      await Promise.all(images.map(async image => {
        image.loading = 'eager';
        if (image.getAttribute('src')) await image.decode().catch(() => {});
      }));
      return images.filter(image => image.getAttribute('src') && image.naturalWidth === 0).map(image => image.src);
    });
    assert.deepEqual(broken, []);
    for (const element of await page.locator('[data-reveal]').all()) {
      await element.scrollIntoViewIfNeeded();
      await page.waitForTimeout(450);
    }
    await page.evaluate(() => scrollTo(0, 0));
    await page.screenshot({ path: join(output, `public-${name}.png`), fullPage: true });
    assert.deepEqual(errors, []);
    await page.close();
  }
  const request = await browser.newContext();
  for (const [path, status] of [['/search', 200], ['/api/release', 200], ['/panel', 404], ['/.env.local', 404]]) {
    const response = await request.request.get(base + path);
    assert.equal(response.status(), status, path);
    if (path === '/api/release') assert.deepEqual(await response.json(), { available: false });
  }
  console.log(JSON.stringify({ site: base, viewports: 3, assets: 'ok', private_routes: 'not published' }));
} finally { await browser.close(); }
