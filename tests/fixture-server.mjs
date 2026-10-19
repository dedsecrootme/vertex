import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm, readFile } from 'node:fs/promises';
import { createServer, request } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { setTimeout as delay } from 'node:timers/promises';

const dir = await mkdtemp(join(tmpdir(), 'vertex-browser-'));
const reservation = createServer();
reservation.listen(0, '127.0.0.1');
await once(reservation, 'listening');
const port = reservation.address().port;
await new Promise(done => reservation.close(done));
const child = spawn(resolve('target/debug', process.platform === 'win32' ? 'api-rs.exe' : 'api-rs'), [], {
  env: { ...process.env, VERTEX_DB_DIR: dir, VERTEX_PORT: String(port),
    VERTEX_PUBLIC_ORIGIN: 'https://vertex.example', VERTEX_ALLOW_DISCOVERY: '1', VERTEX_ADMIN_TOKEN: 'browser-fixture-token-not-for-production-123456' },
  windowsHide: true, stdio: ['ignore', 'inherit', 'inherit'],
});
let launchError;
child.on('error', error => { launchError = error; });
const upstream = `http://127.0.0.1:${port}`;
const proxyPort = Number(process.env.VERTEX_FIXTURE_PORT || 4319);

// Test-only stand-in for HTTPS termination: production uses Caddy and a real
// certificate. The backend still executes its real public access checks.
const proxy = createServer((incoming, outgoing) => {
  const staticFiles = { '/presentation': ['apps/landing/public/index.html', 'text/html; charset=utf-8'],
    '/THIRD-PARTY-NOTICES.txt': ['apps/landing/public/THIRD-PARTY-NOTICES.txt', 'text/plain; charset=utf-8'],
    '/site.css': ['apps/landing/public/site.css', 'text/css; charset=utf-8'],
    '/premium.css': ['apps/landing/public/premium.css', 'text/css; charset=utf-8'],
    '/icons.svg': ['apps/landing/public/icons.svg', 'image/svg+xml'],
    '/vendor/motion-mini.js': ['apps/landing/public/vendor/motion-mini.js', 'text/javascript; charset=utf-8'],
    '/site.js': ['apps/landing/public/site.js', 'text/javascript; charset=utf-8'],
    '/preview.png': ['apps/landing/public/preview.png', 'image/png'],
    '/vertex-chrome.webp': ['apps/landing/public/vertex-chrome.webp', 'image/webp'],
    '/search-view.png': ['apps/landing/public/search-view.png', 'image/png'],
    '/reader-view.png': ['apps/landing/public/reader-view.png', 'image/png'],
    '/crawler-view.png': ['apps/landing/public/crawler-view.png', 'image/png'] };
  if (incoming.url === '/api/release') { outgoing.writeHead(200, {'Content-Type': 'application/json'}); outgoing.end('{"available":false}'); return; }
  if (incoming.url === '/search') { outgoing.writeHead(302, {Location: '/'}); outgoing.end(); return; }
  if (staticFiles[incoming.url]) {
    const [path, contentType] = staticFiles[incoming.url];
    readFile(path).then(data => { outgoing.writeHead(200, { 'Content-Type': contentType }); outgoing.end(data); })
      .catch(() => { outgoing.writeHead(404); outgoing.end(); });
    return;
  }
  const headers = { ...incoming.headers, host: 'vertex.example' };
  if (headers.origin === `http://127.0.0.1:${proxyPort}`) headers.origin = 'https://vertex.example';
  const req = request(upstream + incoming.url, { method: incoming.method, headers }, response => {
    outgoing.writeHead(response.statusCode, response.headers);
    response.pipe(outgoing);
  });
  req.on('error', () => { outgoing.writeHead(502); outgoing.end(); });
  incoming.pipe(req);
});
let cleaning;
function cleanup() {
  return cleaning ||= (async () => {
    proxy.closeAllConnections();
    if (proxy.listening) await new Promise(done => proxy.close(done));
    if (child.exitCode === null && child.signalCode === null && !launchError) {
      const stopped = once(child, 'exit'); child.kill(); await stopped;
    }
    await rm(dir, { recursive: true, force: true });
  })();
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => cleanup().then(() => process.exit(0)));

try {
  for (let i = 0; i < 200; i++) {
    if (launchError) throw launchError;
    if (child.exitCode !== null) throw new Error('Fixture API exited');
    try { await fetch(upstream + '/health', { signal: AbortSignal.timeout(500) }); break; }
    catch { if (i === 199) throw new Error('Fixture startup timed out'); await delay(50); }
  }
  const db = new DatabaseSync(join(dir, 'vertex.db'));
  const insert = db.prepare('INSERT INTO pages(url,domain,title,content,content_html) VALUES (?,?,?,?,?)');
  insert.run('https://example.com/guide', 'example.com', 'Guide Rust', 'Rust guide avec contenu de test',
    '<script>window.vertexXss=true</script><p>Document Rust de test</p><img src="http://127.0.0.1/secret" onerror="window.vertexXss=true"><a href="javascript:alert(1)">Lien interdit</a>');
  insert.run('https://example.com/long', 'example.com', 'W'.repeat(200), 'longword', '<p>' + 'W'.repeat(1000) + '</p>');
  db.close();
  proxy.listen(proxyPort, '127.0.0.1');
  await once(proxy, 'listening');
  console.log('Isolated public browser fixture ready');
} catch (error) { await cleanup(); throw error; }
