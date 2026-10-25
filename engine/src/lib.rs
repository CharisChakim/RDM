mod fsio;
mod hls;
pub mod names;
pub mod segments;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_RANGE, ETAG, LAST_MODIFIED, RANGE};
use reqwest::{Response, StatusCode};
use segments::{Segment, Segments};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub use reqwest::Client;

const BUF_SIZE: usize = 1 << 20;
const MIN_SPLIT: u64 = 1 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RETRIES: u32 = 5;
const UNKNOWN: u64 = u64::MAX;
// An honest UA: servers with bot detection drop requests that claim to be Chrome but lack
// Chrome's other headers. Browser hand-offs send the browser's real UA anyway.
const DEFAULT_UA: &str = concat!("RDM/", env!("CARGO_PKG_VERSION"));

pub fn client() -> Client {
    Client::builder()
        .user_agent(DEFAULT_UA)
        .connect_timeout(Duration::from_secs(15))
        .tcp_nodelay(true)
        .build()
        .expect("http client")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub url: String,
    /// Extra headers from the browser: Cookie, Referer, User-Agent...
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub dir: PathBuf,
    /// Overrides the server-provided name. Pass the previous name to resume a paused download.
    #[serde(default)]
    pub file_name: Option<String>,
    pub connections: usize,
    #[serde(default)]
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    File,
    /// An .m3u8 playlist: fetched segment by segment and joined into one file.
    Hls,
}

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub path: Option<PathBuf>,
    pub total: Option<u64>,
    pub received: u64,
    pub connections: usize,
    pub resumable: bool,
    /// [start, received_up_to, end] per segment, for drawing the IDM-style bar.
    pub segments: Vec<[u64; 3]>,
    /// [done, total] stream pieces, for HLS where the byte size is unknown up front.
    pub pieces: Option<[u64; 2]>,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Completed(PathBuf),
    Paused,
}

#[derive(Clone)]
pub struct Handle {
    shared: Arc<Shared>,
    cancel: CancellationToken,
}

impl Handle {
    pub fn progress(&self) -> Progress {
        let s = &self.shared;
        let total = s.total.load(Relaxed);
        Progress {
            path: s.path.lock().unwrap().clone(),
            total: (total != UNKNOWN).then_some(total),
            received: s.received.load(Relaxed),
            connections: s.active.load(Relaxed),
            resumable: s.resumable.load(Relaxed),
            segments: s.segs.lock().unwrap().list.iter().map(|g| [g.start, g.claimed, g.end]).collect(),
            pieces: match s.pieces_total.load(Relaxed) {
                0 => None,
                n => Some([s.pieces_done.load(Relaxed), n]),
            },
        }
    }

    pub fn pause(&self) {
        self.cancel.cancel();
    }
}

struct Shared {
    path: Mutex<Option<PathBuf>>,
    total: AtomicU64,
    received: AtomicU64,
    active: AtomicUsize,
    resumable: AtomicBool,
    segs: Mutex<Segments>,
    pieces_done: AtomicU64,
    pieces_total: AtomicU64,
}

/// What we persist next to the .part file so a download survives pause/crash.
#[derive(Serialize, Deserialize)]
struct SavedState {
    total: u64,
    etag: Option<String>,
    last_modified: Option<String>,
    segments: Vec<Segment>,
}

struct Ctx {
    client: Client,
    url: String,
    headers: HeaderMap,
    file: Arc<File>,
    shared: Arc<Shared>,
    cancel: CancellationToken,
}

