//! HLS (.m3u8) streams: pick a variant (and its separate audio track, if any),
//! fetch segments in parallel, write them in order, mux with ffmpeg.

use crate::{header_map, names, Outcome, Request, Shared, MAX_RETRIES, READ_TIMEOUT};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::HeaderMap;
use reqwest::{Client, Url};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
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

/// One quality level of a master playlist, as offered to the user.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Variant {
    /// Media playlist of this quality.
    pub url: String,
    /// Separate audio media playlist, when the video variant carries no sound of its own.
    pub audio_url: Option<String>,
    pub bandwidth: u64,
    pub resolution: Option<String>,
    /// e.g. "720p · 2.1 Mbps"
    pub label: String,
}

/// Qualities offered by a playlist, best first. A media playlist yields one "Default" entry.
pub async fn variants(client: &Client, url: &str, headers: &[(String, String)]) -> Result<Vec<Variant>> {
    let base = Url::parse(url)?;
    let text = get_text(client, &base, &header_map(headers)).await?;
    if is_master(&text) {
        return Ok(parse_master(&text, &base));
    }
    Ok(vec![Variant { url: url.into(), audio_url: None, bandwidth: 0, resolution: None, label: "Default".into() }])
}

pub(crate) async fn run(client: Client, req: Request, shared: Arc<Shared>, cancel: CancellationToken) -> Result<Outcome> {
    let headers = header_map(&req.headers);
    let url = Url::parse(&req.url)?;
    let text = get_text(&client, &url, &headers).await?;
    // Given a master (no choice made), take the best quality; given a media playlist, use it as chosen.
    let (video_url, video_text, audio_url) = if is_master(&text) {
        let best = parse_master(&text, &url).into_iter().next().context("master playlist has no variants")?;
        let v = Url::parse(&best.url)?;
        let t = get_text(&client, &v, &headers).await?;
        (v, t, best.audio_url)
    } else {
        (url, text, req.audio_url.clone())
    };
    let video = parse_media(&video_text, &video_url)?;
    if video.segments.is_empty() {
        bail!("playlist has no segments");
    }
    let audio = match audio_url {
        Some(a) => {
            let a = Url::parse(&a)?;
            Some(parse_media(&get_text(&client, &a, &headers).await?, &a)?)
        }
        None => None,
    };

    let stem = names::sanitize(req.file_name.as_deref().unwrap_or("stream"));
    let stem = stem.strip_suffix(".ts").or_else(|| stem.strip_suffix(".mp4")).unwrap_or(&stem).to_string();
    // fMP4 (init + fragments) concatenates into an MP4; a separate audio track gets muxed into one.
    let ext = if video.init.is_some() || audio.is_some() { "mp4" } else { "ts" };
    let name = names::unique(&req.dir, &format!("{stem}.{ext}"));
    let dest = req.dir.join(&name);
    let video_part = req.dir.join(format!("{name}.part"));
    let audio_part = req.dir.join(format!("{name}.audio.part"));
    *shared.path.lock().unwrap() = Some(dest.clone());
    let pieces = video.segments.len() + audio.as_ref().map_or(0, |a| a.segments.len());
    shared.pieces_total.store(pieces as u64, Relaxed);

    let connections = req.connections.max(1);
    shared.active.store(connections, Relaxed);
    let video_init = video.init.is_some();
    let mut result = fetch_stream(&client, &headers, video, &video_part, &shared, &cancel, connections).await;
    let audio_init = audio.as_ref().map(|a| a.init.is_some());
    if let (Ok(true), Some(audio)) = (&result, audio) {
        result = fetch_stream(&client, &headers, audio, &audio_part, &shared, &cancel, connections).await;
    }
    shared.active.store(0, Relaxed);
    if !matches!(result, Ok(true)) {
        // No resume for streams yet, so partial pieces are useless.
        tokio::fs::remove_file(&video_part).await.ok();
        tokio::fs::remove_file(&audio_part).await.ok();
        return result.map(|_| Outcome::Paused);
    }

    let dest = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        let Some(audio_init) = audio_init else {
            std::fs::rename(&video_part, &dest)?;
            return Ok(remux(&dest).unwrap_or(dest));
        };
        if mux(&video_part, &audio_part, &dest) {
            std::fs::remove_file(&video_part).ok();
            std::fs::remove_file(&audio_part).ok();
            return Ok(dest);
        }
        // No ffmpeg: keep both tracks so nothing downloaded is lost.
        let video_dest = dest.with_extension(if video_init { "video.mp4" } else { "video.ts" });
        std::fs::rename(&video_part, &video_dest)?;
        std::fs::rename(&audio_part, dest.with_extension(if audio_init { "audio.m4a" } else { "audio.ts" }))?;
        Ok(video_dest)
    })
    .await??;
    *shared.path.lock().unwrap() = Some(dest.clone());
    Ok(Outcome::Completed(dest))
}

