const $ = (id) => document.getElementById(id);

chrome.runtime.sendMessage({ type: 'status' }).then((on) => {
  $('status').className = on ? 'on' : 'off';
  $('status').textContent = on ? 'RDM running' : 'RDM not running';
});

// File links on the page, picked with checkboxes and sent to RDM together.
(async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const links = (await chrome.tabs.sendMessage(tab.id, { type: 'scan-links' }).catch(() => null)) || [];
  if (!links.length) {
    $('links').outerHTML = '<div class="empty">No file links found on this page.</div>';
    $('links-bar').hidden = true;
    return;
  }
  const boxes = [];
  const update = () => {
    const n = boxes.filter((b) => b.checked).length;
    $('dl-links').disabled = !n;
    $('dl-links').textContent = n ? `Download selected (${n})` : 'Download selected';
    $('all').checked = n === boxes.length;
  };
  for (const l of links) {
    const li = document.createElement('li');
    li.className = 'pick';
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.value = l.url;
    box.onchange = update;
    boxes.push(box);
    let path = l.url;
    try { path = decodeURIComponent(new URL(l.url).pathname); } catch {}
    const ext = /\.([a-z0-9]+)$/i.exec(path)?.[1];
    const span = document.createElement('span');
    // Without an extension the path says little ("/get?id=5"), so show the link text instead.
    span.textContent = (ext ? path.split('/').pop() : l.text) || l.url;
    span.title = l.text ? `${l.text}\n${l.url}` : l.url;
    span.onclick = () => { box.checked = !box.checked; update(); };
    const small = document.createElement('small');
    small.textContent = (ext || 'link').toUpperCase();
    li.append(box, span, small);
    $('links').appendChild(li);
  }
  $('all').onchange = (e) => { boxes.forEach((b) => { b.checked = e.target.checked; }); update(); };
  $('dl-links').onclick = async () => {
    const urls = boxes.filter((b) => b.checked).map((b) => b.value);
    $('dl-links').disabled = true;
    const sent = await chrome.runtime.sendMessage({ type: 'download-urls', tabId: tab.id, urls });
    $('dl-links').textContent = !sent ? 'RDM is not running' : sent === urls.length ? `Sent ${sent} to RDM` : `Sent ${sent} of ${urls.length}`;
  };
})();

chrome.storage.local.get('capture').then(({ capture = true }) => { $('capture').checked = capture; });
$('capture').onchange = (e) => chrome.storage.local.set({ capture: e.target.checked });

(async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const entries = (await chrome.runtime.sendMessage({ type: 'get-entries', tabId: tab.id })) || [];
  if (!entries.length) {
    $('list').outerHTML = '<div class="empty">Nothing detected yet. Start playing the video, then open this again.</div>';
    return;
  }
  for (const e of entries) {
    const li = document.createElement('li');
    const span = document.createElement('span');
    span.textContent = e.kind === 'hls' ? `Video ${e.label}` : e.label;
    span.title = e.variant?.url || e.url;
    const small = document.createElement('small');
    small.textContent = e.kind === 'hls' ? (e.audio ? 'HLS + audio' : 'HLS') : e.size ? `${(e.size / 1048576).toFixed(1)} MB` : '';
    const btn = document.createElement('button');
    btn.textContent = 'Download';
    btn.onclick = async () => {
      const ok = await chrome.runtime.sendMessage({ type: 'download-media', tabId: tab.id, url: e.url, variant: e.variant });
      btn.textContent = ok ? 'Sent' : 'Failed';
    };
    li.append(span, small, btn);
    $('list').appendChild(li);
  }
})();
