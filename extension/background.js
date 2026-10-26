// RDM browser integration. Same shape as IDM's extension: a thin sensor that records what the
// browser actually sent, spots downloads and media, and hands them to the desktop app.

const API = 'http://127.0.0.1:47123';
const MIN_MEDIA_BYTES = 256 * 1024;
// Headers the engine sets itself (or that would break ranged requests).
const SKIP_HEADERS = new Set(['host', 'connection', 'content-length', 'range', 'if-range', 'accept-encoding',
  'if-none-match', 'if-modified-since']);

// URL -> { method, headers } for recent requests, exactly as sent (extraHeaders includes Cookie).
const recent = new Map();
function remember(url, rec) {
  recent.delete(url);
  recent.set(url, rec);
  if (recent.size > 400) recent.delete(recent.keys().next().value);
}

chrome.webRequest.onBeforeSendHeaders.addListener((d) => {
  const headers = d.requestHeaders
    .filter((h) => !SKIP_HEADERS.has(h.name.toLowerCase()))
    .map((h) => [h.name, h.value ?? '']);
  remember(d.url, { method: d.method, headers });
}, { urls: ['<all_urls>'] }, ['requestHeaders', 'extraHeaders']);

// ---- media detection -------------------------------------------------------------------------

function classify(url, contentType) {
  const mime = contentType.split(';')[0].trim().toLowerCase();
  let path;
  try { path = new URL(url).pathname.toLowerCase(); } catch { return null; }
  if (mime.includes('mpegurl') || path.endsWith('.m3u8')) return 'hls';
  // Pieces of a stream, not something to download on their own.
  if (mime === 'video/mp2t' || mime === 'video/iso.segment' || /\.(ts|m4s)$/.test(path)) return null;
  if (/^(video|audio)\//.test(mime)) return 'file';
  if (mime === 'application/octet-stream' && /\.(mp4|webm|mkv|mov|m4v|mp3|m4a|ogg|opus|flac)$/.test(path)) return 'file';
  return null;
}

function totalSize(h) {
  const range = /\/(\d+)\s*$/.exec(h['content-range'] || '');
  if (range) return Number(range[1]);
  return h['content-length'] ? Number(h['content-length']) : null;
}

chrome.webRequest.onHeadersReceived.addListener((d) => {
  if (d.tabId < 0 || (d.statusCode !== 200 && d.statusCode !== 206)) return;
  const h = Object.fromEntries((d.responseHeaders || []).map((x) => [x.name.toLowerCase(), x.value || '']));
  const kind = classify(d.url, h['content-type'] || '');
  if (!kind) return;
  const size = kind === 'file' ? totalSize(h) : null;
  if (size !== null && size < MIN_MEDIA_BYTES) return;
  addMedia(d.tabId, { url: d.url, kind, mime: h['content-type'] || '', size, headers: recent.get(d.url)?.headers || [] });
}, { urls: ['<all_urls>'] }, ['responseHeaders']);

// Per-tab media lists live in session storage so they survive the service worker sleeping.
const key = (tabId) => `media:${tabId}`;
async function getMedia(tabId) {
  return (await chrome.storage.session.get(key(tabId)))[key(tabId)] || [];
}

let chain = Promise.resolve(); // serialize read-modify-write of the lists
function addMedia(tabId, item) {
  chain = chain.then(async () => {
    const list = await getMedia(tabId);
    if (list.some((m) => m.url === item.url)) return;
    const tab = await chrome.tabs.get(tabId).catch(() => null);
    item.title = tab?.title || '';
    item.pageUrl = tab?.url || '';
    list.push(item);
    await chrome.storage.session.set({ [key(tabId)]: list.slice(-30) });
    chrome.action.setBadgeText({ tabId, text: String(Math.min(list.length, 30)) });
  }).catch(() => {});
}

chrome.tabs.onUpdated.addListener((tabId, info) => {
  if (info.url) chain = chain.then(() => chrome.storage.session.remove(key(tabId))).catch(() => {});
});
chrome.tabs.onRemoved.addListener((tabId) => { chrome.storage.session.remove(key(tabId)); });

// ---- talking to the app ----------------------------------------------------------------------

async function sendToApp(payload) {
  try {
    const r = await fetch(`${API}/download`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    return r.ok;
  } catch {
    return false;
  }
}

async function appAlive() {
  try {
    return (await fetch(`${API}/ping`)).ok;
  } catch {
    return false;
  }
}

// Recorded headers if we saw the request, otherwise rebuild the important ones.
async function headersFor(url, referrer) {
  const rec = recent.get(url);
  if (rec?.headers.length) return rec.headers;
  const headers = [['User-Agent', navigator.userAgent]];
  if (referrer) headers.push(['Referer', referrer]);
  const cookies = await chrome.cookies.getAll({ url }).catch(() => []);
  if (cookies.length) headers.push(['Cookie', cookies.map((c) => `${c.name}=${c.value}`).join('; ')]);
  return headers;
}

function cleanTitle(t) {
  return (t || 'video').replace(/^\(\d+\)\s*/, '').replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').trim().slice(0, 120) || 'video';
}

// The app parses playlists (one parser, in Rust); results are cached per playlist URL.
const variantCache = new Map();
async function hlsVariants(item) {
  if (variantCache.has(item.url)) return variantCache.get(item.url);
  try {
    const headers = item.headers.length ? item.headers : await headersFor(item.url, item.pageUrl);
    const r = await fetch(`${API}/hls/variants`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: item.url, headers }),
    });
    if (!r.ok) return null;
    const variants = await r.json();
    variantCache.set(item.url, variants);
    if (variantCache.size > 100) variantCache.delete(variantCache.keys().next().value);
    return variants;
  } catch {
    return null; // app not running: the stream is still offered, at best quality
  }
}

