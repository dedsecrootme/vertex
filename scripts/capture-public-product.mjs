import { chromium } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';

// Human-authorized public captures. Candidates must be reviewed before publication.
const base = 'http://127.0.0.1:3108';
const allowed = new Set(['blog.rust-lang.org', 'rust-lang.github.io', 'doc.rust-lang.org', 'www.rust-lang.org']);
const data = await (await fetch(base + '/api/search?q=Rust')).json();
const results = data.results.filter(row => {
  const url = new URL(row.url);
  return url.protocol === 'https:' && allowed.has(url.hostname) && !url.search && !url.username && !url.password;
});
if (!results.length) throw new Error('Aucun resultat public autorise');
const reader = results.find(row => row.url === 'https://blog.rust-lang.org/releases');
if (!reader) throw new Error('Document public de lecture absent');
const document = await (await fetch(base + '/api/page?' + new URLSearchParams({url: reader.url}))).json();
const sensitive = /[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}|api[_ -]?key\s*[:=]|access[_ -]?token\s*[:=]|password\s*[:=]|C:\\Users\\/i;
if (sensitive.test(JSON.stringify(results)) || sensitive.test((document.content || '') + (document.content_html || ''))) throw new Error('Contenu a verifier manuellement avant toute capture');
await mkdir('test-results', {recursive:true});
const browser = await chromium.launch({channel:'chrome',headless:true});
try {
  const page = await browser.newPage({viewport:{width:1280,height:900},deviceScaleFactor:1});
  await page.route('**/api/config', route => route.fulfill({json:{public:true,shared:false,discovery:false}}));
  await page.route('**/api/stats', async route => {
    const stats = await (await route.fetch()).json();
    await route.fulfill({json:{pages:stats.pages,domains:stats.domains,pending:stats.pending,blocked:stats.blocked}});
  });
  await page.route('**/api/search?**', route => route.fulfill({json:{query:'Rust',results}}));
  await page.route('**/api/favicon?**', route => route.fulfill({status:204}));
  await page.route('**/api/img?**', route => route.fulfill({status:204}));
  await page.goto(base);
  await page.getByRole('searchbox').fill('Rust');
  await page.getByRole('button',{name:'Rechercher',exact:true}).click();
  await page.locator('#results article').first().waitFor();
  if (await page.locator('[data-tab="inbox"]').isVisible()) throw new Error('Vue privee visible');
  await page.screenshot({path:'test-results/candidate-search-real.png'});
  const index = results.findIndex(row => row.url === reader.url);
  await page.locator('.act-read').nth(index).click();
  await page.getByRole('dialog').waitFor();
  await page.locator('#vTitle').filter({hasText:reader.title}).waitFor();
  await page.waitForFunction(() => document.querySelector('#vBody').getAttribute('aria-busy') === 'false');
  await page.screenshot({path:'test-results/candidate-reader-real.png'});
  await page.goto(base + '/panel');
  await page.locator('#content').waitFor({state:'visible'});
  const visible = await page.evaluate(() => [...document.querySelectorAll('h1,h2,strong,small,.worker p')]
    .filter(el => {const rect=el.getBoundingClientRect();return rect.bottom>0&&rect.top<innerHeight;})
    .map(el=>el.textContent).join('\n'));
  if (sensitive.test(visible) || /localhost|192\.168\.|127\.0\.0\./i.test(visible)) throw new Error('Donnee privee dans la zone du panneau');
  await page.screenshot({path:'test-results/candidate-crawler-real.png'});
  await writeFile('test-results/product-capture-review.json',JSON.stringify({capturedAt:new Date().toISOString(),publicSources:results.map(row=>row.url),privateViewsExcluded:true,candidateOnly:true},null,2));
  console.log('Three candidates saved under test-results; review required before copying to public assets.');
} finally {await browser.close();}
