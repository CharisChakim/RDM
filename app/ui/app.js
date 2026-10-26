const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const CATEGORIES = {
  compressed: 'zip rar 7z gz gzip bz2 xz tar tgz arj lzh ace sit sitx sea z iso img',
  documents: 'pdf doc docx xls xlsx ppt pptx pps txt rtf odt ods epub csv',
  music: 'mp3 wav wma aac m4a ogg oga flac opus aif ra mpa',
  programs: 'exe msi msu apk bin dmg deb rpm appimage',
  video: 'mp4 mkv avi mov wmv flv webm m4v mpg mpeg mpe 3gp ogv ts rm rmvb qt asf',
};
const extSets = Object.fromEntries(Object.entries(CATEGORIES).map(([k, v]) => [k, new Set(v.split(' '))]));

let rows = [];
let filter = 'all';
const selected = new Set();
let anchor = null;

const $ = (id) => document.getElementById(id);

function ext(name) {
  const m = /\.([a-z0-9]+)$/i.exec(name);
  return m ? m[1].toLowerCase() : '';
}

function matches(r, f) {
  if (f === 'all') return true;
  if (f === 'finished') return r.status === 'Completed';
  if (f === 'unfinished') return r.status !== 'Completed';
  return extSets[f].has(ext(r.name));
}

