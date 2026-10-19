"use strict";
const $ = s => document.querySelector(s);
const $$ = s => Array.from(document.querySelectorAll(s));
const inTauri = typeof window !== 'undefined' && !!window.__TAURI__ && !!window.__TAURI__.core && !!window.__TAURI__.core.invoke;
const esc = s => (s || '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const domain = u => { try { return new URL(u).hostname; } catch (e) { return (u || '').replace(/^https?:\/\//,'').split('/')[0]; } };
let publicMode = false;
let clientConfig = {};
const configuration = (inTauri ? window.__TAURI__.core.invoke('client_config') : fetch('/api/config').then(async response => {
  if (!response.ok) throw new Error('Configuration indisponible');
  return response.json();
})).then(config => {
  clientConfig = config;
  publicMode = !!config.public;
});
const icon = name => '<svg class="icon" aria-hidden="true"><use href="/icons.svg#' + name + '"></use></svg>';
function showError(error) {
  $('#meta').textContent = error.message || String(error);
  $('#errorBanner').textContent = error.message || String(error);
  $('#errorBanner').hidden = false;
}
window.addEventListener('unhandledrejection', event => { event.preventDefault(); showError(event.reason || 'Action impossible'); });
function safeUrl(value) {
  try {
    const url = new URL(value);
    return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password ? url.href : '';
  } catch (_) { return ''; }
}
function browserBookmarks() {
  let rows;
  try { rows = JSON.parse(localStorage.getItem('vertex.bookmarks') || '[]'); }
  catch (_) { return []; }
  return Array.isArray(rows) ? rows.filter(row => row && safeUrl(row.url)).slice(0, 500) : [];
}
function releaseMedia(root) {
  root.querySelectorAll('img[src^="blob:"]').forEach(img => URL.revokeObjectURL(img.src));
}

async function checkedFetch(url, options) {
  const response = await fetch(url, options);
  if (!response.ok) {
    const payload = await response.json().catch(() => ({}));
    throw new Error(payload.error || 'Erreur HTTP ' + response.status);
  }
  return response;
}

/// Appel unifié : commande Tauri en desktop, endpoint REST en mode web.
async function api(method, params) {
  await configuration;
  params = params || {};
  if (inTauri) return await window.__TAURI__.core.invoke(method, params);
  if (publicMode && ['bookmarks_list', 'bookmark_add', 'bookmark_remove'].includes(method)) {
    let rows = browserBookmarks();
    if (method === 'bookmarks_list') return rows;
    if (!safeUrl(params.url)) throw new Error('Lien non autorisé');
    rows = rows.filter(row => row.url !== params.url);
    if (method === 'bookmark_add') {
      if (rows.length >= 500) throw new Error('Limite de 500 signets atteinte');
      rows.unshift({url: params.url, title: params.title || params.url});
    }
    localStorage.setItem('vertex.bookmarks', JSON.stringify(rows));
    return true;
  }
  switch (method) {
    case 'search': {
      const r = await checkedFetch('/api/search?' + new URLSearchParams({ q: params.query || '', offset: String(params.offset || 0) }));
      return await r.json();
    }
    case 'page': {
      const r = await fetch('/api/page?' + new URLSearchParams({ url: params.url || '' }));
      if (r.status === 404) return null;
      if (!r.ok) throw new Error('Erreur HTTP ' + r.status);
      return await r.json();
    }
    case 'stats': return await (await checkedFetch('/api/stats')).json();
    case 'discover': return (await (await checkedFetch('/api/discover', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({url: params.url}) })).json()).status;
    case 'bookmarks_list': return ((await (await checkedFetch('/api/bookmarks')).json()).bookmarks) || [];
    case 'bookmark_add': {
      await checkedFetch('/api/bookmarks', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(params) });
      return true;
    }
    case 'bookmark_remove': {
      await checkedFetch('/api/bookmarks?' + new URLSearchParams({ url: params.url }), { method: 'DELETE' });
      return true;
    }
    case 'contact_send': {
      const r = await checkedFetch('/api/contact', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(params) });
      return (await r.json()).id;
    }
    case 'contact_list': return ((await (await checkedFetch('/api/contact')).json()).messages) || [];
    case 'contact_mark_read': {
      await checkedFetch('/api/contact/read', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(params) });
      return true;
    }
    case 'crawl': {
      const response = await fetch('/api/crawl', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ duration: params.duration }) });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.error || 'Crawl indisponible');
      return payload;
    }
    case 'crawl_status': return (await (await checkedFetch('/api/crawl/status')).json()).running;
    case 'check_link': return await (await checkedFetch('/api/safety?' + new URLSearchParams({ url: params.url }))).json();
    default: return null;
  }
}

