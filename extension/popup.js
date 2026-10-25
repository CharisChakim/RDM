const $ = (id) => document.getElementById(id);

chrome.runtime.sendMessage({ type: 'status' }).then((on) => {
  $('status').className = on ? 'on' : 'off';
  $('status').textContent = on ? 'RDM running' : 'RDM not running';
});

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
