//! HLS (.m3u8) streams: pick the best variant, fetch segments in parallel, write them in order.

use crate::{header_map, names, Outcome, Request, Shared, MAX_RETRIES, READ_TIMEOUT};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::HeaderMap;
use reqwest::{Client, Url};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq)]
struct Key {
    uri: Url,
    iv: Option<[u8; 16]>,
}

#[derive(Debug, PartialEq)]
struct Seg {
    url: Url,
    key: Option<Key>,
    seq: u64,
}

#[derive(Debug, Default)]
struct Playlist {
    init: Option<Url>,
    segments: Vec<Seg>,
}

pub(crate) async fn run(client: Client, req: Request, shared: Arc<Shared>, cancel: CancellationToken) -> Result<Outcome> {
    let headers = header_map(&req.headers);
    let mut url = Url::parse(&req.url)?;
    let mut text = get_text(&client, &url, &headers).await?;
    if text.contains("#EXT-X-STREAM-INF") {
        url = best_variant(&text, &url).context("master playlist has no variants")?;
        text = get_text(&client, &url, &headers).await?;
    }
    let playlist = parse_media(&text, &url)?;
    if playlist.segments.is_empty() {
        bail!("playlist has no segments");
    }

    let mut name = names::sanitize(req.file_name.as_deref().unwrap_or("stream.ts"));
    if playlist.init.is_some() && !name.ends_with(".mp4") {
        // fMP4 segments: init + fragments concatenate into a playable MP4.
        name = format!("{}.mp4", name.trim_end_matches(".ts"));
    }
    let name = names::unique(&req.dir, &name);
    let dest = req.dir.join(&name);
    let part = req.dir.join(format!("{name}.part"));
    *shared.path.lock().unwrap() = Some(dest.clone());
    shared.pieces_total.store(playlist.segments.len() as u64, Relaxed);

    let mut keys = HashMap::new();
    for k in playlist.segments.iter().filter_map(|s| s.key.as_ref()) {
        if !keys.contains_key(&k.uri) {
            let bytes = get_bytes(&client, &k.uri, &headers).await?;
            let key: [u8; 16] = bytes.as_slice().try_into().map_err(|_| anyhow!("AES key is not 16 bytes"))?;
            keys.insert(k.uri.clone(), key);
        }
    }

    let mut out = tokio::io::BufWriter::with_capacity(1 << 20, tokio::fs::File::create(&part).await?);
    if let Some(init) = &playlist.init {
        out.write_all(&get_bytes(&client, init, &headers).await?).await?;
    }

    let keys = Arc::new(keys);
    let fetches = playlist.segments.into_iter().map(|seg| {
        let (client, headers, keys) = (client.clone(), headers.clone(), keys.clone());
        async move {
            let data = get_bytes(&client, &seg.url, &headers).await?;
            match &seg.key {
                None => Ok(data),
                Some(k) => decrypt(&data, &keys[&k.uri], k.iv.unwrap_or_else(|| seq_iv(seg.seq))),
            }
        }
    });
    // `buffered` keeps N requests in flight but yields results in playlist order.
    let mut stream = futures_util::stream::iter(fetches).buffered(req.connections.max(1));
    shared.active.store(req.connections.max(1), Relaxed);
    let result: Result<bool> = loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => break Ok(false),
            n = stream.next() => n,
        };
        match next {
            None => break Ok(true),
            Some(Err(e)) => break Err(e),
            Some(Ok(data)) => {
                out.write_all(&data).await?;
                shared.received.fetch_add(data.len() as u64, Relaxed);
                shared.pieces_done.fetch_add(1, Relaxed);
            }
        }
    };
    shared.active.store(0, Relaxed);
    out.flush().await?;
    drop(out);
    if !result? {
        tokio::fs::remove_file(&part).await.ok();
        return Ok(Outcome::Paused);
    }
    tokio::fs::rename(&part, &dest).await?;
    let dest = match tokio::task::spawn_blocking({
        let dest = dest.clone();
        move || remux(&dest)
    })
    .await?
    {
        Some(mp4) => mp4,
        None => dest,
    };
    *shared.path.lock().unwrap() = Some(dest.clone());
    Ok(Outcome::Completed(dest))
}

async fn get_bytes(client: &Client, url: &Url, headers: &HeaderMap) -> Result<Vec<u8>> {
    let mut attempt = 0;
    loop {
        let r = async {
            let resp = client.get(url.clone()).headers(headers.clone()).send().await?.error_for_status()?;
            let mut body = Vec::with_capacity(resp.content_length().unwrap_or(0) as usize);
            let mut stream = resp.bytes_stream();
            // Idle timeout, not a total one: big segments on slow links are fine as long as bytes flow.
            while let Some(chunk) = tokio::time::timeout(READ_TIMEOUT, stream.next()).await? {
                body.extend_from_slice(&chunk?);
            }
            Ok::<_, anyhow::Error>(body)
        }
        .await;
        match r {
            Ok(b) => return Ok(b),
            Err(e) if attempt >= MAX_RETRIES => return Err(e.context(format!("fetching {url}"))),
            Err(_) => attempt += 1,
        }
        tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
    }
}

