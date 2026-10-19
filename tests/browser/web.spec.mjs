import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.locator('#results')).toContainText('Aucune recherche');
  page._vertexErrors = errors;
});
test.afterEach(async ({ page }) => { expect(page._vertexErrors).toEqual([]); });

test('public search, safe reader, focus and mobile layout', async ({ page }, info) => {
  await expect(page.locator('#crawlBtn')).toBeHidden();
  await expect(page.locator('[data-tab="inbox"]')).toBeHidden();
  await page.getByRole('searchbox').fill('Rust');
  await page.getByRole('button', { name: 'Rechercher', exact: true }).click();
  await expect(page.locator('#results article')).toHaveCount(1);
  await page.locator('.act-read').click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.locator('#vBody')).toContainText('Document Rust');
  expect(await page.evaluate(() => !!window.vertexXss)).toBe(false);
  await expect(page.locator('#vBody script, #vBody [onerror], #vBody img[src^="http://127"]')).toHaveCount(0);
  await page.keyboard.press('Tab');
  expect(await page.evaluate(() => !!document.activeElement.closest('#view'))).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('reader.png') });
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toBeHidden();
  await expect(page.locator('.act-read')).toBeFocused();
  await page.screenshot({ path: info.outputPath('search.png') });
});

test('bookmark can be removed after a new search, and persists only in this browser', async ({ page, browser }) => {
  await page.getByRole('button', { name: 'Rust', exact: true }).click();
  await page.locator('.act-bm').click();
  await page.reload();
  await page.getByRole('button', { name: 'Rust', exact: true }).click();
  await expect(page.locator('.act-bm')).toHaveAttribute('data-on', '1');
  await page.locator('.act-bm').click();
  await page.locator('[data-tab="bookmarks"]').click();
  await expect(page.locator('#bookmarkList')).toContainText('Aucun signet');
  const other = await browser.newContext();
  const visitor = await other.newPage();
  await visitor.goto('http://127.0.0.1:4319/');
  expect(await visitor.evaluate(() => localStorage.getItem('vertex.bookmarks'))).toBeNull();
  await other.close();
});

test('contact and API denial are real, not hidden controls only', async ({ page }) => {
  expect((await page.request.get('/api/contact')).status()).toBe(401);
  expect((await page.request.post('/api/crawl', { data: { duration: 30 } })).status()).toBe(401);
  await page.locator('[data-tab="contact"]').click();
  await page.getByLabel('Sujet', { exact: true }).fill('Essai navigateur');
  await page.getByLabel('Message *', { exact: true }).fill('Message de verification navigateur');
  await page.getByRole('button', { name: 'Envoyer', exact: true }).click();
  await expect(page.locator('#contactState')).toContainText('Message enregistré');
  await expect(page.locator('#contactSend')).toBeEnabled();
});

test('empty, errors and long content remain usable', async ({ page }) => {
  await page.getByRole('searchbox').fill('missingword');
  await page.getByRole('button', { name: 'Rechercher', exact: true }).click();
  await expect(page.locator('#results')).toContainText('Aucun résultat');
  await page.getByRole('searchbox').fill('longword');
  await page.getByRole('button', { name: 'Rechercher', exact: true }).click();
  await page.locator('.act-read').click();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.keyboard.press('Escape');
  await page.route('**/api/search?**', route => route.fulfill({ status: 503, json: { error: 'Service temporairement indisponible' } }));
  await page.getByRole('button', { name: 'Rust', exact: true }).click();
  await expect(page.locator('#meta')).toContainText('Service temporairement indisponible');
  await expect(page.locator('#more')).toBeEnabled();
});

test('presentation shows the real product and does not offer stale installers', async ({ page }, info) => {
  await page.goto('/presentation');
  await expect(page.getByRole('heading', { level: 1 })).toContainText('Vertex');
  expect(await page.locator('img.preview').evaluate(img => img.complete && img.naturalWidth > 0)).toBe(true);
  await expect(page.locator('a[href^="/downloads/"]')).toHaveCount(0);
  await expect(page.locator('#releaseState')).toContainText('Nouvelle version');
  await expect(page.locator('#downloadButton')).toBeHidden();
  await expect(page.locator('a[href*="github.com"]')).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('presentation.png'), fullPage: true });
});

test('missing public URL is queued only once, a word search does not enqueue', async ({ page }) => {
  const proposals = [];
  page.on('request', request => { if (new URL(request.url()).pathname === '/api/discover') proposals.push(request.url()); });
  await page.getByRole('searchbox').fill('nothing-found-in-this-index');
  await page.getByRole('button', { name: 'Rechercher', exact: true }).click();
  await expect(page.locator('#results')).toContainText('Aucun résultat');
  expect(proposals).toHaveLength(0);
  await page.getByRole('searchbox').fill('https://example.com/new-public-page');
  await page.getByRole('button', { name: 'Rechercher', exact: true }).click();
  await expect(page.locator('#meta')).toContainText('file partagée');
  await expect(page.locator('#contributionSettings')).toBeHidden();
});

test('valid release enables only the compiled installer, malformed release stays unavailable', async ({ page }) => {
  await page.route('**/api/release', route => route.fulfill({json: {available:true, version:'0.2.0', windows:{url:'/downloads/windows',sha256:'a'.repeat(64),size:10485760}}}));
  await page.goto('/presentation');
  await expect(page.locator('#downloadButton')).toBeVisible();
  await expect(page.locator('#downloadLabel')).toContainText('0.2.0');
  await expect(page.locator('#checksum')).toContainText('SHA-256');
  await page.route('**/api/release', route => route.fulfill({json: {available:true, version:'bad',windows:{url:'https://untrusted.example/installer.exe'}}}));
  await page.reload();
  await expect(page.locator('#downloadButton')).toBeHidden();
  await expect(page.locator('#releaseState')).toContainText('indisponible');
});

test('highlight escapes markup and regexp tokens, tabs work with the keyboard', async ({ page }) => {
  const result = await page.evaluate(() => {
    const node = document.createElement('div');
    node.innerHTML = window.highlight('<img src=x onerror=alert(1)> & [ ( a+b', ['img', '&', '[', '(', 'a+b']);
    return { images: node.querySelectorAll('img').length, text: node.textContent };
  });
  expect(result.images).toBe(0);
  expect(result.text).toContain('<img');
  const first = page.locator('[data-tab="results"]');
  await first.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.locator('[data-tab="bookmarks"]')).toBeFocused();
  await expect(page.locator('#panel-bookmarks')).toBeVisible();
});
