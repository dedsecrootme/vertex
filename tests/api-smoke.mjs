import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { request } from 'node:http';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { setTimeout as delay } from 'node:timers/promises';

const reservation = createServer();
reservation.listen(0, '127.0.0.1');
await once(reservation, 'listening');
const port = reservation.address().port;
await new Promise(done => reservation.close(done));
const dir = await mkdtemp(join(tmpdir(), 'vertex-api-smoke-'));
const binary = resolve('target', 'debug', process.platform === 'win32' ? 'api-rs.exe' : 'api-rs');
const publicMode = process.argv.includes('--public');
const token = 'public-smoke-test-token-12345678901234567890';
const env = { ...process.env, VERTEX_DB_DIR: dir, VERTEX_PORT: String(port), VERTEX_ALLOW_DISCOVERY: '1' };
env.VERTEX_LOCAL_CONTROL = '0';
delete env.VERTEX_PUBLIC_ORIGIN;
delete env.VERTEX_ADMIN_TOKEN;
if (publicMode) { env.VERTEX_PUBLIC_ORIGIN = 'https://vertex.example'; env.VERTEX_ADMIN_TOKEN = token; }
const child = spawn(binary, [], {
  env,
  windowsHide: true,
  stdio: 'ignore',
});
let launchError;
child.on('error', error => { launchError = error; });
const base = `http://127.0.0.1:${port}`;
const send = (path, options = {}) => publicMode ? new Promise((resolve, reject) => {
  const req = request(base + path, { method: options.method || 'GET',
    headers: { Host: 'vertex.example', ...options.headers }, timeout: 5000 }, response => {
    const chunks = [];
    response.on('data', chunk => chunks.push(chunk));
    response.on('end', () => resolve(new Response(Buffer.concat(chunks), { status: response.statusCode, headers: response.headers })));
    response.on('error', reject);
  });
  req.on('error', reject);
  req.on('timeout', () => req.destroy(new Error('API timeout')));
  req.end(options.body);
}) : fetch(base + path, { ...options, signal: AbortSignal.timeout(5000) });
const post = (path, body) => send(path, {
  method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
});

