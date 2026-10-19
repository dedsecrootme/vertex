import { test } from 'node:test';
import assert from 'node:assert/strict';
import worker, { validRelease, parseRange } from '../apps/landing/worker.mjs';

const manifest = { version: '0.2.0', windows: { key: `releases/0.2.0/${'a'.repeat(64)}/Vertex-0.2.0-x64-setup.exe`, filename: 'Vertex-0.2.0-x64-setup.exe', size: 10, sha256: 'a'.repeat(64) } };
function environment(value = manifest) {
  const calls = [];
  return { calls, ASSETS: { fetch: async () => new Response('Vertex') }, DOWNLOADS: {
    head: async () => ({ size: 10, etag: 'abc', httpEtag: '"abc"' }),
    get: async (key, options) => {
      calls.push({ key, options });
      if (key === 'releases/stable.json') return { size: 200, json: async () => value };
      const r = options?.range;
      return { body: new Blob([r ? '0123456789'.slice(r.offset, r.offset + r.length) : '0123456789']).stream() };
    },
  } };
}
const fetchWorker = (path, env, options) => worker.fetch(new Request('https://vertex.example' + path, options), env);

test('release manifest never accepts paths, sources or malformed checksums', () => {
  assert.ok(validRelease(manifest));
  for (const key of ['../secret', 'source.zip', 'releases/stable.json']) assert.ok(!validRelease({ ...manifest, windows: { ...manifest.windows, key } }));
  assert.ok(!validRelease({ ...manifest, windows: { ...manifest.windows, sha256: 'bad' } }));
});
test('ranges cover suffix, resume, boundaries and malformed input', () => {
  assert.deepEqual(parseRange('bytes=3-', 10), { offset: 3, length: 7 });
  assert.deepEqual(parseRange('bytes=-3', 10), { offset: 7, length: 3 });
  assert.deepEqual(parseRange('bytes=0-99', 10), { offset: 0, length: 10 });
  for (const value of ['bytes=20-', 'bytes=4-2', 'bytes=-0', 'bytes=0-1,3-4', 'bytes=-', 'garbage']) assert.throws(() => parseRange(value, 10));
});
test('unprovisioned R2 never exposes an old or fake download', async () => {
  assert.deepEqual(await (await fetchWorker('/api/release', environment(null))).json(), { available: false });
  assert.equal((await fetchWorker('/downloads/windows', environment(null))).status, 503);
});
test('private R2 streaming supports resume, HEAD, etag and safe metadata', async () => {
  const env = environment();
  const response = await fetchWorker('/downloads/windows', env, { headers: { Range: 'bytes=2-4' } });
  assert.equal(response.status, 206);
  assert.equal(await response.text(), '234');
  assert.equal(response.headers.get('Content-Range'), 'bytes 2-4/10');
  assert.equal(response.headers.get('Content-Length'), '3');
  assert.equal(response.headers.get('X-Content-Type-Options'), 'nosniff');
  assert.deepEqual(env.calls[1].options.onlyIf, { etagMatches: 'abc' });
  assert.equal((await fetchWorker('/downloads/windows', env, { headers: { Range: 'bytes=99-' } })).status, 416);
  assert.equal((await fetchWorker('/downloads/windows', env, { headers: { 'If-None-Match': '"abc"' } })).status, 304);
  assert.equal((await fetchWorker('/downloads/windows', env, { method: 'HEAD' })).headers.get('Content-Length'), '10');
});
test('no writes, open proxy, key listing or source route is available', async () => {
  const env = environment();
  assert.equal((await fetchWorker('/downloads/secret', env)).status, 404);
  assert.equal((await fetchWorker('/api/secrets', env)).status, 404);
  assert.equal((await fetchWorker('/api/release', env, { method: 'POST', body: '{}' })).status, 405);
  assert.equal((await fetchWorker('/search', env)).status, 503);
  assert.equal((await fetchWorker('/search', { ...env, VERTEX_APP_URL: 'https://index.example' })).headers.get('Location'), 'https://index.example/');
  assert.equal((await fetchWorker('/search', { ...env, VERTEX_APP_URL: 'https://vertex.example' })).status, 503);
});

test('security headers cover assets, redirects, errors and method rejection', async () => {
  for(const [path,env,options] of [
    ['/',environment()],['/api/unknown',environment()],
    ['/search',{...environment(),VERTEX_APP_URL:'https://index.example'}],
    ['/api/release',environment(),{method:'POST',body:'{}'}],
    ['/',{ASSETS:{fetch:async()=>{throw new Error('private-stack-secret');}}}],
  ]){
    const response=await fetchWorker(path,env,options);
    assert.equal(response.headers.get('X-Content-Type-Options'),'nosniff');
    assert.match(response.headers.get('Content-Security-Policy'),/script-src 'self'/);
    assert.match(response.headers.get('Content-Security-Policy'),/frame-ancestors 'none'/);
    assert.match(response.headers.get('Permissions-Policy'),/camera=\(\)/);
    assert.equal(response.headers.get('Referrer-Policy'),'no-referrer');
    assert.ok(!(await response.text()).includes('private-stack-secret'));
  }
});

test('search redirects cannot target HTTP, credentials or a user-supplied destination',async()=>{
  for(const target of ['http://index.example','https://user:secret@index.example','javascript:alert(1)','//evil.example']){
    assert.equal((await fetchWorker('/search',{...environment(),VERTEX_APP_URL:target})).status,503);
  }
  const response=await fetchWorker('/search?url=https://evil.example',{...environment(),VERTEX_APP_URL:'https://index.example'});
  assert.equal(response.headers.get('Location'),'https://index.example/');
});

test('oversized manifests and replaced objects never produce an installer stream',async()=>{
  const oversized=environment();oversized.DOWNLOADS.get=async()=>({size:8193,json:async()=>{throw new Error('must not parse');}});
  assert.equal((await fetchWorker('/downloads/windows',oversized)).status,503);
  const replaced=environment();
  const original=replaced.DOWNLOADS.get;
  replaced.DOWNLOADS.get=async(key,options)=>key==='releases/stable.json'?original(key,options):{etag:'changed'};
  assert.equal((await fetchWorker('/downloads/windows',replaced)).status,503);
});