/// Renvoie une URL de média *sans jamais* contacter le tiers depuis le navigateur.
async function mediaSrc(url, kind) {
  if (!url) return '';
  kind = kind || 'img';
  try {
    if (inTauri) {
      const r = await window.__TAURI__.core.invoke('fetch_media', { url });
      const blob = new Blob([new Uint8Array(r.data)], { type: r.content_type || 'application/octet-stream' });
      return URL.createObjectURL(blob);
    }
    return '/' + (kind === 'favicon' ? 'api/favicon?' : 'api/img?') + new URLSearchParams({ url });
  } catch (e) {
    return '';
  }
}

async function setFavicon(el, pageUrl) {
  if (!el) return;
  const d = domain(pageUrl);
  if (!d) return;
  const src = await mediaSrc('https://' + d + '/favicon.ico', 'favicon');
  if (src && el.isConnected) {
    releaseMedia(el);
    const img = new Image();
    img.alt = '';
    img.addEventListener('error', () => { img.hidden = true; });
    img.src = src;
    el.replaceChildren(img);
  } else if (src.startsWith('blob:')) URL.revokeObjectURL(src);
}

function safetyBadge(verdict) {
  const v = verdict || 'unknown';
  const label = { safe: '✔ sûr', suspicious: '⚠ suspect', malicious: '⛔ malveillant', unknown: '· inconnu' }[v] || v;
  return '<span class="badge ' + esc(v) + '">' + esc(label) + '</span>';
}