function fileLabel(m) {
  let name = '';
  try { name = decodeURIComponent(new URL(m.url).pathname.split('/').pop()); } catch {}
  return `${m.mime.split(';')[0] || 'media'} · ${name.slice(0, 40)}`;
}

// What to offer for a tab: every file, and every stream expanded into its qualities.
// A player loads the master playlist and then one of its children (a quality or the audio track);
// those children would only duplicate the master's choices, so they are hidden.
async function entries(tabId) {
  const media = await getMedia(tabId);
  const streams = media.filter((m) => m.kind === 'hls');
  const lists = await Promise.all(streams.map(hlsVariants));
  const children = new Set();
  streams.forEach((m, i) => (lists[i] || []).forEach((v) => {
    if (v.url !== m.url) children.add(v.url);
    if (v.audio_url) children.add(v.audio_url);
  }));
  const out = [];
  streams.forEach((m, i) => {
    if (children.has(m.url)) return;
    if (!lists[i]) out.push({ url: m.url, kind: 'hls', label: 'Stream (HLS)' });
    else for (const v of lists[i]) out.push({ url: m.url, kind: 'hls', label: v.label, audio: Boolean(v.audio_url), variant: v });
  });
  media.filter((m) => m.kind === 'file').sort((a, b) => (b.size || 0) - (a.size || 0))
    .forEach((m) => out.push({ url: m.url, kind: 'file', label: fileLabel(m), size: m.size }));
  return out;
}

async function downloadMedia(tabId, url, variant) {
  const item = (await getMedia(tabId)).find((m) => m.url === url);
  if (!item) return false;
  const headers = item.headers.length ? item.headers : await headersFor(url, item.pageUrl);
  // Stream URLs carry no useful name, so name HLS after the page title.
  const fileName = item.kind === 'hls' ? `${cleanTitle(item.title)}.ts` : null;
  return sendToApp({
    url: variant?.url || url,
    headers,
    file_name: fileName,
    kind: item.kind,
    audio_url: variant?.audio_url || null,
  });
}