pub fn start(client: Client, req: Request) -> (Handle, tokio::task::JoinHandle<Result<Outcome>>) {
    let shared = Arc::new(Shared {
        path: Mutex::new(None),
        total: AtomicU64::new(UNKNOWN),
        received: AtomicU64::new(0),
        active: AtomicUsize::new(0),
        resumable: AtomicBool::new(false),
        segs: Mutex::new(Segments::new(0, MIN_SPLIT)),
        pieces_done: AtomicU64::new(0),
        pieces_total: AtomicU64::new(0),
    });
    let handle = Handle { shared: shared.clone(), cancel: CancellationToken::new() };
    let cancel = handle.cancel.clone();
    let task = tokio::spawn(async move {
        match req.kind {
            Kind::File => run(client, req, shared, cancel).await,
            Kind::Hls => hls::run(client, req, shared, cancel).await,
        }
    });
    (handle, task)
}

async fn run(client: Client, req: Request, shared: Arc<Shared>, cancel: CancellationToken) -> Result<Outcome> {
    let headers = header_map(&req.headers);
    // The probe asks for the whole file as a range: if the server honours it,
    // this response simply becomes connection #1 instead of costing a round trip.
    let mut attempt: u32 = 0;
    let first = loop {
        match client.get(&req.url).headers(headers.clone()).header(RANGE, "bytes=0-").send().await {
            Ok(r) if !r.status().is_server_error() => break r,
            Ok(r) if attempt >= MAX_RETRIES => bail!("server replied {}", r.status()),
            Err(e) if attempt >= MAX_RETRIES => return Err(e.into()),
            _ => {}
        }
        attempt += 1;
        tokio::select! {
            _ = cancel.cancelled() => return Ok(Outcome::Paused),
            _ = tokio::time::sleep(Duration::from_millis(500 * attempt as u64)) => {}
        }
    };
    let status = first.status();
    if !status.is_success() {
        bail!("server replied {status}");
    }
    let url = first.url().to_string();
    let total = match status {
        StatusCode::PARTIAL_CONTENT => content_range(first.headers()).and_then(|(_, t)| t),
        _ => None,
    };
    let etag = header_str(first.headers(), ETAG);
    let last_modified = header_str(first.headers(), LAST_MODIFIED);

    let name = match &req.file_name {
        Some(n) => names::sanitize(n),
        None => names::from_response(first.headers(), first.url()),
    };
    let state_path = req.dir.join(format!("{name}.part.json"));
    let saved = load_state(&state_path)
        .filter(|s| Some(s.total) == total && s.etag == etag && s.last_modified == last_modified);
    let name = if saved.is_some() || req.file_name.is_some() { name } else { names::unique(&req.dir, &name) };
    let dest = req.dir.join(&name);
    let part = req.dir.join(format!("{name}.part"));
    let state_path = req.dir.join(format!("{name}.part.json"));
    *shared.path.lock().unwrap() = Some(dest.clone());

    let Some(total) = total else {
        return single_stream(first, &dest, &part, &shared, &cancel).await;
    };
    shared.total.store(total, Relaxed);
    shared.resumable.store(true, Relaxed);

    let resuming = saved.is_some();
    let file = OpenOptions::new().create(true).write(true).truncate(!resuming).open(&part)?;
    let mut first = Some(first);
    {
        let mut segs = shared.segs.lock().unwrap();
        match saved {
            Some(s) => {
                *segs = Segments::restore(s.segments, MIN_SPLIT);
                first = None; // its range starts at 0, which is likely already done
            }
            None => {
                fsio::set_sparse(&file);
                file.set_len(total)?;
                *segs = Segments::new(total, MIN_SPLIT);
                segs.list[0].owned = true;
            }
        }
        let received = segs.list.iter().map(|g| g.done - g.start).sum();
        shared.received.store(received, Relaxed);
    }

    let ctx = Arc::new(Ctx { client, url, headers, file: Arc::new(file), shared: shared.clone(), cancel: cancel.clone() });
    let mut workers = JoinSet::new();
    for i in 0..req.connections.max(1) {
        let first = if i == 0 { first.take().map(|r| (0, r)) } else { None };
        workers.spawn(worker(ctx.clone(), first));
    }

    let save = || save_state(&state_path, total, &etag, &last_modified, &shared);
    let mut last_err = None;
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            joined = workers.join_next() => match joined {
                None => break,
                Some(Ok(Err(e))) => last_err = Some(e),
                Some(Err(e)) => last_err = Some(e.into()),
                Some(Ok(Ok(()))) => {}
            },
            _ = ticker.tick() => { save().ok(); }
        }
    }

    if shared.segs.lock().unwrap().finished() {
        ctx.file.sync_all()?;
        drop(ctx);
        std::fs::rename(&part, &dest).context("renaming finished download")?;
        std::fs::remove_file(&state_path).ok();
        return Ok(Outcome::Completed(dest));
    }
    save()?;
    if cancel.is_cancelled() {
        return Ok(Outcome::Paused);
    }
    Err(last_err.unwrap_or_else(|| anyhow!("download stopped before completion")))
}