function highlight(text, terms) {
  const words = (terms || []).filter(Boolean).slice(0, 32);
  if (!words.length) return esc(text || '');
  const expression = new RegExp('(' + words.map(word => word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|') + ')', 'gi');
  return String(text || '').split(expression).map((part, index) => index % 2 ? '<mark>' + esc(part) + '</mark>' : esc(part)).join('');
}

let currentQuery = '';
let loaded = 0;
let searchVersion = 0;
let searchBusy = false;

async function doSearch(q, append) {
  const version = append ? searchVersion : ++searchVersion;
  const list = $('#results');
  if (append && searchBusy) return;
  searchBusy = true;
  $('#more').disabled = true;
  if (!append) { loaded = 0; releaseMedia(list); list.innerHTML = '<p class="empty">Recherche…</p>'; }
  let data;
  try { data = await api('search', { query: q, offset: loaded }); }
  catch (error) {
    if (version === searchVersion) { if (!append) list.innerHTML = ''; showError(error); }
    return;
  } finally {
    if (version === searchVersion) { searchBusy = false; $('#more').disabled = false; }
  }
  if (version !== searchVersion) return;
  const results = Array.isArray(data) ? data : (data && data.results) || [];
  const terms = q.split(/\s+/).filter(Boolean);
  if (!append) list.innerHTML = '';
  if (!append && results.length === 0) {
    list.innerHTML = '<p class="empty">Aucun résultat pour « ' + esc(q) + ' ».</p>';
    $('#meta').textContent = ''; $('#more').style.display = 'none';
    if (clientConfig.discovery && safeUrl(q)) {
      try {
        const status = await api('discover', {url: q});
        if (version !== searchVersion) return;
        const labels = {queued: 'URL ajoutée à la file partagée.', processing: 'Indexation en cours.', ready: 'Page déjà présente dans l’index.', deferred: 'Nouvelle tentative différée.', blocked: 'Exploration non autorisée pour cette URL.', full: 'File d’indexation momentanément pleine.'};
        $('#meta').textContent = labels[status] || 'Demande reçue.';
      } catch (error) { if (version === searchVersion) $('#meta').textContent = error.message || String(error); }
    }
    return;
  }
  for (const r of results) {
    r.url = safeUrl(r.url);
    if (!r.url) continue;
    if (publicMode) r.bookmarked = browserBookmarks().some(row => row.url === r.url);
    const d = r.domain || domain(r.url);
    const card = document.createElement('article');
    card.className = 'card';
    card.innerHTML =
      '<div class="row"><div class="fav"></div><span class="badge">' + esc(d) + '</span>' +
      safetyBadge(r.safety) + '</div>' +
      '<h2><a href="#" class="openPage">' + highlight(r.title, terms) + '</a></h2>' +
      '<p class="snippet">' + highlight(r.snippet, terms) + '</p>' +
      '<div class="actions">' +
        '<button class="btn primary act-read">' + icon('book-open') + 'Lire</button>' +
        '<a class="btn ghost" href="' + esc(r.url) + '" target="_blank" rel="noopener noreferrer">' + icon('external-link') + 'Ouvrir le site</a>' +
        '<button class="btn ghost act-bm">' + icon('bookmark') + (r.bookmarked ? 'Retirer' : 'Signet') + '</button>' +
        '<button class="btn ghost act-safety" aria-label="Vérifier la sûreté" title="Vérifier la sûreté">' + icon('shield-check') + '</button>' +
      '</div>' +
      '<div class="safety-detail meta"></div>';
    setFavicon(card.querySelector('.fav'), r.url);
    card.querySelector('.act-bm').dataset.on = r.bookmarked ? '1' : '0';
    card.querySelector('.act-read').onclick = () => openPage(r.url);
    card.querySelector('.openPage').onclick = e => { e.preventDefault(); openPage(r.url); };
    card.querySelector('.act-bm').onclick = async ev => {
      const b = ev.currentTarget;
      b.disabled = true;
      try {
        if (b.dataset.on === '1') { await api('bookmark_remove', { url: r.url }); b.dataset.on = '0'; b.textContent = 'Signet'; }
        else { await api('bookmark_add', { url: r.url, title: r.title }); b.dataset.on = '1'; b.textContent = 'Retirer'; }
        await refreshStats();
      } catch (error) { showError(error); }
      finally { b.disabled = false; }
    };
    card.querySelector('.act-safety').onclick = async () => {
      const rep = await api('check_link', { url: r.url });
      const reasons = (rep && rep.reasons && rep.reasons.length) ? rep.reasons.join(' ; ') : 'Aucun signalement.';
      card.querySelector('.safety-detail').textContent = '🛡️ ' + (rep ? rep.verdict : '?') + ' (score ' + (rep ? rep.score : '?') + ') — ' + reasons;
    };
    list.appendChild(card);
  }
  loaded += results.length;
  $('#meta').textContent = loaded + ' résultat(s) pour « ' + q + ' »';
  $('#more').style.display = results.length >= 30 && loaded < 30000 ? '' : 'none';
}

function run(q) {
  $('#errorBanner').hidden = true;
  showTab('results');
  if (!q) { $('#meta').textContent = 'Saisissez un terme.'; $('#results').innerHTML = ''; return; }
  currentQuery = q;
  doSearch(q, false);
}

let readerVersion = 0;
async function openPage(url) {
  const version = ++readerVersion;
  url = safeUrl(url);
  if (!url) { showError(new Error('Lien non autorisé')); return; }
  if (!$('#view').classList.contains('open')) {
    previousFocus = document.activeElement;
    previousOverflow = document.body.style.overflow;
    previousTitle = document.title;
    history.pushState({ ...history.state, vertexReader: true }, '', location.href);
  }
  $('#view').classList.add('open');
  document.body.style.overflow = 'hidden';
  $('main').inert = true;
  $('.hero').inert = true;
  $('footer').inert = true;
  $('#view').scrollTop = 0;
  $('#vTitle').textContent = 'Chargement…';
  $('#vUrl').textContent = url; $('#vUrl').href = url;
  $('#vMeta').textContent = ''; $('#vReading').textContent = '';
  $('#vSafety').textContent = '';
  $('#vNotice').replaceChildren(); $('#vLinks').replaceChildren(); $('#vCounts').textContent = '0';
  releaseMedia($('#vBody')); releaseMedia($('#vFav'));
  $('#vBody').textContent = 'Chargement de la page…';
  $('#vBody').setAttribute('aria-busy', 'true');
  $('#readingProgress').value = 0;
  $('#closeView').focus();
  try {
  const pg = await api('page', { url });
  if (version !== readerVersion) return;
  if (!pg) throw new Error('Page introuvable dans l’index.');
  $('#vTitle').textContent = pg.title || '(sans titre)';
  document.title = (pg.title || 'Lecture') + ' | Vertex';
  $('#vReading').textContent = Math.max(1, Math.ceil((pg.content || $('#vBody').textContent).trim().split(/\s+/).length / 200)) + ' min de lecture';
  $('#vUrl').textContent = pg.url; $('#vUrl').href = safeUrl(pg.url) || '#';
  $('#vSafety').outerHTML = safetyBadge(pg.safety).replace('class="badge', 'id="vSafety" class="badge');
  $('#vMeta').textContent = (pg.status_code ? '· HTTP ' + pg.status_code : '') + (pg.language ? ' · ' + pg.language : '') + (pg.crawled_at ? ' · ' + pg.crawled_at : '');
  setFavicon($('#vFav'), pg.url);

  $('#vNotice').innerHTML = '';
  if (pg.safety === 'suspicious' || pg.safety === 'malicious') {
    $('#vNotice').innerHTML = '<div class="danger-note">⚠️ Ce lien est signalé <b>' + esc(pg.safety) + '</b>. Contenu affiché en lecture seule, sans script.</div>';
  }

  releaseMedia($('#vBody'));
  let html = pg.content_html || '';
  if (html) {
    $('#vBody').innerHTML = html;
  } else {
    $('#vBody').replaceChildren(...String(pg.content || '(aucun contenu enregistré)').split(/\r?\n\s*\r?\n/).map(text => {
      const paragraph = document.createElement('p');
      paragraph.textContent = text;
      paragraph.style.whiteSpace = 'pre-line';
      return paragraph;
    }));
  }

  $$('#vBody img[data-src]').forEach(async (img, index) => {
    if (index >= 24) { img.remove(); return; }
    const src = await mediaSrc(img.getAttribute('data-src'), 'img');
    if (!img.isConnected) { if (src.startsWith('blob:')) URL.revokeObjectURL(src); return; }
    if (src) img.src = src; else img.remove();
  });

  const imgs = pg.images || [], links = pg.links || [];
  $('#vCounts').textContent = imgs.length + ' images · ' + links.length + ' liens';
  $('#vLinks').innerHTML = links.filter(l => safeUrl(l.url)).slice(0, 200).map(l =>
    safetyBadge(l.safety) + ' <a href="' + esc(l.url) + '" target="_blank" rel="noopener noreferrer">' + esc(l.url) + '</a>'
  ).join('<br>') || 'Aucun lien documenté.';

  } catch (error) {
    if (version !== readerVersion) return;
    $('#vTitle').textContent = 'Page indisponible';
    $('#vBody').textContent = error.message || 'La page n’a pas pu être chargée.';
  } finally {
    if (version === readerVersion) $('#vBody').setAttribute('aria-busy', 'false');
  }
}

let previousFocus;
let previousOverflow = '', previousTitle = '', readerSize = 18;
function closeView(fromHistory = false) {
  if (!$('#view').classList.contains('open')) return;
  readerVersion++;
  $('#view').classList.remove('open');
  document.body.style.overflow = previousOverflow;
  document.title = previousTitle;
  $('main').inert = false;
  $('.hero').inert = false;
  $('footer').inert = false;
  releaseMedia($('#vBody'));
  releaseMedia($('#vFav'));
  $('#vBody').replaceChildren();
  if (previousFocus && previousFocus.isConnected) previousFocus.focus();
  if (!fromHistory && history.state?.vertexReader) history.back();
}
$('#closeView').addEventListener('click', () => closeView());
addEventListener('popstate', () => { if (!history.state?.vertexReader) closeView(true); });
function resizeReader(change) {
  readerSize = Math.min(24, Math.max(16, readerSize + change));
  $('#vBody').style.setProperty('--reader-size', readerSize + 'px');
  $('#readerSmaller').disabled = readerSize === 16;
  $('#readerLarger').disabled = readerSize === 24;
}
$('#readerSmaller').addEventListener('click', () => resizeReader(-1));
$('#readerLarger').addEventListener('click', () => resizeReader(1));
$('#view').addEventListener('scroll', () => {
  const view = $('#view'), available = view.scrollHeight - view.clientHeight;
  $('#readingProgress').value = available > 0 ? 100 * view.scrollTop / available : 0;
}, { passive: true });

document.addEventListener('click', event => {
  if (!inTauri) return;
  const link = event.target.closest && event.target.closest('a[target="_blank"][href]');
  if (!link) return;
  event.preventDefault();
  window.__TAURI__.core.invoke('open', { url: link.href }).catch(error => {
    $('#meta').textContent = String(error);
  });
});

function showTab(name) {
  if (publicMode && name === 'inbox') return;
  $$('.tab').forEach(t => t.classList.toggle('active', t.dataset.tab === name));
  $$('.tab').forEach(t => t.setAttribute('aria-selected', String(t.dataset.tab === name)));
  $$('.tab').forEach(t => { t.tabIndex = t.dataset.tab === name ? 0 : -1; });
  ['results', 'bookmarks', 'contact', 'inbox'].forEach(p => {
    $('#panel-' + p).style.display = (p === name) ? '' : 'none';
  });
  if (name === 'bookmarks') loadBookmarks();
  if (name === 'inbox') loadInbox();
}

async function loadBookmarks() {
  const list = await api('bookmarks_list', {});
  const el = $('#bookmarkList');
  releaseMedia(el);
  if (!list || list.length === 0) { el.innerHTML = '<p class="empty">Aucun signet pour l\'instant.</p>'; return; }
  el.innerHTML = '';
  for (const b of list) {
    const card = document.createElement('div');
    card.className = 'card';
    card.innerHTML =
      '<div class="row"><div class="fav"></div><span class="badge">' + esc(domain(b.url)) + '</span></div>' +
      '<h2><a href="#" class="bmOpen">' + esc(b.title || b.url) + '</a></h2>' +
      '<p class="meta">' + esc(b.url) + '</p>' +
      '<div class="actions">' +
        '<button class="btn primary bmRead">👁️ Lire</button>' +
        '<a class="btn ghost" href="' + esc(b.url) + '" target="_blank" rel="noopener noreferrer">Ouvrir</a>' +
        '<button class="btn danger bmDel">🗑️ Retirer</button></div>';
    setFavicon(card.querySelector('.fav'), b.url);
    card.querySelector('.bmOpen').onclick = e => { e.preventDefault(); openPage(b.url); };
    card.querySelector('.bmRead').onclick = () => openPage(b.url);
    card.querySelector('.bmDel').onclick = async () => { await api('bookmark_remove', { url: b.url }); loadBookmarks(); refreshStats(); };
    el.appendChild(card);
  }
}

async function loadInbox() {
  const list = await api('contact_list', {});
  const el = $('#inboxList');
  if (!list || list.length === 0) { el.innerHTML = '<p class="empty">Aucun message reçu.</p>'; return; }
  el.innerHTML = '';
  for (const m of list) {
    const d = document.createElement('div');
    d.className = 'card';
    d.innerHTML = '<div class="row"><span class="badge">' + (m.is_read ? 'lu' : 'nouveau') + '</span><strong>' + esc(m.subject || '(sans sujet)') + '</strong></div>' +
      '<p class="meta">De ' + esc(m.name || 'anonyme') + ' ' + (m.email ? '&lt;' + esc(m.email) + '&gt;' : '') + ' · ' + esc(m.created_at || '') + '</p>' +
      '<p>' + esc(m.message).replace(/\n/g, '<br>') + '</p>' +
      (m.is_read ? '' : '<button class="btn ghost mkRead">Marquer comme lu</button>');
    if (!m.is_read) d.querySelector('.mkRead').onclick = async () => { await api('contact_mark_read', { id: m.id }); loadInbox(); refreshStats(); };
    el.appendChild(d);
  }
}

async function refreshStats() {
  try {
    const s = await api('stats', {});
    if (!s) return;
    $('#statsBar').textContent = 'Pages ' + (s.pages || 0) + ' · domaines ' + (s.domains || 0) +
      ' · en attente ' + (s.pending || 0) + ' · bloqués ' + (s.blocked || 0);
    const bookmarkCount = publicMode ? browserBookmarks().length : s.bookmarks;
    $('#bmCount').textContent = bookmarkCount ? '(' + bookmarkCount + ')' : '';
    $('#msgCount').textContent = s.unread ? '(' + s.unread + ')' : '';
    if (publicMode) return;
    const running = await api('crawl_status', {});
    $('#crawlBtn').disabled = running;
    $('#crawlBtn').textContent = running ? 'Crawl en cours...' : 'Crawler le web';
  } catch (e) { $('#statsBar').textContent = 'Statistiques indisponibles'; }
}

$('#searchForm').addEventListener('submit', e => { e.preventDefault(); run($('#q').value.trim()); });
$$('.chip').forEach(c => c.addEventListener('click', () => { $('#q').value = c.dataset.q; run(c.dataset.q); }));
$('#more').addEventListener('click', () => doSearch(currentQuery, true));
$$('.tab').forEach(t => t.addEventListener('click', () => showTab(t.dataset.tab)));
$('.tabs').addEventListener('keydown', event => {
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
  const tabs = $$('.tab').filter(tab => !tab.hidden);
  const index = tabs.indexOf(document.activeElement);
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1
    : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
  event.preventDefault();
  tabs[next].focus(); showTab(tabs[next].dataset.tab);
});

$('#crawlBtn').addEventListener('click', async () => {
  const duration = parseInt($('#durSel').value, 10) || 3600;
  $('#crawlBtn').disabled = true;
  try {
    await api('crawl', { duration });
    await refreshStats();
  } catch (error) {
    $('#crawlBtn').disabled = false;
    $('#crawlBtn').textContent = 'Crawler le web';
    $('#meta').textContent = error.message || String(error);
  }
});

$('#contactForm').addEventListener('submit', async event => {
  event.preventDefault();
  if ($('#contactSend').disabled) return;
  const message = $('#cMessage').value.trim();
  if (!message) { $('#contactState').textContent = 'Message vide.'; return; }
  $('#contactSend').disabled = true;
  try { await api('contact_send', {
    name: $('#cName').value.trim() || null,
    email: $('#cEmail').value.trim() || null,
    subject: $('#cSubject').value.trim() || null,
    message
  });
    $('#cMessage').value = ''; $('#cSubject').value = '';
    $('#contactState').textContent = 'Message enregistré pour l’administrateur.';
  } catch (error) { $('#contactState').textContent = error.message || String(error); }
  finally { $('#contactSend').disabled = false; }
});

$('#view').addEventListener('click', e => { if (e.target.id === 'view') closeView(); });
document.addEventListener('keydown', e => {
  if (!$('#view').classList.contains('open')) return;
  if (e.key === 'Escape') closeView();
  if (e.key === 'Tab') {
    const items = Array.from($('#view').querySelectorAll('button, a[href], summary, [tabindex="0"]')).filter(el => el.getClientRects().length);
    const first = items[0], last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  }
});

$('#q').focus();
configuration.then(() => {
  document.body.dataset.mode = publicMode ? 'public' : 'local';
  if (publicMode) {
    $$('[data-local]').forEach(el => { el.hidden = true; });
    $('#privacyNote').textContent = 'Vertex · index partagé · signets dans ce navigateur uniquement';
  }
  $('#indexMode').textContent = clientConfig.shared ? 'Index partagé' : 'Index personnel';
  if (inTauri && clientConfig.shared) $('#privacyNote').textContent = 'Vertex · index partagé · signets privés sur cet appareil';
  if (inTauri) {
    $('#monitorLink').hidden = true;
    $('#contributionSettings').hidden = false;
    const sync = async () => {
      const settings = await api('contribution_status');
      $('#contributionToggle').checked = settings.enabled;
      $('#backgroundToggle').checked = settings.background;
      $('#backgroundToggle').disabled = !settings.enabled;
      $('#shareToggle').checked = !!settings.share_origin;
      $('#shareToggle').disabled = !settings.enabled || !clientConfig.shared;
      $('#contributionState').textContent = settings.error || (settings.enabled
        ? (settings.running ? 'Exploration active · quota 512 Mo · IP visible aux sites' : 'Démarrage de l’exploration…')
        : (settings.running ? 'Arrêt en cours…' : 'Exploration volontaire désactivée'));
    };
    const change = async () => {
      const enabled = $('#contributionToggle').checked;
      const background = enabled && $('#backgroundToggle').checked;
      const shareDiscoveries = enabled && clientConfig.shared && $('#shareToggle').checked;
      if (enabled && !window.confirm(background
        ? 'Autoriser Vertex à explorer des sites publics avec votre connexion, même après fermeture de la fenêtre ? Les sites verront votre IP. Arrêt possible ici. Pas de démarrage automatique après redémarrage du PC.'
        : 'Autoriser Vertex à explorer des sites publics sur ce PC ? Les sites verront votre IP. Une connexion de crawl, quota de stockage 512 Mo. Aucun signet privé n’est partagé.')) { await sync(); return; }
      if (shareDiscoveries && !window.confirm('Proposer uniquement les URL publiques découvertes, sans paramètres, à ' + clientConfig.origin + ' ? Aucun contenu de page, signet ni historique ne sera envoyé. Le serveur vérifiera ses propres copies.')) { await sync(); return; }
      try { await api('contribution_set', { enabled, background, shareDiscoveries }); }
      catch (error) { showError(error); }
      await sync();
    };
    $('#contributionToggle').addEventListener('change', change);
    $('#backgroundToggle').addEventListener('change', change);
    $('#shareToggle').addEventListener('change', change);
    sync().catch(showError);
    setInterval(() => { if (!document.hidden) sync().catch(showError); }, 4000);
  }
  $$('.tab').forEach(tab => {
    tab.id = 'tab-' + tab.dataset.tab;
    tab.tabIndex = tab.classList.contains('active') ? 0 : -1;
    tab.setAttribute('role', 'tab');
    tab.setAttribute('aria-selected', String(tab.classList.contains('active')));
    tab.setAttribute('aria-controls', 'panel-' + tab.dataset.tab);
    $('#panel-' + tab.dataset.tab).setAttribute('role', 'tabpanel');
    $('#panel-' + tab.dataset.tab).setAttribute('aria-labelledby', tab.id);
  });
  $('#results').innerHTML = '<p class="empty">Aucune recherche en cours.</p>';
  refreshStats();
  setInterval(() => { if (!document.hidden) refreshStats(); }, publicMode ? 30000 : 4000);
}).catch(showError);
