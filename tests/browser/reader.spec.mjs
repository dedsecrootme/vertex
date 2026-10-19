import { test, expect } from '@playwright/test';

async function open(page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Rust', exact: true }).click();
  await page.locator('.act-read').click();
  await expect(page.locator('#vBody')).toContainText('Document Rust');
}

test('reader is a full page, resizes text and restores focus on browser Back', async ({ page }) => {
  await open(page);
  const size = await page.locator('#view').boundingBox();
  expect(size.height).toBe(page.viewportSize().height);
  await expect(page.locator('#vTitle')).toHaveText('Guide Rust');
  await expect(page.locator('#vBody')).toHaveAttribute('aria-busy', 'false');
  const before = await page.locator('#vBody').evaluate(el => getComputedStyle(el).fontSize);
  await page.getByRole('button', { name: 'Agrandir le texte' }).click();
  expect(await page.locator('#vBody').evaluate(el => getComputedStyle(el).fontSize)).not.toBe(before);
  await page.goBack();
  await expect(page.getByRole('dialog')).toBeHidden();
  await expect(page.locator('.act-read')).toBeFocused();
  expect(await page.evaluate(() => document.body.style.overflow)).not.toBe('hidden');
});

test('reader displays a recoverable failure instead of an empty page', async ({ page }) => {
  await page.route('**/api/page?*', route => route.fulfill({ status: 503, json: { error: 'Indisponible' } }));
  await page.goto('/');
  await page.getByRole('button', { name: 'Rust', exact: true }).click();
  await page.locator('.act-read').click();
  await expect(page.locator('#vTitle')).toHaveText('Page indisponible');
  await expect(page.locator('#vBody')).toContainText('503');
  await expect(page.locator('#vBody')).toHaveAttribute('aria-busy', 'false');
  await page.getByRole('button', { name: 'Fermer la lecture' }).click();
  await expect(page.getByRole('dialog')).toBeHidden();
  await page.unroute('**/api/page?*');
  await page.locator('.act-read').click();
  await expect(page.locator('#vBody')).toContainText('Document Rust');
});

test('legacy text is rendered as paragraphs, never code or executable markup', async ({ page }) => {
  await page.route('**/api/page?*', async route => {
    const response = await route.fetch();
    const data = await response.json();
    data.content_html = '';
    data.content = 'Document Rust\n\nUn second paragraphe <img src=x onerror=bad()>.';
    await route.fulfill({ json: data });
  });
  await open(page);
  await expect(page.locator('#vBody p')).toHaveCount(2);
  await expect(page.locator('#vBody pre, #vBody img')).toHaveCount(0);
  await expect(page.locator('#vBody p').last()).toContainText('<img src=x onerror=bad()>');
});

test('long documents scroll as a page, progress works and layout stays bounded', async ({ page }, info) => {
  await page.route('**/api/page?*', async route => {
    const response = await route.fetch();
    const data = await response.json();
    data.content_html = '<h2>Un document complet</h2>' + '<p>Document Rust lisible avec des paragraphes, des liens et une mise en page continue.</p>'.repeat(60);
    await route.fulfill({ json: data });
  });
  await open(page);
  await page.locator('#view').evaluate(el => { el.scrollTop = el.scrollHeight; });
  await expect.poll(() => page.locator('#readingProgress').evaluate(el => el.value)).toBeGreaterThan(95);
  expect(await page.locator('#view').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
  expect(await page.locator('#vBody').evaluate(el => getComputedStyle(el).maxHeight)).toBe('none');
  await page.screenshot({ path: info.outputPath('reader-fullpage.png') });
});