/// Server without range support (or unknown size): one connection, no resume.
async fn single_stream(
    resp: Response,
    dest: &Path,
    part: &Path,
    shared: &Shared,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    use tokio::io::AsyncWriteExt;
    if let Some(len) = resp.content_length() {
        shared.total.store(len, Relaxed);
    }
    shared.received.store(0, Relaxed);
    shared.active.store(1, Relaxed);
    let mut out = tokio::io::BufWriter::with_capacity(BUF_SIZE, tokio::fs::File::create(part).await?);
    let mut stream = resp.bytes_stream();
    let result: Result<bool> = loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => break Ok(false),
            n = tokio::time::timeout(READ_TIMEOUT, stream.next()) => n,
        };
        match next {
            Err(_) => break Err(anyhow!("read timed out")),
            Ok(None) => break Ok(true),
            Ok(Some(Err(e))) => break Err(e.into()),
            Ok(Some(Ok(chunk))) => {
                out.write_all(&chunk).await?;
                shared.received.fetch_add(chunk.len() as u64, Relaxed);
            }
        }
    };
    shared.active.store(0, Relaxed);
    out.flush().await?;
    drop(out);
    if result? {
        tokio::fs::rename(part, dest).await?;
        Ok(Outcome::Completed(dest.to_path_buf()))
    } else {
        tokio::fs::remove_file(part).await.ok();
        Ok(Outcome::Paused)
    }
}

async fn worker(ctx: Arc<Ctx>, mut first: Option<(usize, Response)>) -> Result<()> {
    loop {
        let (idx, resp) = match first.take() {
            Some((i, r)) => (i, Some(r)),
            None => match ctx.shared.segs.lock().unwrap().acquire() {
                Some(i) => (i, None),
                None => return Ok(()),
            },
        };
        ctx.shared.active.fetch_add(1, Relaxed);
        let result = drive(&ctx, idx, resp).await;
        ctx.shared.active.fetch_sub(1, Relaxed);
        ctx.shared.segs.lock().unwrap().list[idx].owned = false;
        result?;
        if ctx.cancel.is_cancelled() {
            return Ok(());
        }
    }
}

/// Download one segment to its (possibly shrinking) end, retrying broken connections.
async fn drive(ctx: &Ctx, idx: usize, mut resp: Option<Response>) -> Result<()> {
    let claimed = || ctx.shared.segs.lock().unwrap().list[idx].claimed;
    let mut failures = 0;
    loop {
        let before = claimed();
        let result = fetch(ctx, idx, resp.take()).await;
        if ctx.shared.segs.lock().unwrap().list[idx].remaining() == 0 || ctx.cancel.is_cancelled() {
            return Ok(());
        }
        if claimed() > before {
            failures = 0;
        }
        failures += 1;
        if failures > MAX_RETRIES {
            return Err(result.err().unwrap_or_else(|| anyhow!("connection kept closing early")));
        }
        tokio::select! {
            _ = ctx.cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_millis(500 * failures as u64)) => {}
        }
    }
}

