// Local benchmark server: one generated file with Range support, optionally throttled per
// connection (many real servers cap each connection, which is where multi-connection wins).
// usage: node serve.js <sizeMiB> [perConnectionKiBps, 0 = unlimited] [port]
const http = require('http');
const crypto = require('crypto');

const size = Number(process.argv[2] || 1024) * 1048576;
const rate = Number(process.argv[3] || 0) * 1024;
const port = Number(process.argv[4] || 47950);
const block = crypto.randomBytes(1 << 20); // repeated; content doesn't matter for timing

http.createServer((req, res) => {
  let start = 0;
  let end = size - 1;
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.range || '');
  if (m) {
    start = m[1] ? Number(m[1]) : 0;
    end = m[2] ? Math.min(Number(m[2]), size - 1) : size - 1;
  }
  const headers = {
    'Content-Length': end - start + 1,
    'Accept-Ranges': 'bytes',
    'Content-Type': 'application/octet-stream',
    'Content-Disposition': 'attachment; filename="test.bin"',
  };
  if (m) headers['Content-Range'] = `bytes ${start}-${end}/${size}`;
  res.writeHead(m ? 206 : 200, headers);
  if (req.method === 'HEAD') return res.end();

  let pos = start;
  let sent = 0;
  const t0 = Date.now();
  res.on('close', () => { pos = end + 1; });
  const pump = () => {
    while (pos <= end) {
      if (rate && sent >= ((Date.now() - t0) / 1000) * rate) return setTimeout(pump, 10);
      const off = pos % block.length;
      const n = Math.min(block.length - off, end - pos + 1, 256 * 1024);
      const more = res.write(block.subarray(off, off + n));
      pos += n;
      sent += n;
      if (!more) return res.once('drain', pump);
    }
    res.end();
  };
  pump();
}).listen(port, '127.0.0.1', () => console.log(`serving ${size} bytes at http://127.0.0.1:${port}/test.bin`));
