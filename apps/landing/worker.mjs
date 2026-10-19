const headers = {
  'X-Content-Type-Options': 'nosniff',
  'Referrer-Policy': 'no-referrer',
  'Permissions-Policy': 'camera=(), microphone=(), geolocation=()',
  'Content-Security-Policy': "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
  'Strict-Transport-Security': 'max-age=31536000',
};

export function validRelease(value) {
  const w = value?.windows;
  return /^\d+\.\d+\.\d+$/.test(value?.version || '') && w &&
    w.key === `releases/${value.version}/${w.sha256}/Vertex-${value.version}-x64-setup.exe` &&
    w.filename === `Vertex-${value.version}-x64-setup.exe` &&
    /^[a-f0-9]{64}$/.test(w.sha256 || '') &&
    Number.isSafeInteger(w.size) && w.size > 0 && w.size <= 500_000_000;
}

export function parseRange(value, size) {
  if (!value) return null;
  const match = /^bytes=(\d*)-(\d*)$/.exec(value);
  if (!match || (!match[1] && !match[2])) throw new Error('range');
  const start = match[1] ? Number(match[1]) : Math.max(0, size - Number(match[2]));
  const end = match[1] ? (match[2] ? Math.min(Number(match[2]), size - 1) : size - 1) : size - 1;
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || start > end || start >= size) throw new Error('range');
  return { offset: start, length: end - start + 1 };
}

async function release(env) {
  if (!env.DOWNLOADS) return null;
  const object = await env.DOWNLOADS.get('releases/stable.json');
  if (!object || object.size > 8192) return null;
  let value;
  try { value = await object.json(); } catch { return null; }
  if (!validRelease(value)) return null;
  const file = await env.DOWNLOADS.head(value.windows.key);
  if (!file || file.size !== value.windows.size) return null;
  return { value, file };
}

function json(value, status = 200) {
  return Response.json(value, { status, headers: { 'Cache-Control': 'no-store' } });
}

async function route(request, env) {
  const url = new URL(request.url);
  if (!['GET', 'HEAD'].includes(request.method)) return new Response('Methode refusee', { status: 405, headers: { Allow: 'GET, HEAD' } });
  if (url.pathname === '/search') {
    let target;
    try { target = new URL(env.VERTEX_APP_URL); } catch { /* Not yet provisioned. */ }
    if (!target || target.protocol !== 'https:' || target.username || target.password || target.origin === url.origin) {
      return new Response('Index partage en cours de mise en service.', { status: 503, headers: { 'Retry-After': '3600', 'Cache-Control': 'no-store' } });
    }
    return Response.redirect(target.origin, 302);
  }
  if (url.pathname === '/api/release' || url.pathname === '/downloads/windows') {
    const available = await release(env);
    if (!available) return json({ available: false }, url.pathname === '/api/release' ? 200 : 503);
    const { value, file } = available;
    if (url.pathname === '/api/release') return json({ available: true, version: value.version,
      windows: { filename: value.windows.filename, size: file.size, sha256: value.windows.sha256, url: '/downloads/windows' } });
    const h = new Headers({ 'Content-Type': 'application/octet-stream', 'Content-Disposition': `attachment; filename="${value.windows.filename}"`,
      'Accept-Ranges': 'bytes', ETag: file.httpEtag, 'Cache-Control': 'private, no-cache', 'X-Checksum-SHA256': value.windows.sha256 });
    if (request.headers.get('If-None-Match')?.split(',').map(v => v.trim()).includes(file.httpEtag)) return new Response(null, { status: 304, headers: h });
    let range;
    try {
      range = request.method === 'HEAD' || (request.headers.has('If-Range') && request.headers.get('If-Range') !== file.httpEtag)
        ? null : parseRange(request.headers.get('Range'), file.size);
    } catch {
      h.set('Content-Range', `bytes */${file.size}`);
      return new Response(null, { status: 416, headers: h });
    }
    h.set('Content-Length', String(range?.length ?? file.size));
    if (range) h.set('Content-Range', `bytes ${range.offset}-${range.offset + range.length - 1}/${file.size}`);
    if (request.method === 'HEAD') return new Response(null, { headers: h });
    // Pin the object version used for metadata; never stream a concurrently replaced file.
    const object = await env.DOWNLOADS.get(value.windows.key, { onlyIf: { etagMatches: file.etag }, ...(range ? { range } : {}) });
    if (!object || !object.body) return json({ available: false }, 503);
    return new Response(object.body, { status: range ? 206 : 200, headers: h });
  }
  if (url.pathname.startsWith('/downloads/') || url.pathname.startsWith('/api/')) return new Response('Introuvable', { status: 404 });
  return env.ASSETS.fetch(request);
}

export default {
  async fetch(request, env) {
    let response;
    try { response = await route(request, env); }
    catch { response = json({ error: 'Service temporairement indisponible' }, 503); }
    const result = new Response(request.method === 'HEAD' ? null : response.body, response);
    for (const [key, value] of Object.entries(headers)) result.headers.set(key, value);
    return result;
  },
};