async fn fetch(ctx: &Ctx, idx: usize, resp: Option<Response>) -> Result<()> {
    let (from, to) = {
        let s = &ctx.shared.segs.lock().unwrap().list[idx];
        (s.claimed, s.end)
    };
    if from >= to {
        return Ok(());
    }
    let resp = match resp {
        Some(r) => r,
        None => {
            let r = ctx
                .client
                .get(&ctx.url)
                .headers(ctx.headers.clone())
                .header(RANGE, format!("bytes={from}-{}", to - 1))
                .send()
                .await?;
            if r.status() != StatusCode::PARTIAL_CONTENT {
                bail!("range request got {}", r.status());
            }
            if content_range(r.headers()).map(|(start, _)| start) != Some(from) {
                bail!("server returned the wrong range");
            }
            r
        }
    };

    let mut stream = resp.bytes_stream();
    let mut buf = Vec::with_capacity(BUF_SIZE);
    let mut buf_at = from;
    let result = loop {
        let next = tokio::select! {
            _ = ctx.cancel.cancelled() => break Ok(()),
            n = tokio::time::timeout(READ_TIMEOUT, stream.next()) => n,
        };
        let chunk = match next {
            Err(_) => break Err(anyhow!("read timed out")),
            Ok(None) => break Ok(()),
            Ok(Some(Err(e))) => break Err(e.into()),
            Ok(Some(Ok(c))) => c,
        };
        // Claim under the lock so a concurrent split never hands out bytes we already have.
        let (take, reached_end) = {
            let mut segs = ctx.shared.segs.lock().unwrap();
            let s = &mut segs.list[idx];
            let take = (chunk.len() as u64).min(s.remaining()) as usize;
            s.claimed += take as u64;
            (take, s.remaining() == 0)
        };
        ctx.shared.received.fetch_add(take as u64, Relaxed);
        buf.extend_from_slice(&chunk[..take]);
        if buf.len() >= BUF_SIZE {
            buf_at = flush(ctx, idx, &mut buf, buf_at).await?;
        }
        if reached_end {
            break Ok(());
        }
    };
    flush(ctx, idx, &mut buf, buf_at).await?;
    result
}

async fn flush(ctx: &Ctx, idx: usize, buf: &mut Vec<u8>, at: u64) -> Result<u64> {
    if buf.is_empty() {
        return Ok(at);
    }
    let data = std::mem::take(buf);
    let file = ctx.file.clone();
    let mut data = tokio::task::spawn_blocking(move || fsio::write_at(&file, &data, at).map(|_| data)).await??;
    let end = at + data.len() as u64;
    ctx.shared.segs.lock().unwrap().list[idx].done = end;
    data.clear();
    *buf = data;
    Ok(end)
}

fn header_map(pairs: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in pairs {
        if k.eq_ignore_ascii_case("range") || k.eq_ignore_ascii_case("accept-encoding") {
            continue;
        }
        if let (Ok(k), Ok(v)) = (HeaderName::try_from(k.as_str()), HeaderValue::try_from(v.as_str())) {
            map.insert(k, v);
        }
    }
    map
}

fn header_str(h: &HeaderMap, name: HeaderName) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

/// "bytes 100-199/1000" -> (100, Some(1000)); total is None for "/*".
fn content_range(h: &HeaderMap) -> Option<(u64, Option<u64>)> {
    let v = h.get(CONTENT_RANGE)?.to_str().ok()?.strip_prefix("bytes ")?;
    let (range, total) = v.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    Some((start, total.trim().parse().ok()))
}

fn load_state(path: &Path) -> Option<SavedState> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn save_state(
    path: &Path,
    total: u64,
    etag: &Option<String>,
    last_modified: &Option<String>,
    shared: &Shared,
) -> std::io::Result<()> {
    let segments = shared.segs.lock().unwrap().list.clone();
    let state = SavedState { total, etag: etag.clone(), last_modified: last_modified.clone(), segments };
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&state)?)?;
    std::fs::rename(tmp, path)
}