function size(b) {
  if (b == null) return '';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i ? (b < 10 ? 2 : 1) : 0)} ${u[i]}`;
}

function eta(r) {
  if (r.status !== 'Downloading' || !r.total || r.speed < 1) return '';
  let s = Math.round((r.total - r.received) / r.speed);
  const h = Math.floor(s / 3600); s %= 3600;
  const m = Math.floor(s / 60); s %= 60;
  return h ? `${h}h ${m}m` : m ? `${m}m ${s}s` : `${s}s`;
}

// HLS has no byte total up front, so its percentage comes from pieces done.
const hasPct = (r) => Boolean(r.total || r.pieces);
function pct(r) {
  if (r.total) return (r.received * 100) / r.total;
  return r.pieces ? (r.pieces[0] * 100) / r.pieces[1] : 0;
}

function statusText(r) {
  switch (r.status) {
    case 'Completed': return 'Complete';
    case 'Error': return 'Error';
    case 'Paused': return hasPct(r) ? `Paused ${pct(r).toFixed(1)}%` : 'Paused';
    default: return hasPct(r) ? `${pct(r).toFixed(1)}%` : size(r.received);
  }
}

// "26/10/26 11:33": short enough for the column; the full date is in the tooltip.
function shortDate(secs) {
  return new Date(secs * 1000).toLocaleString(undefined, {
    day: '2-digit', month: '2-digit', year: '2-digit', hour: '2-digit', minute: '2-digit',
  });
}

function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
}

function render() {
  const visible = rows.filter((r) => matches(r, filter)).sort((a, b) => b.id - a.id);
  $('rows').innerHTML = visible.map((r) => `
    <tr data-id="${r.id}" class="${r.status}${selected.has(r.id) ? ' sel' : ''}" title="${esc(r.error || r.url)}">
      <td>${esc(r.name)}</td>
      <td>${size(r.total ?? (r.status === 'Completed' ? r.received : null))}</td>
      <td><span class="st">${statusText(r)}${r.status === 'Downloading' && hasPct(r) ? `<span class="mini"><i style="width:${pct(r)}%"></i></span>` : ''}</span></td>
      <td>${eta(r)}</td>
      <td>${r.status === 'Downloading' ? size(r.speed) + '/s' : ''}</td>
      <td>${r.status === 'Downloading' ? r.connections : ''}</td>
      <td title="${new Date(r.added * 1000).toLocaleString()}">${shortDate(r.added)}</td>
    </tr>`).join('');
  $('empty').hidden = rows.length > 0;

  for (const a of document.querySelectorAll('.sidebar a')) {
    a.querySelector('b').textContent = rows.filter((r) => matches(r, a.dataset.filter)).length || '';
  }
  const active = rows.filter((r) => r.status === 'Downloading');
  const speed = active.reduce((s, r) => s + r.speed, 0);
  $('totals').textContent = active.length ? `${active.length} active · ${size(speed)}/s` : '';

  const sel = rows.filter((r) => selected.has(r.id));
  $('btn-resume').disabled = !sel.some((r) => r.status === 'Paused' || r.status === 'Error');
  $('btn-pause').disabled = !sel.some((r) => r.status === 'Downloading');
  $('btn-delete').disabled = !sel.length;
  $('btn-folder').disabled = sel.length !== 1 || !sel[0].path;
  renderDetail(sel.length === 1 ? sel[0] : null);
}

function renderDetail(r) {
  $('detail').hidden = !r;
  if (!r) return;
  $('d-name').textContent = r.name;
  const parts = [`${size(r.received)}${r.total ? ' of ' + size(r.total) : ''}`];
  if (r.status === 'Downloading') parts.push(`${size(r.speed)}/s`, `${r.connections} connections`,
    r.pieces ? `${r.pieces[0]}/${r.pieces[1]} stream pieces` : `${r.segments.length} segments`);
  if (r.status === 'Error') parts.push(r.error);
  if (!r.resumable) parts.push('resume not supported by server');
  $('d-stats').textContent = parts.join(' · ');
  $('d-url').textContent = r.url;
  $('d-path').textContent = r.path || '';
  drawBar(r);
}

// IDM-style bar: one block per segment, filled up to what that connection has received.
function drawBar(r) {
  const c = $('d-bar');
  const w = c.clientWidth;
  const dpr = window.devicePixelRatio || 1;
  c.width = w * dpr;
  c.height = 18 * dpr;
  const g = c.getContext('2d');
  g.scale(dpr, dpr);
  const css = getComputedStyle(document.documentElement);
  g.fillStyle = css.getPropertyValue('--bar-bg');
  g.fillRect(0, 0, w, 18);
  g.fillStyle = css.getPropertyValue('--bar-fill');
  if (r.status === 'Completed') { g.fillRect(0, 0, w, 18); return; }
  if (!r.total) { g.fillRect(0, 0, (pct(r) / 100) * w, 18); return; }
  const x = (b) => (b / r.total) * w;
  if (!r.segments.length) { g.fillRect(0, 0, x(r.received), 18); return; }
  for (const [start, got, end] of r.segments) {
    g.fillStyle = css.getPropertyValue('--bar-fill');
    g.fillRect(x(start), 0, Math.max(x(got) - x(start), 0), 18);
    if (got < end && r.status === 'Downloading') {
      g.fillStyle = css.getPropertyValue('--bar-edge');
      g.fillRect(Math.max(x(got) - 1, 0), 0, 2, 18);
    }
  }
}

function selectedIds() { return [...selected]; }

$('rows').addEventListener('mousedown', (e) => {
  const tr = e.target.closest('tr');
  if (!tr) return;
  const id = Number(tr.dataset.id);
  if (e.shiftKey && anchor != null) {
    const ids = [...$('rows').children].map((t) => Number(t.dataset.id));
    const [a, b] = [ids.indexOf(anchor), ids.indexOf(id)].sort((x, y) => x - y);
    selected.clear();
    ids.slice(a, b + 1).forEach((i) => selected.add(i));
  } else if (e.ctrlKey) {
    selected.has(id) ? selected.delete(id) : selected.add(id);
    anchor = id;
  } else {
    selected.clear();
    selected.add(id);
    anchor = id;
  }
  render();
});

$('rows').addEventListener('dblclick', (e) => {
  const tr = e.target.closest('tr');
  if (!tr) return;
  const r = rows.find((x) => x.id === Number(tr.dataset.id));
  if (r?.status === 'Completed') invoke('open', { id: r.id, folder: false });
  else if (r && r.status !== 'Downloading') invoke('resume', { ids: [r.id] });
});

$('sidebar').addEventListener('click', (e) => {
  const a = e.target.closest('a');
  if (!a) return;
  filter = a.dataset.filter;
  document.querySelectorAll('.sidebar a').forEach((x) => x.classList.toggle('active', x === a));
  render();
});

$('btn-add').onclick = () => { $('add-url').value = ''; $('dlg-add').showModal(); };
$('dlg-add').addEventListener('close', async () => {
  if ($('dlg-add').returnValue !== 'ok') return;
  const url = $('add-url').value.trim();
  let job = { url };
  if (/\.m3u8(\?|$)/i.test(url)) {
    const v = await pickQuality(() => invoke('hls_variants', { url }));
    if (!v) return;
    job = { url: v.url, kind: 'hls', audio_url: v.audio_url };
  } else if (VIDEO_SITE.test(url)) {
    // A page, not a file: yt-dlp finds the streams, RDM downloads them.
    const c = await pickQuality(() => invoke('page_formats', { url }));
    if (!c) return;
    job = { url: c.url, kind: c.kind, audio_url: c.audio_url, headers: c.headers, file_name: c.file_name };
  }
  const id = await invoke('add', { job });
  selected.clear();
  selected.add(id);
});

const VIDEO_SITE = /^https?:\/\/([a-z0-9-]+\.)*(youtube\.com|youtu\.be|vimeo\.com|instagram\.com|x\.com|twitter\.com|tiktok\.com|facebook\.com|fb\.watch|dailymotion\.com|twitch\.tv|reddit\.com|bilibili\.com)\//i;

// Opens the quality dialog while `load` runs, then resolves to the chosen item (best first and
// preselected), or null when cancelled or loading failed.
function pickQuality(load) {
  const dlg = $('dlg-quality');
  const ok = dlg.querySelector('button.primary');
  $('q-list').innerHTML = '<div class="qwait">Reading available qualities…</div>';
  ok.disabled = true;
  dlg.showModal();
  let items = [];
  load().then((list) => {
    items = list;
    $('q-list').innerHTML = list.map((v, i) => `
      <label><input type="radio" name="q" value="${i}"${i ? '' : ' checked'}>
        <span>${esc(v.label)}</span>${v.audio_url ? '<small>separate audio, merged</small>' : ''}</label>`).join('');
    ok.disabled = false;
  }, (e) => {
    $('q-list').innerHTML = `<div class="qwait">Could not read qualities: ${esc(e)}</div>`;
  });
  return new Promise((resolve) => {
    dlg.addEventListener('close', () => {
      const checked = dlg.querySelector('input[name=q]:checked');
      resolve(dlg.returnValue === 'ok' && checked ? items[Number(checked.value)] : null);
    }, { once: true });
  });
}
$('btn-resume').onclick = () => invoke('resume', { ids: selectedIds() });
$('btn-pause').onclick = () => invoke('pause', { ids: selectedIds() });
$('btn-delete').onclick = () => {
  const ids = selectedIds();
  if (!ids.length) return;
  const withFiles = confirm(`Remove ${ids.length} download(s).\n\nOK = also delete the files from disk\nCancel = remove from list only`);
  invoke('remove', { ids, deleteFiles: withFiles });
  selected.clear();
};
$('btn-clean').onclick = () => invoke('remove', { ids: rows.filter((r) => r.status === 'Completed').map((r) => r.id), deleteFiles: false });
$('btn-folder').onclick = () => invoke('open', { id: selectedIds()[0], folder: true });

$('btn-settings').onclick = async () => {
  const s = await invoke('get_settings');
  $('set-dir').value = s.dir;
  $('set-conn').value = s.connections;
  $('set-autostart').checked = s.autostart;
  $('dlg-settings').showModal();
};
$('set-browse').onclick = async () => {
  const dir = await invoke('pick_dir');
  if (dir) $('set-dir').value = dir;
};
$('dlg-settings').addEventListener('close', () => {
  if ($('dlg-settings').returnValue !== 'ok') return;
  invoke('set_settings', {
    settings: { dir: $('set-dir').value, connections: Number($('set-conn').value) || 8, autostart: $('set-autostart').checked },
  });
});

document.addEventListener('keydown', (e) => {
  if (document.querySelector('dialog[open]')) return;
  if (e.key === 'Delete') $('btn-delete').click();
  if (e.key === 'a' && e.ctrlKey) {
    e.preventDefault();
    rows.filter((r) => matches(r, filter)).forEach((r) => selected.add(r.id));
    render();
  }
});

// Paste a URL anywhere to start it, like IDM's clipboard catch but explicit.
document.addEventListener('paste', (e) => {
  if (document.querySelector('dialog[open]')) return;
  const text = e.clipboardData.getData('text').trim();
  if (/^https?:\/\//i.test(text)) { $('add-url').value = text; $('dlg-add').showModal(); }
});

window.addEventListener('resize', render);
listen('downloads', (e) => { rows = e.payload; render(); });
invoke('list').then((r) => { rows = r; render(); });