/// Downloads one media playlist into `part`. Ok(false) means cancelled.
async fn fetch_stream(
    client: &Client,
    headers: &HeaderMap,
    playlist: Playlist,
    part: &Path,
    shared: &Shared,
    cancel: &CancellationToken,
    connections: usize,
) -> Result<bool> {
    let mut keys = HashMap::new();
    for k in playlist.segments.iter().filter_map(|s| s.key.as_ref()) {
        if !keys.contains_key(&k.uri) {
            let bytes = get_bytes(client, &k.uri, headers).await?;
            let key: [u8; 16] = bytes.as_slice().try_into().map_err(|_| anyhow!("AES key is not 16 bytes"))?;
            keys.insert(k.uri.clone(), key);
        }
    }

    let mut out = tokio::io::BufWriter::with_capacity(1 << 20, tokio::fs::File::create(part).await?);
    if let Some(init) = &playlist.init {
        out.write_all(&get_bytes(client, init, headers).await?).await?;
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
    let mut stream = futures_util::stream::iter(fetches).buffered(connections);
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
    out.flush().await?;
    result
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

fn is_master(text: &str) -> bool {
    text.contains("#EXT-X-STREAM-INF")
}

/// Variants best first, each paired with its audio group's DEFAULT (or first) separate track.
fn parse_master(text: &str, base: &Url) -> Vec<Variant> {
    let mut audio: HashMap<String, Vec<(bool, Url)>> = HashMap::new();
    let mut streams = Vec::new();
    let mut pending = None;
    for line in text.lines().map(str::trim) {
        if let Some(v) = line.strip_prefix("#EXT-X-MEDIA:") {
            let a = attrs(v);
            // Renditions without a URI are already muxed into the variant.
            if let (Some("AUDIO"), Some(group), Some(Ok(uri))) =
                (a.get("TYPE").map(String::as_str), a.get("GROUP-ID"), a.get("URI").map(|u| base.join(u)))
            {
                let default = a.get("DEFAULT").is_some_and(|d| d == "YES");
                audio.entry(group.clone()).or_default().push((default, uri));
            }
        } else if let Some(v) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(attrs(v));
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(a) = pending.take() {
                streams.push((a, line));
            }
        }
    }
    let mut out: Vec<(bool, Variant)> = streams
        .into_iter()
        .filter_map(|(a, uri)| {
            let aac = a.get("CODECS").is_some_and(|c| c.contains("mp4a"));
            let bandwidth = a.get("BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0);
            let resolution = a.get("RESOLUTION").cloned();
            let audio_url = a
                .get("AUDIO")
                .and_then(|g| audio.get(g))
                .and_then(|tracks| tracks.iter().find(|t| t.0).or(tracks.first()))
                .map(|t| t.1.to_string());
            let rate = if bandwidth >= 1_000_000 {
                format!("{:.1} Mbps", bandwidth as f64 / 1e6)
            } else {
                format!("{} kbps", bandwidth / 1000)
            };
            let label = match resolution.as_deref().and_then(|r| r.split_once('x')) {
                Some((_, h)) => format!("{h}p · {rate}"),
                None => rate,
            };
            Some((aac, Variant { url: base.join(uri).ok()?.to_string(), audio_url, bandwidth, resolution, label }))
        })
        .collect();
    // The same video is often listed once per audio codec (AAC, AC-3, E-AC-3...). Keep one entry,
    // preferring AAC: it plays everywhere, AC-3/E-AC-3 often don't.
    out.sort_by(|a, b| a.1.url.cmp(&b.1.url).then(b.0.cmp(&a.0)));
    out.dedup_by(|a, b| a.1.url == b.1.url);
    let mut out: Vec<Variant> = out.into_iter().map(|(_, v)| v).collect();
    out.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));
    out
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

/// Runs ffmpeg (stream copy only, never re-encodes). False if it is missing or fails.
fn ffmpeg(args: impl FnOnce(&mut Command) -> &mut Command) -> bool {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y"]).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    args(&mut cmd).status().is_ok_and(|s| s.success())
}

/// .ts -> .mp4 when ffmpeg is on PATH. Keeps the .ts if anything fails.
fn remux(ts: &Path) -> Option<PathBuf> {
    if ts.extension()? != "ts" {
        return None;
    }
    let mp4 = ts.with_extension("mp4");
    if mp4.exists() {
        return None;
    }
    if ffmpeg(|c| c.arg("-i").arg(ts).args(["-c", "copy"]).arg(&mp4)) {
        std::fs::remove_file(ts).ok();
        Some(mp4)
    } else {
        std::fs::remove_file(&mp4).ok();
        None
    }
}

/// Video track + separate audio track -> one file.
fn mux(video: &Path, audio: &Path, out: &Path) -> bool {
    let ok = ffmpeg(|c| {
        c.arg("-i").arg(video).arg("-i").arg(audio).args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"]).arg(out)
    });
    if !ok {
        std::fs::remove_file(out).ok();
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_variants_best_first() {
        let base = Url::parse("https://cdn.example/v/master.m3u8").unwrap();
        let m = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/i.m3u8\n\
                 #EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=90000,URI=\"iframe.m3u8\"\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=5000000,CODECS=\"avc1.64001f,mp4a.40.2\",RESOLUTION=1920x1080\nhigh/i.m3u8\n";
        let v = parse_master(m, &base);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].url, "https://cdn.example/v/high/i.m3u8");
        assert_eq!(v[0].label, "1080p · 5.0 Mbps");
        assert_eq!(v[1].label, "360p · 800 kbps");
        assert_eq!(v[0].audio_url, None);
    }

    #[test]
    fn pairs_variants_with_separate_audio() {
        let base = Url::parse("https://cdn.example/v/master.m3u8").unwrap();
        let m = "#EXTM3U\n\
                 #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Deutsch\",LANGUAGE=\"de\",URI=\"a/de.m3u8\"\n\
                 #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",DEFAULT=YES,URI=\"a/en.m3u8\"\n\
                 #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"muxed\",NAME=\"main\",DEFAULT=YES\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720,AUDIO=\"aud\"\nv720.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x360,AUDIO=\"muxed\"\nv360.m3u8\n";
        let v = parse_master(m, &base);
        assert_eq!(v[0].audio_url.as_deref(), Some("https://cdn.example/v/a/en.m3u8"));
        assert_eq!(v[1].audio_url, None);
    }

    #[test]
    fn prefers_aac_when_a_video_is_listed_per_audio_codec() {
        let base = Url::parse("https://cdn.example/m.m3u8").unwrap();
        let m = "#EXTM3U\n\
                 #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"en\",DEFAULT=YES,URI=\"a1.m3u8\"\n\
                 #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"ac3\",NAME=\"en\",DEFAULT=YES,URI=\"a2.m3u8\"\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=553010,CODECS=\"avc1.640015,mp4a.40.2\",RESOLUTION=480x270,AUDIO=\"aac\"\nv2.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=774387,CODECS=\"avc1.640015,ac-3\",RESOLUTION=480x270,AUDIO=\"ac3\"\nv2.m3u8\n";
        let v = parse_master(m, &base);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].audio_url.as_deref(), Some("https://cdn.example/a1.m3u8"));
        assert_eq!(v[0].label, "270p · 553 kbps");
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
