// "Download this video" button over playing videos, like IDM's panel (but drawn in the page).
(() => {
  let media = [];
  let host, btn, menu, target, hideTimer;

  const refresh = () => chrome.runtime.sendMessage({ type: 'get-media' }).then((m) => { media = m || []; }).catch(() => {});
  chrome.runtime.onMessage.addListener((m, _sender, reply) => {
    if (m.type === 'media-updated') refresh();
    // Only the top frame answers, so the popup gets one list per page.
    if (m.type === 'scan-links' && window === window.top) reply(scanLinks());
  });
  refresh();

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
  const label = (e) => (e.kind === 'hls' ? `Video ${e.label}${e.audio ? ' + audio' : ''}` : `${e.label}${size(e.size)}`);

  function ui() {
    if (host) return;
    host = document.createElement('div');
    host.style.cssText = 'position:fixed;z-index:2147483647;display:none;';
    const root = host.attachShadow({ mode: 'closed' });
    root.innerHTML = `<style>
      button,a{font:600 12px/1.2 "Segoe UI",system-ui,sans-serif}
      #b{display:flex;gap:6px;align-items:center;padding:6px 10px;border:0;border-radius:6px;background:#1f6feb;color:#fff;cursor:pointer;box-shadow:0 2px 8px rgb(0 0 0/.35)}
      #b:hover{background:#1858c4}
      #m{display:none;margin-top:4px;min-width:260px;max-width:420px;background:#1b2027;border-radius:6px;box-shadow:0 4px 14px rgb(0 0 0/.45);overflow:hidden}
      #m a{display:block;padding:7px 10px;color:#e3e8ef;cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-weight:400}
      #m a:hover{background:#2b3a52}
    </style><button id="b">&#11015; Download this video</button><div id="m"></div>`;
    btn = root.getElementById('b');
    menu = root.getElementById('m');
    btn.addEventListener('click', async (e) => {
      e.stopPropagation();
      if (menu.style.display === 'block') { menu.style.display = 'none'; return; }
      btn.textContent = '… Reading qualities';
      const list = (await chrome.runtime.sendMessage({ type: 'get-entries' }).catch(() => null)) || [];
      btn.innerHTML = '&#11015; Download this video';
      if (list.length === 1) return send(list[0]);
      menu.innerHTML = '';
      for (const entry of list) {
        const a = document.createElement('a');
        a.textContent = label(entry);
        a.title = entry.variant?.url || entry.url;
        a.onclick = (ev) => { ev.stopPropagation(); send(entry); };
        menu.appendChild(a);
      }
      menu.style.display = 'block';
    });
    host.addEventListener('mouseenter', () => clearTimeout(hideTimer));
    document.documentElement.appendChild(host);
  }

  function send(entry) {
    chrome.runtime.sendMessage({ type: 'download-media', url: entry.url, variant: entry.variant }).then((ok) => {
      btn.textContent = ok ? '✔ Sent to RDM' : '✖ RDM is not running';
      setTimeout(() => { btn.innerHTML = '&#11015; Download this video'; }, 2000);
    });
    menu.style.display = 'none';
  }

  function place(v) {
    const r = v.getBoundingClientRect();
    host.style.top = `${Math.max(r.top + 8, 4)}px`;
    host.style.left = `${Math.max(r.right - 200, r.left + 8)}px`;
    host.style.display = 'block';
  }

  function hideSoon() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => { if (host) { host.style.display = 'none'; menu.style.display = 'none'; } target = null; }, 1500);
  }

  // Players cover the <video> with their own controls, so hit-test by geometry rather than event target.
  let pending = false;
  document.addEventListener('mousemove', (e) => {
    if (pending || !media.length) return;
    pending = true;
    requestAnimationFrame(() => {
      pending = false;
      const v = [...document.querySelectorAll('video')].find((el) => {
        const r = el.getBoundingClientRect();
        return r.width >= 160 && r.height >= 90 && e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
      });
      if (v) {
        ui();
        clearTimeout(hideTimer);
        target = v;
        place(v);
      } else if (target) {
        hideSoon();
      }
    });
  }, { passive: true });

  window.addEventListener('scroll', () => { if (target) place(target); }, { passive: true });
})();