async fn get_text(client: &Client, url: &Url, headers: &HeaderMap) -> Result<String> {
    let text = String::from_utf8(get_bytes(client, url, headers).await?)?;
    if !text.trim_start().starts_with("#EXTM3U") {
        bail!("not an HLS playlist");
    }
    Ok(text)
}

fn best_variant(text: &str, base: &Url) -> Option<Url> {
    let mut best: Option<(u64, &str)> = None;
    let mut pending: Option<u64> = None;
    for line in text.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(attrs(v).get("BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0));
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(bw) = pending.take() {
                if best.map_or(true, |(b, _)| bw > b) {
                    best = Some((bw, line));
                }
            }
        }
    }
    base.join(best?.1).ok()
}

fn parse_media(text: &str, base: &Url) -> Result<Playlist> {
    let mut p = Playlist::default();
    let mut seq = 0;
    let mut key = None;
    for line in text.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            seq = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("#EXT-X-KEY:") {
            let a = attrs(v);
            key = match a.get("METHOD").map(String::as_str) {
                None | Some("NONE") => None,
                Some("AES-128") => Some(Key {
                    uri: base.join(a.get("URI").context("key without URI")?)?,
                    iv: a.get("IV").map(|iv| parse_iv(iv)).transpose()?,
                }),
                Some(m) => bail!("{m} encrypted streams (usually DRM) are not supported"),
            };
        } else if let Some(v) = line.strip_prefix("#EXT-X-MAP:") {
            p.init = Some(base.join(attrs(v).get("URI").context("map without URI")?)?);
        } else if !line.is_empty() && !line.starts_with('#') {
            p.segments.push(Seg { url: base.join(line)?, key: key.clone(), seq });
            seq += 1;
        }
    }
    Ok(p)
}

/// `A=1,B="x,y",C=z` -> map; quoted values may contain commas.
fn attrs(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = s;
    while let Some((k, v)) = rest.split_once('=') {
        let (val, next) = if let Some(q) = v.strip_prefix('"') {
            let end = q.find('"').unwrap_or(q.len());
            (&q[..end], q[end..].trim_start_matches('"').trim_start_matches(','))
        } else {
            v.split_once(',').unwrap_or((v, ""))
        };
        out.insert(k.trim().to_string(), val.to_string());
        rest = next;
    }
    out
}

fn parse_iv(s: &str) -> Result<[u8; 16]> {
    let hex = s.trim_start_matches("0x").trim_start_matches("0X");
    let n = u128::from_str_radix(hex, 16).context("bad IV")?;
    Ok(n.to_be_bytes())
}

/// Without an explicit IV, HLS uses the media sequence number.
fn seq_iv(seq: u64) -> [u8; 16] {
    (seq as u128).to_be_bytes()
}

fn decrypt(data: &[u8], key: &[u8; 16], iv: [u8; 16]) -> Result<Vec<u8>> {
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    cbc::Decryptor::<aes::Aes128>::new(key.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| anyhow!("segment decryption failed (wrong key?)"))
}

/// .ts -> .mp4 without re-encoding, when ffmpeg is on PATH. Keeps the .ts if anything fails.
fn remux(ts: &Path) -> Option<PathBuf> {
    if ts.extension()? != "ts" {
        return None;
    }
    let mp4 = ts.with_extension("mp4");
    if mp4.exists() {
        return None;
    }
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-i"]).arg(ts).args(["-c", "copy"]).arg(&mp4).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    if cmd.status().ok()?.success() {
        std::fs::remove_file(ts).ok();
        Some(mp4)
    } else {
        std::fs::remove_file(&mp4).ok();
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_highest_bandwidth() {
        let base = Url::parse("https://cdn.example/v/master.m3u8").unwrap();
        let m = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/i.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=5000000,CODECS=\"avc1.64001f,mp4a.40.2\"\nhigh/i.m3u8\n";
        assert_eq!(best_variant(m, &base).unwrap().as_str(), "https://cdn.example/v/high/i.m3u8");
    }

    #[test]
    fn parses_keys_and_sequence() {
        let base = Url::parse("https://cdn.example/v/i.m3u8").unwrap();
        let m = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:7\n#EXTINF:4,\na.ts\n\
                 #EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\",IV=0x000102030405060708090a0b0c0d0e0f\n#EXTINF:4,\nb.ts\n";
        let p = parse_media(m, &base).unwrap();
        assert_eq!(p.segments.len(), 2);
        assert_eq!(p.segments[0].key, None);
        assert_eq!(p.segments[1].seq, 8);
        let k = p.segments[1].key.as_ref().unwrap();
        assert_eq!(k.uri.as_str(), "https://cdn.example/v/k.bin");
        assert_eq!(k.iv.unwrap()[15], 0x0f);
    }

    #[test]
    fn rejects_drm() {
        let base = Url::parse("https://cdn.example/i.m3u8").unwrap();
        assert!(parse_media("#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\na.ts\n", &base).is_err());
    }
}
