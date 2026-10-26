// "Download this video" button over playing videos, like IDM's panel (but drawn in the page).
(() => {
  let host, btn, menu, target, hideTimer;

  chrome.runtime.onMessage.addListener((m, _sender, reply) => {
    // Only the top frame answers, so the popup gets one list per page.
    if (m.type === 'scan-links' && window === window.top) reply(scanLinks());
  });

  // File types worth offering, after IDM's default capture list plus a few modern ones.
  const DOWNLOADABLE = new RegExp('\\.(' + [
    '3gp', '7z', 'aac', 'ace', 'aif', 'apk', 'arj', 'asf', 'avi', 'bin', 'bz2', 'exe', 'gz', 'gzip', 'img', 'iso',
    'lzh', 'm4a', 'm4v', 'mkv', 'mov', 'mp3', 'mp4', 'mpa', 'mpe', 'mpeg', 'mpg', 'msi', 'msu', 'ogg', 'ogv', 'pdf',
    'pps', 'ppt', 'qt', 'r\\d\\d', 'ra', 'rar', 'rm', 'rmvb', 'sea', 'sit', 'sitx', 'tar', 'tif', 'tiff', 'wav', 'wma',
    'wmv', 'z', 'zip', 'webm', 'flac', 'opus', 'doc', 'docx', 'xls', 'xlsx', 'pptx', 'epub', 'dmg', 'deb', 'rpm',
    'appimage', 'xz', 'tgz', 'zst', 'torrent',
  ].join('|') + ')$', 'i');

  // Links to files on this page, like IDM's "Download all links".
  function scanLinks() {
    const found = new Map();
    const add = (href, text, explicit) => {
      let u;
      try { u = new URL(href, location.href); } catch { return; }
      if (!/^https?:$/.test(u.protocol) || found.has(u.href)) return;
      if (!explicit && !DOWNLOADABLE.test(decodeURIComponent(u.pathname))) return;
      found.set(u.href, { url: u.href, text: (text || '').replace(/\s+/g, ' ').trim().slice(0, 80) });
    };
    document.querySelectorAll('a[href], area[href]').forEach((a) => add(a.href, a.textContent || a.title, a.hasAttribute('download')));
    document.querySelectorAll('video[src], audio[src], source[src]').forEach((m) => add(m.src, '', false));
    return [...found.values()];
  }

  const size = (b) => (b ? `${(b / 1048576).toFixed(1)} MB` : '');
  // A menu row as three parts: what it is (bold), detail (muted), size (right).
  // Entries come from the background: streams (one per quality) first, then files.
  const parts = (e) => {
    if (e.page) return { main: 'More qualities from this page…', sub: '', size: '', more: true };
    if (e.choice) {
      const [main, sub = '', sz = ''] = e.choice.label.split(' · '); // "1080p60 · H.264 · 255 MB"
      return { main, sub, size: sz };
    }
    const [a, b = ''] = e.label.split(' · ');
    if (e.kind === 'hls') return { main: a, sub: e.audio ? 'HLS + audio' : 'HLS', size: b }; // "720p · 2.0 Mbps"
    return { main: b || a, sub: b ? a : '', size: size(e.size) }; // "video/mp4 · trailer.mp4"
  };
  // The RDM logo, small, inline (no web-accessible files a page could probe for).
  const LOGO = `<svg viewBox="0 0 512 512" aria-hidden="true"><defs><linearGradient id="g" x1=".1" y1="0" x2=".9" y2="1">
    <stop offset="0" stop-color="#5b9bff"/><stop offset=".55" stop-color="#1f6feb"/><stop offset="1" stop-color="#1546c2"/></linearGradient></defs>
    <rect x="20" y="20" width="472" height="472" rx="120" fill="url(#g)"/>
    <g stroke="#fff" stroke-linecap="round" fill="none"><path d="M286 112v190M202 228l84 84 84-84" stroke-width="56" stroke-linejoin="round"/>
    <path d="M174 396h224" stroke-width="44"/><path d="M92 252h82" stroke-width="22" stroke-opacity=".7"/></g></svg>`;
  // The panel appears by itself when a real video starts playing, like IDM's "Download this video"
  // panel, and the qualities are fetched right then, so a click shows them at once.
  let menuFor = null; // page URL the prefetched menu belongs to
  let menuPromise = null;
  let best = '';
  let dismissedUrl = null;

  let title, badge, bar;
  function idle() {
    bar.className = '';
    title.textContent = 'Download';
    badge.textContent = best;
  }
  function busy() {
    title.textContent = 'Reading qualities…';
    badge.textContent = '';
  }
  const flash = (text, ok) => {
    bar.className = ok ? 'ok' : 'err';
    title.textContent = text;
    badge.textContent = '';
    setTimeout(idle, 2500);
  };

  function prefetch() {
    if (menuFor === location.href && menuPromise) return menuPromise;
    menuFor = location.href;
    best = '';
    menuPromise = chrome.runtime.sendMessage({ type: 'video-menu' }).catch(() => null).then((r) => {
      if (r?.ok) {
        const first = r.list.find((e) => !e.page);
        // Streams: their best quality ("2160p60"); plain files: type and size ("MP4 · 1.0 MB").
        const ext = (n) => (/\.([a-z0-9]{2,4})$/i.exec(n || '')?.[1] || '').toUpperCase();
        best = !first ? '' : first.choice || first.kind === 'hls' ? parts(first).main
          : [ext(parts(first).main), size(first.size)].filter(Boolean).join(' · ');
        if (btn) idle();
      } else if (menuFor === location.href) {
        menuPromise = null; // let a click try again
      }
      return r;
    });
    return menuPromise;
  }

  function ui() {
    if (host) return;
    host = document.createElement('div');
    host.style.cssText = 'position:fixed;z-index:2147483647;display:none;';
    const root = host.attachShadow({ mode: 'closed' });
    root.innerHTML = `<style>
      :host{all:initial}
      *{box-sizing:border-box;font-family:"Segoe UI Variable Text","Segoe UI",system-ui,sans-serif}
      #wrap{display:flex;flex-direction:column;align-items:flex-end;animation:pop .18s ease-out}
      @keyframes pop{from{opacity:0;transform:translateY(-4px) scale(.98)}to{opacity:1;transform:none}}
      #bar{display:flex;align-items:stretch;overflow:hidden;border-radius:10px;
        background:rgba(17,21,28,.78);backdrop-filter:blur(12px) saturate(1.4);
        border:1px solid rgba(255,255,255,.14);box-shadow:0 6px 20px rgba(0,0,0,.35),inset 0 1px 0 rgba(255,255,255,.06);
        transition:border-color .2s}
      #bar.ok{border-color:rgba(63,185,80,.75)}
      #bar.err{border-color:rgba(248,81,73,.75)}
      button{all:unset;cursor:pointer;color:#f1f5fb}
      #b{display:flex;align-items:center;gap:8px;padding:7px 11px 7px 8px;font-size:13px;font-weight:600;letter-spacing:.1px}
      #b:hover,#x:hover{background:rgba(255,255,255,.09)}
      .logo svg{display:block;width:20px;height:20px}
      #q{font-size:11px;font-weight:700;color:#d6e4ff;background:rgba(76,141,255,.3);border:1px solid rgba(130,175,255,.4);padding:1px 8px;border-radius:999px}
      #q:empty{display:none}
      .chev{opacity:.65;font-size:10px}
      #x{display:flex;align-items:center;padding:0 10px;border-left:1px solid rgba(255,255,255,.12);color:rgba(241,245,251,.65);font-size:11px}
      #m{display:none;margin-top:6px;width:310px;max-height:350px;overflow:auto;padding:6px;border-radius:12px;
        background:rgba(17,21,28,.93);backdrop-filter:blur(14px);border:1px solid rgba(255,255,255,.14);
        box-shadow:0 14px 34px rgba(0,0,0,.45);animation:pop .14s ease-out}
      .h{font-size:10.5px;font-weight:600;letter-spacing:.07em;text-transform:uppercase;color:rgba(241,245,251,.5);padding:6px 10px 5px}
      .it{display:flex;align-items:baseline;gap:8px;padding:8px 10px;border-radius:8px;color:#f1f5fb;cursor:pointer;font-size:13px}
      .it:hover{background:rgba(76,141,255,.22)}
      .main{font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:170px}
      .sub{flex:1;color:rgba(241,245,251,.55);font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
      .size{color:rgba(241,245,251,.75);font-size:12px;font-variant-numeric:tabular-nums;white-space:nowrap}
      .more .main{color:#9ec2ff;font-weight:500}
    </style><div id="wrap"><div id="bar"><button id="b" title="Download with RDM"><span class="logo">${LOGO}</span>
      <span id="t"></span><span id="q"></span><span class="chev">&#9662;</span></button>
      <button id="x" title="Hide for this page">&#x2715;</button></div>
      <div id="m" role="menu"><div class="h">Download with RDM</div><div id="items"></div></div></div>`;
    btn = root.getElementById('b');
    menu = root.getElementById('m');
    bar = root.getElementById('bar');
    title = root.getElementById('t');
    badge = root.getElementById('q');
    idle();
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      if (menu.style.display === 'block') { menu.style.display = 'none'; return; }
      if (menuFor !== location.href || !menuPromise) busy();
      const r = await prefetch();
      if (!r?.ok) return flash(`${r?.error || 'RDM is not running'}`.slice(0, 60), false);
      idle();
      if (r.list.length === 1) return send(r.list[0]);
      show(r.list);
    });
    root.getElementById('x').addEventListener('click', (e) => {
      e.stopPropagation();
      dismissedUrl = location.href;
      hide();
    });
    host.addEventListener('mouseenter', () => clearTimeout(hideTimer));
    document.documentElement.appendChild(host);
  }

  function show(list) {
    const items = menu.querySelector('#items');
    items.textContent = '';
    for (const entry of list) {
      const p = parts(entry);
      const row = document.createElement('div');
      row.className = p.more ? 'it more' : 'it';
      row.setAttribute('role', 'menuitem');
      for (const [cls, text] of [['main', p.main], ['sub', p.sub], ['size', p.size]]) {
        const span = document.createElement('span');
        span.className = cls;
        span.textContent = text;
        row.appendChild(span);
      }
      row.title = entry.choice?.file_name || entry.variant?.url || entry.url || '';
      row.onclick = (ev) => { ev.stopPropagation(); send(entry); };
      items.appendChild(row);
    }
    menu.style.display = 'block';
  }

  async function pageQualities() {
    menu.style.display = 'none';
    busy();
    const r = await chrome.runtime.sendMessage({ type: 'page-formats' }).catch(() => null);
    if (!r?.ok) return flash(`${r?.error || 'RDM is not running'}`.slice(0, 60), false);
    idle();
    show(r.choices.map((choice) => ({ choice })));
  }

  function send(entry) {
    if (entry.page) return pageQualities();
    const msg = entry.choice
      ? { type: 'download-choice', choice: entry.choice }
      : { type: 'download-media', url: entry.url, variant: entry.variant };
    chrome.runtime.sendMessage(msg).then((ok) => flash(ok ? '✓ Sent to RDM' : 'RDM is not running', ok));
    menu.style.display = 'none';
  }

  // Pinned to the video's top-right corner, inside the visible part of the window.
  function place(v) {
    const r = v.getBoundingClientRect();
    const w = host.offsetWidth || 220;
    host.style.top = `${Math.min(Math.max(r.top + 8, 4), window.innerHeight - 40)}px`;
    host.style.left = `${Math.max(Math.min(r.right, window.innerWidth) - w - 8, r.left + 8, 4)}px`;
    host.style.display = 'block';
  }

  function hide() {
    clearTimeout(hideTimer);
    if (host) {
      host.style.display = 'none';
      menu.style.display = 'none';
    }
    target = null;
  }

  function hideSoon() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(hide, 1500);
  }

  const visible = (v) => {
    const r = v.getBoundingClientRect();
    return v.isConnected && r.width >= 160 && r.height >= 90 && r.bottom > 0 && r.top < window.innerHeight;
  };
  // A real video, not an ad clip or an animated thumbnail.
  const worthy = (v) => {
    const r = v.getBoundingClientRect();
    return r.width >= 320 && r.height >= 180 && !(v.duration > 0 && v.duration <= 10);
  };

  function attach(v) {
    if (dismissedUrl === location.href) return;
    ui();
    clearTimeout(hideTimer);
    target = v;
    place(v);
  }

  // Shows itself when a video starts playing, and starts reading the qualities right away.
  document.addEventListener('playing', (e) => {
    const v = e.target;
    if (!(v instanceof HTMLVideoElement) || !worthy(v) || dismissedUrl === location.href) return;
    attach(v);
    prefetch();
  }, true);

  // Players cover the <video> with their own controls, so hit-test by geometry rather than event target.
  let pending = false;
  document.addEventListener('mousemove', (e) => {
    if (pending) return;
    pending = true;
    requestAnimationFrame(() => {
      pending = false;
      const v = [...document.querySelectorAll('video')].find((el) => {
        const r = el.getBoundingClientRect();
        return r.width >= 160 && r.height >= 90 && e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
      });
      if (v) {
        attach(v);
      } else if (target && target.paused) {
        hideSoon(); // a playing video keeps its panel, like IDM
      }
    });
  }, { passive: true });

  // Follow the video through scrolling, layout changes and SPA navigation; drop it when it's gone.
  // Also catches videos that started before this script loaded (YouTube autoplays that early).
  setInterval(() => {
    if (!target) {
      if (dismissedUrl === location.href) return;
      const v = [...document.querySelectorAll('video')].find((el) => !el.paused && worthy(el) && visible(el));
      if (v) {
        attach(v);
        prefetch();
      }
      return;
    }
    if (!visible(target)) return hide();
    place(target);
  }, 400);
})();
