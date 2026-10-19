import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm, mkdir } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { setTimeout as delay } from 'node:timers/promises';
import { chromium } from '@playwright/test';

// Product illustrations only. Never read the user's index, bookmarks or messages.
const dir = await mkdtemp(join(tmpdir(), 'vertex-product-illustration-'));
await mkdir('test-results', {recursive:true});
const reservation = createServer();
reservation.listen(0, '127.0.0.1');
await once(reservation, 'listening');
const port = reservation.address().port;
await new Promise(done => reservation.close(done));
const env = {...process.env, VERTEX_DB_DIR: dir, VERTEX_PORT: String(port), VERTEX_BIND_HOST:'127.0.0.1', VERTEX_ALLOW_DISCOVERY:'0'};
delete env.VERTEX_PUBLIC_ORIGIN; delete env.VERTEX_ADMIN_TOKEN;
const child = spawn(resolve('target/debug/api-rs.exe'), [], {env, windowsHide:true, stdio:'ignore'});
let browser;
try {
  const base = `http://127.0.0.1:${port}`;
  for (let i=0;i<100;i++) {
    if (await fetch(base+'/health').then(r=>r.ok).catch(()=>false)) break;
    await delay(100);
  }
  const db = new DatabaseSync(join(dir,'vertex.db'));
  const insert = db.prepare('INSERT INTO pages(url,domain,title,description,content,content_html,language,status_code) VALUES(?,?,?,?,?,?,?,?)');
  const articles = [
    ['https://documentation.example/recherche','documentation.example','Une autre facon de parcourir le web','Recherche, lecture et exploration dans un meme espace.','Le web est un ensemble de pages et de liens. Un index permet de retrouver les documents deja explores, puis de passer du resultat a la lecture.'],
    ['https://journal.example/exploration','journal.example','Les chemins du web','Des liens aux pages : comprendre le parcours d’un crawler.','Explorer le web commence par une adresse. Le moteur visite les pages publiques autorisees et conserve les informations utiles a la recherche.'],
    ['https://guides.example/lecture','guides.example','Lire le web, garder le contexte','Une page et ses liens, dans une vue de lecture.','La lecture rassemble le texte du document et les liens references. Les signets permettent de garder une page a portee de main.'],
  ];
  for (const [url,domain,title,description,content] of articles) {
    insert.run(url,domain,title,description,content,`<h1>${title}</h1><p>${content}</p><h2>De la recherche a la lecture</h2><p>${description}</p><p>Chaque resultat conserve son adresse et les informations de la page indexee. Vous pouvez poursuivre votre recherche ou conserver le document dans vos signets.</p><h2>Un espace pour explorer</h2><p>Les donnees de cet apercu sont des illustrations. Elles ne proviennent pas d'un historique personnel.</p>`,'fr',200);
  }
  db.close();
  browser = await chromium.launch({channel:'chrome',headless:true});
  const page = await browser.newPage({viewport:{width:1280,height:900},deviceScaleFactor:1});
  await page.route('**/api/config',r=>r.fulfill({json:{public:true,shared:false,discovery:false}}));
  await page.route('**/api/favicon?**',r=>r.fulfill({status:204}));
  await page.goto(base);
  await page.getByRole('searchbox').fill('web');
  await page.getByRole('button',{name:'Rechercher',exact:true}).click();
  await page.locator('#results article').first().waitFor();
  await page.screenshot({path:'test-results/demo-search-view.png'});
  await page.locator('.act-read').first().click();
  await page.getByRole('dialog').waitFor();
  await page.screenshot({path:'test-results/demo-reader-view.png'});
  const now = Math.floor(Date.now()/1000);
  const stamp = seconds => new Date((now-seconds)*1000).toISOString().slice(0,19).replace('T',' ');
  await page.route('**/api/admin/monitor',r=>r.fulfill({json:{
    at:now,running:true,total:128000,skipped:2000,query_ms:12,
    stats:{pages:44800,pending:80000,done:44800,failed:1200,blocked:0},
    runtime:{fresh:true,age_seconds:1,online:12,data:{configured:12,mode:'drain',workers:Array.from({length:12},(_,i)=>({id:i+1,online:true,phase:i%4===0?'saving':'fetching',domain:['documentation.example','journal.example','guides.example'][i%3]}))}},
    runs:Array.from({length:12},(_,i)=>({id:42-i,started_at:stamp(60*(i+1)),finished_at:stamp(60*i),ok:80+(i*13)%65,failed:i%4,total:85+(i*13)%65,status:'done'})),
    errors:[],audit:null,database:{engine:'SQLite + FTS5',version:'3.45',schema:'6',journal:'wal',bytes:1024**3,wal_bytes:12*1024**2,free_bytes:80*1024**3},verification:{status:'not_run'}
  }}));
  await page.goto(base+'/panel');
  await page.locator('#content').waitFor({state:'visible'});
  await page.screenshot({path:'test-results/demo-crawler-view.png'});
  console.log('Three product illustrations captured from an isolated database. No real user data read.');
} finally {
  if(browser) await browser.close();
  if(child.exitCode===null) {const stopped=once(child,'exit');child.kill();await stopped;}
  await rm(dir,{recursive:true,force:true});
}