// Pages whose streams can't be read from the network (YouTube and friends): the app asks yt-dlp.
const pageCache = new Map();
async function pageFormats(tabId) {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab?.url || !/^https?:/i.test(tab.url)) return { ok: false, error: 'not a web page' };
  if (pageCache.has(tab.url)) return pageCache.get(tab.url);
  // yt-dlp picks YouTube's clients itself; browser cookies would force the more restricted one.
  const youtube = /(^|\.)(youtube\.com|youtu\.be)$/i.test(new URL(tab.url).hostname);
  const headers = youtube ? [] : (await headersFor(tab.url, null)).filter(([k]) => k === 'Cookie');
  try {
    const r = await fetch(`${API}/page/formats`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: tab.url, headers }),
    });
    const result = r.ok ? { ok: true, choices: await r.json() } : { ok: false, error: (await r.text()) || `HTTP ${r.status}` };
    if (result.ok) pageCache.set(tab.url, result);
    if (pageCache.size > 50) pageCache.delete(pageCache.keys().next().value);
    return result;
  } catch {
    return { ok: false, error: 'RDM is not running' };
  }
}

// Everything the video panel offers: streams seen on the network (plus a way to ask the page for
// more), or, when nothing usable went over the network (YouTube), the page's own qualities.
async function videoMenu(tabId) {
  const list = await entries(tabId);
  if (list.length) return { ok: true, list: [...list, { page: true }] };
  const r = await pageFormats(tabId);
  return r.ok ? { ok: true, list: r.choices.map((choice) => ({ choice })) } : r;
}

function downloadChoice(c) {
  return sendToApp({ url: c.url, audio_url: c.audio_url, kind: c.kind, headers: c.headers, file_name: c.file_name });
}

// Several links picked in the popup; each goes to the app with the page as referrer.
async function downloadUrls(tabId, urls) {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  let sent = 0;
  for (const url of urls) {
    if (await sendToApp({ url, headers: await headersFor(url, tab?.url), file_name: null })) sent++;
  }
  return sent;
}

// ---- taking over browser downloads -----------------------------------------------------------

chrome.downloads.onCreated.addListener(async (item) => {
  const { capture = true } = await chrome.storage.local.get('capture');
  const url = item.finalUrl || item.url;
  if (!capture || item.state !== 'in_progress' || !/^https?:/i.test(url)) return;
  // A form POST can't be replayed as a GET; leave those to the browser.
  if (recent.get(url)?.method === 'POST') return;
  if (!(await appAlive())) return;
  await chrome.downloads.pause(item.id).catch(() => {});
  if (await sendToApp({ url, headers: await headersFor(url, item.referrer), file_name: null })) {
    await chrome.downloads.cancel(item.id).catch(() => {});
    chrome.downloads.erase({ id: item.id });
  } else {
    chrome.downloads.resume(item.id).catch(() => {});
  }
});

// ---- context menu, popup and content-script messages -----------------------------------------

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: 'rdm', title: 'Download with RDM', contexts: ['link', 'video', 'audio', 'image'] });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.linkUrl || info.srcUrl;
  if (url && /^https?:/i.test(url)) sendToApp({ url, headers: await headersFor(url, tab?.url), file_name: null });
});

chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  const tabId = msg.tabId ?? sender.tab?.id;
  if (msg.type === 'get-entries') entries(tabId).then(reply);
  else if (msg.type === 'download-media') downloadMedia(tabId, msg.url, msg.variant).then(reply);
  else if (msg.type === 'download-urls') downloadUrls(tabId, msg.urls).then(reply);
  else if (msg.type === 'page-formats') pageFormats(tabId).then(reply);
  else if (msg.type === 'video-menu') videoMenu(tabId).then(reply);
  else if (msg.type === 'download-choice') downloadChoice(msg.choice).then(reply);
  else if (msg.type === 'status') appAlive().then(reply);
  else return false;
  return true; // reply is async
});
