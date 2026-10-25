const $ = (id) => document.getElementById(id);

chrome.runtime.sendMessage({ type: 'status' }).then((on) => {
  $('status').className = on ? 'on' : 'off';
  $('status').textContent = on ? 'RDM running' : 'RDM not running';
});

chrome.storage.local.get('capture').then(({ capture = true }) => { $('capture').checked = capture; });
$('capture').onchange = (e) => chrome.storage.local.set({ capture: e.target.checked });

(async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const media = (await chrome.runtime.sendMessage({ type: 'get-media', tabId: tab.id })) || [];
  if (!media.length) {
    $('list').outerHTML = '<div class="empty">Nothing detected yet. Start playing the video, then open this again.</div>';
    return;
  }
  for (const m of [...media].reverse()) {
    const li = document.createElement('li');
    let name = m.url;
    try { name = decodeURIComponent(new URL(m.url).pathname.split('/').pop()) || m.url; } catch {}
    const span = document.createElement('span');
    span.textContent = name;
    span.title = m.url;
    const small = document.createElement('small');
    small.textContent = m.kind === 'hls' ? 'HLS' : m.size ? `${(m.size / 1048576).toFixed(1)} MB` : m.mime.split(';')[0];
    const btn = document.createElement('button');
    btn.textContent = 'Download';
    btn.onclick = async () => {
      const ok = await chrome.runtime.sendMessage({ type: 'download-media', tabId: tab.id, url: m.url });
      btn.textContent = ok ? 'Sent' : 'Failed';
    };
    li.append(span, small, btn);
    $('list').appendChild(li);
  }
})();