try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    if (launchError) throw launchError;
    ready = await send('/health').then(r => r.ok).catch(() => false);
    if (ready) break;
    await delay(50);
  }
  assert.ok(ready, 'API startup failed');
  const db = new DatabaseSync(join(dir, 'vertex.db'));
  db.prepare('INSERT INTO pages(url, title, content, content_html) VALUES (?, ?, ?, ?)')
    .run('https://example.com/guide', 'Rust guide', 'Rust crawler tutorial',
      '<script>bad()</script><p>Rust</p><img src="http://127.0.0.1/secret" onerror="bad()">');
  db.close();

  if(publicMode) {
    const response=await send('/api/admin/crawler',{method:'POST',headers:{'Content-Type':'application/json',Authorization:'Bearer '+token},body:JSON.stringify({action:'start',workers:64})});
    assert.equal(response.status,403,'Even a public admin cannot execute commands on the PC');
  }else{
    assert.equal((await post('/api/admin/crawler',{action:'stop'})).status,process.platform==='win32'?403:501);
    assert.equal((await post('/api/admin/crawler',{action:'start',workers:65})).status,400);
    assert.equal((await post('/api/admin/crawler',{action:'start; calc.exe'})).status,422);
    assert.equal((await post('/api/admin/crawler',{action:'start',script:'other.ps1'})).status,422);
  }
  const found = await (await send('/api/search?q=Rust')).json();
  assert.equal(found.results.length, 1);
  assert.equal((await (await send('/api/search?q=' + encodeURIComponent('https://example.com/guide#section'))).json()).results.length, 1);
  assert.equal((await (await post('/api/discover', {url: 'https://example.com/guide'})).json()).status, 'ready');
  for (let i = 0; i < 2; i++) assert.equal((await (await post('/api/discover', {url: 'https://example.com/new#fragment'})).json()).status, 'queued');
  assert.equal((await post('/api/discover', {url: 'http://127.0.0.1/private'})).status, 400);
  assert.equal((await post('/api/discover', {url: 'https://user:pass@example.com/'})).status, 400);
  const queueDb = new DatabaseSync(join(dir, 'vertex.db'));
  assert.equal(queueDb.prepare("SELECT count(*) AS n FROM urls WHERE url='https://example.com/new'").get().n, 1);
  assert.equal(queueDb.prepare("SELECT count(*) AS n FROM urls WHERE url='https://example.com/guide'").get().n, 0);
  queueDb.close();
  const page = await (await send('/api/page?url=https%3A%2F%2Fexample.com%2Fguide')).json();
  assert.ok(page.content_html.includes('<p>Rust</p>'));
  assert.ok(!/<script|onerror|<img src=/.test(page.content_html));
  assert.equal((await send('/api/search?q=%22%20OR%20title%3Ax')).status, 200);

  const health = await send('/health');
  assert.equal(health.headers.get('x-content-type-options'), 'nosniff');
  assert.ok(health.headers.get('content-security-policy').includes("script-src 'self';"));
  assert.equal((await send('/api/search?q=' + 'a'.repeat(257))).status, 400);
  assert.equal((await send('/api/search?q=Rust&offset=30001')).status, 400);
  assert.equal((await post('/api/contact', { message: 'x'.repeat(8001) })).status, 400);
  assert.equal((await send('/api/contact', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ message: 'x'.repeat(20000) }) })).status, 413);

  if (publicMode) {
    assert.equal((await send('/', { headers: { 'Sec-Fetch-Site': 'cross-site' } })).status, 200);
    assert.equal((await send('/api/search?q=Rust', { headers: { 'Sec-Fetch-Site': 'cross-site' } })).status, 403);
    for (const path of ['/api/bookmarks', '/api/contact', '/api/admin/monitor']) assert.equal((await send(path)).status, 401);
    assert.equal((await post('/api/admin/verify', {})).status, 401);
    assert.equal((await send('/api/admin/monitor', {headers:{Authorization:`Bearer ${token}`}})).status,200);
    assert.equal((await post('/api/crawl', { duration: 30 })).status, 401);
    assert.equal((await post('/api/contact/read', { id: 1 })).status, 401);
    assert.equal((await send('/api/img?url=https%3A%2F%2Funindexed.example%2Fpic.png')).status, 403);
    assert.equal((await send('/api/config')).status, 200);
    assert.equal((await (await send('/api/config')).json()).public, true);
    assert.equal((await (await send('/api/stats')).json()).messages, undefined);
    assert.equal((await send('/api/contact', { headers: { Authorization: 'Bearer wrong' } })).status, 401);
    assert.equal((await send('/api/contact', { headers: { Authorization: `Bearer ${token}` } })).status, 200);
    const capacityDb = new DatabaseSync(join(dir, 'vertex.db'));
    capacityDb.exec("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO contact_messages(message) SELECT 'capacity fixture' FROM n");
    assert.equal((await post('/api/contact', { message: 'Mailbox full' })).status, 503);
    capacityDb.exec("DELETE FROM contact_messages WHERE message='capacity fixture'");
    capacityDb.close();
    for (let i = 0; i < 2; i++) assert.equal((await post('/api/contact', { message: 'Public message' })).status, 200);
    assert.equal((await post('/api/contact', { message: 'Too many' })).status, 429);
    assert.equal((await send('/health', { headers: { Origin: 'https://attacker.example' } })).status, 403);
    assert.equal((await send('/health', { headers: { Host: 'attacker.example' } })).status, 403);
    assert.equal((await send('/health')).headers.get('strict-transport-security'), 'max-age=31536000');
    assert.equal((await post('/api/contact', { message: 'a'.repeat(20000) })).status, 429);
    console.log('Public smoke passed: admin isolation, media allowlist, privacy, limits, HTTPS headers.');
  } else {

  assert.equal((await post('/api/bookmarks', { url: 'https://example.com/guide', title: 'Rust' })).status, 200);
  assert.equal((await (await send('/api/bookmarks')).json()).bookmarks.length, 1);
  assert.equal((await post('/api/bookmarks', { url: 'javascript:alert(1)' })).status, 400);
  assert.equal((await send('/api/bookmarks?url=https%3A%2F%2Fexample.com%2Fguide', { method: 'DELETE' })).status, 200);
  assert.equal((await (await send('/api/bookmarks')).json()).bookmarks.length, 0);

  const message = await (await post('/api/contact', { message: 'Smoke test', subject: 'Test' })).json();
  assert.equal((await post('/api/contact/read', { id: message.id })).status, 200);
  assert.equal((await (await send('/api/contact')).json()).messages[0].is_read, true);
  assert.equal((await send('/health', { headers: { Origin: 'https://attacker.example' } })).status, 403);
  // fetch may rewrite Host; use the HTTP API to preserve the hostile value.
  const hostileHostStatus = await new Promise((resolve, reject) => {
    const req = request(`${base}/health`, {
      headers: { Host: 'attacker.example' }, timeout: 5000,
    }, response => {
      response.resume();
      resolve(response.statusCode);
    });
    req.on('error', reject);
    req.on('timeout', () => req.destroy(new Error('Host test timed out')));
    req.end();
  });
  assert.equal(hostileHostStatus, 403);
  assert.equal((await send('/api/img?url=http%3A%2F%2F127.0.0.1%2Fsecret')).status, 400);
  assert.equal((await (await send('/api/crawl/status')).json()).running, false);
  const monitor=await (await send('/api/admin/monitor')).json();
  assert.equal(monitor.running,false);
  assert.equal(monitor.runtime,null);
  assert.equal(monitor.stats.pages,1);
  assert.equal(monitor.stats.pending,1);
  assert.equal(monitor.database.schema,'6');
  assert.equal((await send('/panel')).status,200);
  assert.equal((await post('/api/admin/verify',{})).status,202);
  let verification;
  for(let i=0;i<100;i++) {
    verification=(await (await send('/api/admin/monitor')).json()).verification;
    if(verification.status!=='running')break;
    await delay(20);
  }
  assert.equal(verification.status,'ok',JSON.stringify(verification));
  assert.equal(verification.counter_mismatches,0);
  console.log('API smoke passed: search, sanitized reader, bookmarks, contact, origin/host checks, local proxy blocking.');
  }
} finally {
  if (child.exitCode === null && !launchError) {
    const stopped = once(child, 'exit');
    child.kill();
    await stopped;
  }
  await rm(dir, { recursive: true, force: true });
}
