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
  const LOGO = `<svg aria-hidden="true" viewBox="0 0 512 512" stroke-linejoin="round" stroke-linecap="round"><defs><linearGradient id="crab" x1="0.2" y1="0" x2="0.8" y2="1"><stop offset="0" stop-color="#4f8cff"/><stop offset="1" stop-color="#1f4fd6"/></linearGradient><radialGradient id="ocean" cx="0.38" cy="0.32" r="0.75"><stop offset="0" stop-color="#6cc6f2"/><stop offset="0.5" stop-color="#1f7fc4"/><stop offset="1" stop-color="#0a3b78"/></radialGradient><linearGradient id="land" x1="0" y1="0" x2="0.6" y2="1"><stop offset="0" stop-color="#bfe07a"/><stop offset="1" stop-color="#4f9a3a"/></linearGradient><radialGradient id="shade" cx="0.38" cy="0.32" r="0.72"><stop offset="0.55" stop-color="#001a4d" stop-opacity="0"/><stop offset="1" stop-color="#001a4d" stop-opacity="0.5"/></radialGradient><linearGradient id="gloss" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff" stop-opacity="0.6"/><stop offset="1" stop-color="#ffffff" stop-opacity="0"/></linearGradient><linearGradient id="faceTop" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#e4ebf4"/></linearGradient><linearGradient id="faceLow" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#c3cfdf"/><stop offset="1" stop-color="#93a4bd"/></linearGradient><clipPath id="globe"><circle cx="300" cy="285" r="135"/></clipPath><clipPath id="front"><path d="M-88.1 381.8L688.1 188.2L688.1 600L-88.1 600Z"/></clipPath></defs>
    <g transform="translate(256 288) scale(0.84) translate(-300 -285)"><path d="M150 316L118 366L114 410M176 320L150 372L152 414M450 316L482 366L486 410M424 320L450 372L448 414" fill="none" stroke="#0c2c7a" stroke-width="16"/><path d="M150 316L118 366L114 410M176 320L150 372L152 414M450 316L482 366L486 410M424 320L450 372L448 414" fill="none" stroke="#2f63f0" stroke-width="9"/><path d="M130.3 203.2L115.6 175L148.1 169L140.3 138.4L173.3 139.4L172.8 107.9L204.8 115.8L211.5 85L240.9 99.5L254.7 70.8L280 91.1L300 66L320 91.1L345.3 70.8L359.1 99.5L388.5 85L395.2 115.8L427.2 107.9L426.7 139.4L459.7 138.4L451.9 169L484.4 175L469.7 203.2L486 300Q486 320 466 320H134Q114 320 114 300Z" fill="url(#crab)" stroke="#0c2c7a" stroke-width="6"/><ellipse cx="256" cy="126" rx="21" ry="25" fill="#081534"/><ellipse cx="344" cy="126" rx="21" ry="25" fill="#081534"/><g fill="#fff"><circle cx="263" cy="117" r="8"/><circle cx="250" cy="136" r="3"/><circle cx="351" cy="117" r="8"/><circle cx="338" cy="136" r="3"/></g><path d="M402 236C452 226 506.8 171.2 480.8 195.2M196 238C150 226 110 226 112 238" fill="none" stroke="#0c2c7a" stroke-width="20"/><path d="M402 236C452 226 506.8 171.2 480.8 195.2M196 238C150 226 110 226 112 238" fill="none" stroke="#2f63f0" stroke-width="13"/><path transform="translate(0 52)" d="M112 186C88 182 72 200 76 222L98 212L100 236C122 238 136 222 134 202C132 192 124 186 112 186Z" fill="url(#crab)" stroke="#0c2c7a" stroke-width="5"/><g fill="none"><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#5d6f8c" stroke-width="48"/><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#a9b7cb" stroke-width="42"/><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#d6dee9" stroke-width="32" transform="translate(0 -3)"/><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#f0f4f9" stroke-width="22" transform="translate(0 -4.5)"/><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#ffffff" stroke-width="10" transform="translate(0 -6)"/></g><circle cx="300" cy="285" r="135" fill="url(#ocean)"/><g clip-path="url(#globe)"><path d="M200 230L228 205L262 196L292 206L306 222L300 238L316 248L306 262L292 270L284 290L292 302L280 306L262 290L244 280L226 268L198 254ZM296 168L322 172L328 186L312 194L298 186ZM294 312L310 318L338 330L354 346L344 370L326 396L314 410L308 384L298 356L290 336ZM392 214L410 202L434 214L432 246L412 244Z" fill="url(#land)" stroke="#3f8a33" stroke-width="2.5"/><circle cx="300" cy="285" r="135" fill="url(#shade)"/><ellipse cx="266" cy="209" rx="86" ry="44" fill="url(#gloss)"/></g><circle cx="300" cy="285" r="135" fill="none" stroke="#0a3b78" stroke-width="4"/><path transform="translate(22.8 -96.8)" d="M458 292C432 292 418 312 424 334L446 320L452 344C474 342 486 324 482 306C478 296 468 292 458 292Z" fill="url(#crab)" stroke="#0c2c7a" stroke-width="5"/><g fill="none" clip-path="url(#front)"><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#5d6f8c" stroke-width="48"/><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#a9b7cb" stroke-width="42"/><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#d6dee9" stroke-width="32" transform="translate(0 -3)"/><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#f0f4f9" stroke-width="22" transform="translate(0 -4.5)"/><path d="M389.9 207.3L383.3 207.8L376.7 208.4L370 209.2L363.2 210L356.3 210.9L349.3 211.9L342.3 213L335.2 214.2L328.1 215.5L320.9 216.8L313.8 218.3L306.6 219.8L299.4 221.4L292.2 223.1L285 224.8L277.8 226.7L270.7 228.6L263.6 230.5L256.5 232.6L249.5 234.7L242.6 236.8L235.7 239L228.9 241.3L222.2 243.6L215.6 246L209 248.4L202.6 250.9L196.3 253.4L190.2 256L184.2 258.5L178.3 261.2L172.5 263.8L167 266.5L161.5 269.2L156.3 271.9L151.2 274.6L146.3 277.4L141.6 280.1L137.1 282.9L132.8 285.6L128.7 288.4L124.8 291.1L121.1 293.9L117.6 296.6L114.4 299.3L111.3 302L108.5 304.7L106 307.4L103.7 310L101.6 312.6L99.7 315.2L98.1 317.7L96.8 320.2L95.7 322.7L94.8 325.1L94.2 327.4L93.9 329.7L93.8 332L93.9 334.2L94.3 336.3L94.9 338.4L95.8 340.4L97 342.3L98.4 344.2L100 346L101.9 347.7L104 349.3L106.4 350.9L109 352.4L111.8 353.8L114.9 355.1L118.2 356.3L121.7 357.5L125.4 358.5L129.4 359.5L133.5 360.4L137.9 361.2L142.4 361.9L147.1 362.5L152.1 363L157.2 363.4L162.4 363.7L167.9 363.9L173.5 364L179.3 364.1L185.2 364L191.2 363.8L197.4 363.6L203.7 363.2L210.1 362.7L216.7 362.2L223.3 361.6L230 360.8L236.8 360L243.7 359.1L250.7 358.1L257.7 357L264.8 355.8L271.9 354.5L279.1 353.2L286.2 351.7L293.4 350.2L300.6 348.6L307.8 346.9L315 345.2L322.2 343.3L329.3 341.4L336.4 339.5L343.5 337.4L350.5 335.3L357.4 333.2L364.3 331L371.1 328.7L377.8 326.4L384.4 324L391 321.6L397.4 319.1L403.7 316.6L409.8 314L415.8 311.5L421.7 308.8L427.5 306.2L433 303.5L438.5 300.8" stroke="#ffffff" stroke-width="10" transform="translate(0 -6)"/></g><path d="M412.4 249L480.5 295.4L464.5 352.7L426 311.6Z" fill="#b4c1d3"/><path d="M412.4 249L572.5 233.6L480.5 295.4Z" fill="url(#faceTop)"/><path d="M480.5 295.4L572.5 233.6L464.5 352.7Z" fill="url(#faceLow)"/><path d="M412.4 249L572.5 233.6L464.5 352.7L426 311.6ZM412.4 249L480.5 295.4L572.5 233.6M480.5 295.4L464.5 352.7" fill="none" stroke="#5d6f8c" stroke-width="3"/></g></svg>`;
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
