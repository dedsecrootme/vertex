import { test, expect } from '@playwright/test';

test('panel requires admin and renders real data after login',async({page})=>{
  await page.goto('/panel');
  await expect(page.locator('#login')).toBeVisible();
  await page.getByLabel('Jeton administrateur').fill('browser-fixture-token-not-for-production-123456');
  await page.getByRole('button',{name:'Se connecter'}).click();
  await expect(page.locator('#content')).toBeVisible();
  await expect(page.locator('#pages')).toHaveText('2');
  await expect(page.locator('#online')).toHaveText('0');
  await page.getByRole('button',{name:'Base et vérification',exact:true}).click();
  await expect(page.locator('#database')).toContainText('6 / WAL');
  page.once('dialog',dialog=>dialog.accept());
  await page.getByRole('button',{name:'Vérifier la base',exact:true}).click();
  await expect(page.locator('#verificationStatus')).toHaveText('Contrôles réussis',{timeout:15000});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);
  await page.screenshot({path:`test-results/panel-${test.info().project.name}.png`,fullPage:true});
});

test('panel never reports stale workers as online and retains errors safely',async({page})=>{
  const snapshot = {
    at:1780000000, running:true,
    runtime:{fresh:false,age_seconds:45,online:0,data:{configured:12,mode:'drain',pid:123,workers:[{id:1,online:true,phase:'fetching',domain:'<img src=x onerror=alert(1)>'}]}},
    stats:{pages:40,pending:100,done:40,failed:2,blocked:0},
    skipped:0,total:142,runs:[],errors:[{at:'2026-10-19 00:00:00',message:'<script>bad()</script>'}],audit:null,
    database:{engine:'SQLite + FTS5',version:'test',schema:'6',journal:'wal',bytes:1024,wal_bytes:0},
    verification:{status:'not_run'},query_ms:1
  };
  await page.route('**/api/admin/monitor',route=>route.fulfill({json:snapshot}));
  await page.goto('/panel');
  await expect(page.locator('#online')).toHaveText('Signal absent');
  await expect(page.locator('.worker')).not.toHaveClass(/online/);
  await expect(page.locator('#errors')).toContainText('<script>bad()</script>');
  expect(await page.locator('#errors script').count()).toBe(0);
  await page.route('**/api/admin/monitor',route=>route.fulfill({status:503,json:{error:'Indisponible'}}));
  await page.getByRole('button',{name:'Actualiser',exact:true}).click();
  await expect(page.locator('#error')).toBeVisible();
  await expect(page.locator('#connection')).toHaveText('Connexion indisponible');
});

test('panel distinguishes live index growth, database location and cleanup counts',async({page})=>{
  let at=1780000000,pages=100;
  await page.route('**/api/admin/monitor',route=>route.fulfill({json:{
    at,running:true,runtime:{fresh:true,age_seconds:1,online:1,data:{configured:64,mode:'drain',pid:123,workers:[]}},
    stats:{pages,pending:1000,done:pages,failed:0,blocked:0},skipped:12,total:1112,runs:[],errors:[],audit:null,
    maintenance:{status:'waiting-for-writer',totalExcluded:12},
    database:{engine:'SQLite + FTS5',version:'test',schema:'6',journal:'wal',bytes:1024,wal_bytes:0,path:'D:\\Vertex\\database'},
    verification:{status:'not_run'},query_ms:1,
  }}));
  await page.goto('/panel');
  await expect(page.locator('#pages')).toHaveText('100');
  await page.locator('#auto').uncheck();
  at+=10;pages+=50;
  await page.getByRole('button',{name:'Actualiser',exact:true}).click();
  await expect(page.locator('#rate')).toContainText('5 page(s)/s');
  await expect(page.locator('#rate')).toContainText('moyenne 10 s');
  await page.getByRole('button',{name:'Base et vérification',exact:true}).click();
  await expect(page.locator('#database')).toContainText('D:\\Vertex\\database');
  await expect(page.locator('#database')).toContainText('Attente du verrou SQLite');
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});
