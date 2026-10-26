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

  const size = (b) => (b ? ` ${(b / 1048576).toFixed(1)} MB` : '');
  // Entries come from the background: streams (one per quality) first, then files.
  const label = (e) => {
    if (e.page) return 'More qualities from this page…';
    if (e.choice) return e.choice.label.startsWith('Audio') ? e.choice.label : `Video ${e.choice.label}`;
    return e.kind === 'hls' ? `Video ${e.label}${e.audio ? ' + audio' : ''}` : `${e.label}${size(e.size)}`;
  };
  // The panel appears by itself when a real video starts playing, like IDM's "Download this video"
  // panel, and the qualities are fetched right then, so a click shows them at once.
  let menuFor = null; // page URL the prefetched menu belongs to
  let menuPromise = null;
  let best = '';
  let dismissedUrl = null;

  function idle() {
    btn.textContent = '⬇ Download this video';
    if (best) {
      const small = document.createElement('small');
      small.textContent = best;
      btn.append(' ', small);
    }
  }
  const flash = (text) => {
    btn.textContent = text;
    setTimeout(idle, 2500);
  };

  function prefetch() {
    if (menuFor === location.href && menuPromise) return menuPromise;
    menuFor = location.href;
    best = '';
    menuPromise = chrome.runtime.sendMessage({ type: 'video-menu' }).catch(() => null).then((r) => {
      if (r?.ok) {
        const first = r.list.find((e) => !e.page);
        // "2160p60 · AV1 · 689 MB" -> "2160p60"
        best = first ? label(first).replace(/^Video /, '').split(' · ')[0] : '';
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
      button,a{font:600 12px/1.2 "Segoe UI",system-ui,sans-serif}
      #row{display:flex;gap:2px;justify-content:flex-end}
      #b{display:flex;gap:6px;align-items:center;padding:6px 10px;border:0;border-radius:6px 0 0 6px;background:#1f6feb;color:#fff;cursor:pointer;box-shadow:0 2px 8px rgb(0 0 0/.35)}
      #b small{font-weight:400;opacity:.85}
      #b:hover,#x:hover{background:#1858c4}
      #x{padding:6px 8px;border:0;border-radius:0 6px 6px 0;background:#1f6feb;color:#fff;cursor:pointer;box-shadow:0 2px 8px rgb(0 0 0/.35)}
      #m{display:none;margin-top:4px;min-width:260px;max-width:420px;background:#1b2027;border-radius:6px;box-shadow:0 4px 14px rgb(0 0 0/.45);overflow:hidden}
      #m a{display:block;padding:7px 10px;color:#e3e8ef;cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-weight:400}
      #m a:hover{background:#2b3a52}
    </style><div id="row"><button id="b"></button><button id="x" title="Hide for this video">&#x2715;</button></div><div id="m"></div>`;
    btn = root.getElementById('b');
    menu = root.getElementById('m');
    idle();
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      if (menu.style.display === 'block') { menu.style.display = 'none'; return; }
      const pending = menuFor !== location.href || !menuPromise;
      if (pending) btn.textContent = '… Reading qualities';
      const r = await prefetch();
      if (!r?.ok) return flash(`✖ ${r?.error || 'RDM is not running'}`.slice(0, 80));
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
    menu.innerHTML = '';
    for (const entry of list) {
      const a = document.createElement('a');
      a.textContent = label(entry);
      a.title = entry.choice?.url || entry.variant?.url || entry.url || '';
      a.onclick = (ev) => { ev.stopPropagation(); send(entry); };
      menu.appendChild(a);
    }
    menu.style.display = 'block';
  }

  async function pageQualities() {
    menu.style.display = 'none';
    btn.textContent = '… Reading qualities';
    const r = await chrome.runtime.sendMessage({ type: 'page-formats' }).catch(() => null);
    if (!r?.ok) return flash(`✖ ${r?.error || 'RDM is not running'}`.slice(0, 80));
    idle();
    show(r.choices.map((choice) => ({ choice })));
  }

  function send(entry) {
    if (entry.page) return pageQualities();
    const msg = entry.choice
      ? { type: 'download-choice', choice: entry.choice }
      : { type: 'download-media', url: entry.url, variant: entry.variant };
    chrome.runtime.sendMessage(msg).then((ok) => flash(ok ? '✔ Sent to RDM' : '✖ RDM is not running'));
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
  setInterval(() => {
    if (!target) return;
    if (!visible(target)) return hide();
    place(target);
  }, 400);
})();
