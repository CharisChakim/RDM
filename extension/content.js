// "Download this video" button over playing videos, like IDM's panel (but drawn in the page).
(() => {
  let media = [];
  let host, btn, menu, target, hideTimer;

  const refresh = () => chrome.runtime.sendMessage({ type: 'get-media' }).then((m) => { media = m || []; }).catch(() => {});
  chrome.runtime.onMessage.addListener((m) => { if (m.type === 'media-updated') refresh(); });
  refresh();

  const size = (b) => (b ? `${(b / 1048576).toFixed(1)} MB` : '');
  const label = (m) => {
    let name = '';
    try { name = decodeURIComponent(new URL(m.url).pathname.split('/').pop()); } catch {}
    return `${m.kind === 'hls' ? 'Stream (HLS)' : (m.mime.split(';')[0] || 'media')} · ${name.slice(0, 40)} ${size(m.size)}`;
  };
  // Streams first (they are the real video on most sites), then biggest files.
  const ordered = () => [...media].sort((a, b) => (b.kind === 'hls') - (a.kind === 'hls') || (b.size || 0) - (a.size || 0));

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
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      const list = ordered();
      if (list.length === 1) return send(list[0].url);
      menu.innerHTML = '';
      for (const m of list) {
        const a = document.createElement('a');
        a.textContent = label(m);
        a.title = m.url;
        a.onclick = (ev) => { ev.stopPropagation(); send(m.url); };
        menu.appendChild(a);
      }
      menu.style.display = menu.style.display === 'block' ? 'none' : 'block';
    });
    host.addEventListener('mouseenter', () => clearTimeout(hideTimer));
    document.documentElement.appendChild(host);
  }

  function send(url) {
    chrome.runtime.sendMessage({ type: 'download-media', url }).then((ok) => {
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
