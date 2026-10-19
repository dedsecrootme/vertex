import {chromium} from '@playwright/test';
import {mkdir} from 'node:fs/promises';
import assert from 'node:assert/strict';
import {request} from 'node:http';

async function previewRequest(path,options={}){
  return new Promise((resolve,reject)=>{
    const req=request({hostname:'127.0.0.1',port:3110,path,...options},response=>{
      response.resume();response.on('end',()=>resolve(response.statusCode));
    });
    req.on('error',reject);req.setTimeout(5000,()=>req.destroy(new Error('Preview request timeout')));req.end();
  });
}
assert.equal(await previewRequest('/',{headers:{Host:'evil.example'}}),403);
assert.equal(await previewRequest('/',{headers:{Origin:'https://evil.example'}}),403);
assert.equal(await previewRequest('/',{method:'POST'}),405);
assert.equal(await previewRequest('/database/vertex.db'),404);
assert.equal(await previewRequest('/Cargo.lock'),404);
assert.equal(await previewRequest('//['),400);
assert.equal(await previewRequest('/'),200);
console.log('Preview security checks passed: Host, Origin, methods, private files and malformed URL recovery.');

await mkdir('test-results',{recursive:true});
const browser=await chromium.launch({channel:'chrome'});
try{
  for(const [name,width,height] of [['desktop',1366,900],['wide',1920,1080],['laptop',1366,768],['mobile',390,844],['small',320,760]]){
    const page=await browser.newPage({viewport:{width,height}});
    const errors=[],violations=[],external=[];
    page.on('pageerror',error=>errors.push(error.message));
    page.on('request',request=>{if(new URL(request.url()).hostname!=='127.0.0.1')external.push(request.url());});
    await page.addInitScript(()=>{window.cspErrors=[];document.addEventListener('securitypolicyviolation',event=>window.cspErrors.push(event.violatedDirective));});
    await page.goto('http://127.0.0.1:3110/');
    await page.waitForTimeout(1800);
    assert.equal(await page.locator('.hero-art').evaluate(img=>img.complete&&img.naturalWidth>0),true);
    const productTop=await page.locator('#produit').evaluate(el=>el.getBoundingClientRect().top);
    assert.ok(productTop<height,`${name}: next section must remain visible`);
    await page.screenshot({path:`test-results/glass-${name}-top.png`});
    for(const card of await page.locator('.glass-card').all()){
      await card.scrollIntoViewIfNeeded();
      await page.waitForTimeout(1000);
      assert.equal(await card.locator('img').evaluate(img=>img.complete&&img.naturalWidth>0),true);
    }
    await page.locator('#exploration').scrollIntoViewIfNeeded();
    await page.waitForTimeout(800);
    await page.screenshot({path:`test-results/glass-${name}-cards.png`});
    for(const view of ['reader','crawler','search']){
      await page.locator(`[data-open-view="${view}"]`).click();
      await page.waitForFunction(view=>document.querySelector('#productPreview').getAttribute('src')===`/${view}-view.png`,view);
      await page.waitForTimeout(500);
    }
    await page.locator('#download').scrollIntoViewIfNeeded();
    await page.waitForTimeout(1000);
    await page.evaluate(()=>scrollTo(0,0));
    await page.waitForTimeout(700);
    await page.screenshot({path:`test-results/glass-${name}-full.png`,fullPage:true});
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
    violations.push(...await page.evaluate(()=>window.cspErrors));
    assert.deepEqual({errors,violations,external},{errors:[],violations:[],external:[]});
    console.log(JSON.stringify({name,width,height,productTop,errors,violations,external}));
    await page.close();
  }
}finally{await browser.close();}
