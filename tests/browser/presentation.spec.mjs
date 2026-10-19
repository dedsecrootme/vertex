import {test,expect} from '@playwright/test';

test('premium presentation has working gallery, lightbox and keyboard controls',async({page})=>{
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto('/presentation');
  await expect(page.locator('.hero-art')).toBeVisible();
  expect(await page.locator('.hero-art').evaluate(img=>img.complete&&img.naturalWidth>0)).toBe(true);
  await page.getByRole('tab',{name:'Lecture',exact:true}).click();
  await expect(page.locator('#productPreview')).toHaveAttribute('src','/reader-view.png');
  await expect(page.getByRole('tab',{name:'Lecture',exact:true})).toHaveAttribute('aria-selected','true');
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab',{name:'Crawler',exact:true})).toBeFocused();
  await expect(page.locator('#productPreview')).toHaveAttribute('src','/crawler-view.png');
  await page.getByRole('button',{name:'Agrandir l’aperçu',exact:true}).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.locator('#largePreview')).toHaveAttribute('src',/crawler-view\.png$/);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toBeHidden();
  await expect(page.getByRole('button',{name:'Agrandir l’aperçu',exact:true})).toBeFocused();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  expect(errors).toEqual([]);
});

test('mobile navigation and reduced motion stay accessible',async({page})=>{
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.goto('/presentation');
  expect(await page.evaluate(()=>document.body.classList.contains('motion-enabled'))).toBe(false);
  const menu=page.getByRole('button',{name:'Ouvrir le menu',exact:true});
  if(await menu.isVisible()){
    await menu.click();
    await expect(page.locator('#menuButton')).toHaveAttribute('aria-expanded','true');
    await page.getByRole('navigation',{name:'Navigation principale'}).getByRole('link',{name:'Vos données'}).click();
    await expect(page.locator('#menuButton')).toHaveAttribute('aria-expanded','false');
    await expect(page).toHaveURL(/#confidentialite$/);
  }
  await page.locator('#privacy-title').scrollIntoViewIfNeeded();
  await expect(page.locator('#privacy-title')).toBeVisible();
});

test('gallery keeps a valid image after a failed request and can retry',async({page})=>{
  await page.route('**/reader-view.png',route=>route.abort());
  await page.goto('/presentation');
  await page.getByRole('tab',{name:'Lecture',exact:true}).click();
  await expect(page.locator('#previewError')).toBeVisible();
  await expect(page.locator('#productPreview')).toHaveAttribute('src','/search-view.png');
  await expect(page.getByRole('tab',{name:'Recherche',exact:true})).toHaveAttribute('aria-selected','true');
  await page.unroute('**/reader-view.png');
  await page.getByRole('button',{name:'Réessayer le chargement',exact:true}).click();
  await expect(page.locator('#productPreview')).toHaveAttribute('src','/reader-view.png');
  await expect(page.locator('#previewError')).toBeHidden();
  await expect(page.locator('#product-view')).toHaveAttribute('aria-busy','false');
});

test('a slow previous gallery request cannot replace the latest selection',async({page})=>{
  await page.route('**/reader-view.png',async route=>{
    await new Promise(resolve=>setTimeout(resolve,700));
    await route.continue();
  });
  await page.goto('/presentation');
  await page.getByRole('tab',{name:'Lecture',exact:true}).click();
  await page.getByRole('tab',{name:'Crawler',exact:true}).click();
  await expect(page.locator('#productPreview')).toHaveAttribute('src','/crawler-view.png');
  await page.waitForTimeout(850);
  await expect(page.locator('#productPreview')).toHaveAttribute('src','/crawler-view.png');
  await expect(page.getByRole('tab',{name:'Crawler',exact:true})).toHaveAttribute('aria-selected','true');
});

test('glass cards open their real gallery view without overflow',async({page})=>{
  await page.goto('/presentation');
  await expect(page.locator('.glass-card')).toHaveCount(3);
  for(const view of ['search','reader','crawler']){
    await page.locator(`[data-open-view="${view}"]`).click();
    await expect(page.locator('#productPreview')).toHaveAttribute('src',`/${view}-view.png`);
    await expect(page.locator(`#tab-${view}`)).toHaveAttribute('aria-selected','true');
  }
  await page.locator('.glass-card').first().scrollIntoViewIfNeeded();
  await expect(page.locator('.navigation')).toHaveClass(/is-scrolled/);
  expect(await page.locator('.navigation').evaluate(el=>getComputedStyle(el).backdropFilter)).not.toBe('none');
  const cards=await page.locator('.glass-card').evaluateAll(elements=>elements.map(el=>{
    const title=el.querySelector('h3').getBoundingClientRect();
    const paragraph=el.querySelector('p').getBoundingClientRect();
    return {fits:el.scrollWidth<=el.clientWidth,titleFits:title.bottom<=paragraph.top+1};
  }));
  expect(cards.every(card=>card.fits&&card.titleFits)).toBe(true);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});

test('optional animation failure never prevents gallery navigation',async({page})=>{
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.route('**/vendor/motion-mini.js',route=>route.abort());
  await page.goto('/presentation');
  await page.locator('[data-open-view="reader"]').click();
  await expect(page.locator('#productPreview')).toHaveAttribute('src','/reader-view.png');
  await page.getByRole('button',{name:'Agrandir l’aperçu',exact:true}).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  expect(errors).toEqual([]);
});

test('changing reduced motion stops active animations and leaves cards visible',async({page})=>{
  await page.goto('/presentation');
  await page.getByRole('tab',{name:'Lecture',exact:true}).click();
  await expect(page.locator('#productPreview')).toHaveAttribute('src','/reader-view.png');
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.locator('.glass-card').first().scrollIntoViewIfNeeded();
  expect(await page.locator('.glass-card').first().evaluate(el=>getComputedStyle(el).opacity)).toBe('1');
  expect(await page.evaluate(()=>document.getAnimations().filter(animation=>animation.playState==='running').length)).toBe(0);
});

test('presentation remains readable without JavaScript',async({browser,baseURL})=>{
  const context=await browser.newContext({javaScriptEnabled:false});
  const page=await context.newPage();
  try{
    await page.goto(baseURL+'/presentation');
    await expect(page.getByRole('heading',{name:'Vertex.',exact:true})).toBeVisible();
    for(const card of await page.locator('.glass-card').all()){
      await card.scrollIntoViewIfNeeded();
      expect(await card.evaluate(el=>getComputedStyle(el).opacity)).toBe('1');
    }
    await expect(page.getByRole('link',{name:'Vos données',exact:true}).last()).toBeVisible();
  }finally{await context.close();}
});
