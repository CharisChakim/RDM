//! Video pages (YouTube, Vimeo, Instagram, X...) resolved to direct media URLs with yt-dlp.
//! yt-dlp only finds the URLs; the RDM engine downloads them multi-connection and ffmpeg joins
//! the tracks, the same split IDM uses (its extension finds streams, its engine downloads).

use rdm_engine::{names, Kind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// One downloadable quality of a page.
#[derive(Serialize, Clone, Debug)]
pub struct Choice {
    pub label: String,
    pub url: String,
    pub audio_url: Option<String>,
    pub kind: Kind,
    pub headers: Vec<(String, String)>,
    pub file_name: String,
    pub size: Option<u64>,
}

#[derive(Deserialize, Default, Clone)]
struct Format {
    #[serde(default)]
    format_id: String,
    url: Option<String>,
    ext: Option<String>,
    protocol: Option<String>,
    vcodec: Option<String>,
    acodec: Option<String>,
    height: Option<u32>,
    fps: Option<f64>,
    tbr: Option<f64>,
    abr: Option<f64>,
    audio_channels: Option<u32>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
    #[serde(default)]
    http_headers: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Info {
    title: Option<String>,
    #[serde(default)]
    formats: Vec<Format>,
    /// Single-format pages put the format's fields at the top level.
    #[serde(flatten)]
    own: Format,
}

pub async fn resolve(page: &str, cookie: Option<&str>) -> Result<Vec<Choice>, String> {
    let mut cmd = tokio::process::Command::new(executable());
    cmd.args(["-J", "--no-warnings", "--no-playlist", "--js-runtimes", "node"]);
    if let Some(c) = cookie {
        cmd.arg("--add-header").arg(format!("Cookie:{c}"));
    }
    cmd.arg("--").arg(page);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(Duration::from_secs(90), cmd.output())
        .await
        .map_err(|_| "yt-dlp took too long".to_string())?
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "yt-dlp is not installed (winget install yt-dlp.yt-dlp)".to_string(),
            _ => e.to_string(),
        })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("yt-dlp failed");
        return Err(last.trim_start_matches("ERROR: ").to_string());
    }
    let info: Info = serde_json::from_slice(&out.stdout).map_err(|e| format!("unexpected yt-dlp output: {e}"))?;
    let choices = choices(info);
    if choices.is_empty() {
        return Err("no downloadable formats (DRM-protected or unsupported)".into());
    }
    Ok(choices)
}

/// PATH first; winget's links folder covers apps started before yt-dlp was installed.
fn executable() -> std::path::PathBuf {
    let winget = std::env::var_os("LOCALAPPDATA")
        .map(|d| std::path::Path::new(&d).join("Microsoft").join("WinGet").join("Links").join("yt-dlp.exe"));
    match winget {
        Some(p) if p.exists() => p,
        _ => "yt-dlp".into(),
    }
}

fn has(codec: &Option<String>) -> bool {
    codec.as_deref().is_some_and(|c| c != "none")
}

fn is_hls(f: &Format) -> bool {
    f.protocol.as_deref().is_some_and(|p| p.starts_with("m3u8"))
}

fn size(f: &Format) -> Option<u64> {
    f.filesize.or(f.filesize_approx)
}

/// Lower is better: widely playable codecs first.
fn codec_rank(f: &Format) -> u8 {
    match f.vcodec.as_deref().unwrap_or("") {
        c if c.starts_with("avc1") || c.starts_with("h264") => 0,
        c if c.starts_with("av01") => 1,
        c if c.starts_with("vp9") || c.starts_with("vp09") => 2,
        _ => 3,
    }
}

fn codec_name(f: &Format) -> &'static str {
    match codec_rank(f) {
        0 => "H.264",
        1 => "AV1",
        2 => "VP9",
        _ => "video",
    }
}

fn human(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
    } else {
        format!("{} MB", (bytes as f64 / (1u64 << 20) as f64).round())
    }
}

/// One choice per resolution (best playable format), paired with the best matching audio track,
/// plus an audio-only choice.
fn choices(info: Info) -> Vec<Choice> {
    let title = names::sanitize(info.title.as_deref().unwrap_or("video"));
    let mut formats = info.formats;
    if formats.is_empty() {
        formats.push(info.own);
    }
    let usable: Vec<&Format> = formats
        .iter()
        .filter(|f| {
            f.url.is_some() && matches!(f.protocol.as_deref(), Some("https" | "http" | "m3u8" | "m3u8_native") | None)
        })
        .collect();
    let audios: Vec<&Format> = usable.iter().copied().filter(|f| !has(&f.vcodec) && has(&f.acodec)).collect();
    // Same delivery as the video (HLS with HLS), m4a for mp4, stereo over 5.1 (plays everywhere,
    // and it's what the player uses by default), original over dynamic-range-compressed.
    let best_audio = |hls: bool, want_m4a: bool| -> Option<&Format> {
        audios.iter().copied().filter(|a| is_hls(a) == hls).max_by(|a, b| {
            let key = |f: &Format| {
                (
                    (f.ext.as_deref() == Some("m4a")) == want_m4a,
                    f.audio_channels.is_none_or(|c| c <= 2),
                    !f.format_id.contains("drc"),
                    f.abr.or(f.tbr).unwrap_or(0.0),
                )
            };
            key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
        })
    };

    let mut by_height: Vec<&Format> = Vec::new();
    // Codec info can be missing (simple sites, or codecs yt-dlp doesn't know): then go by extension,
    // so an unknown audio-only m4a isn't offered as "video".
    let video_ext = |f: &Format| matches!(f.ext.as_deref(), Some("mp4" | "webm" | "mkv" | "mov" | "flv" | "3gp"));
    for f in usable.iter().copied().filter(|f| has(&f.vcodec) || (f.vcodec.is_none() && f.acodec.is_none() && video_ext(f))) {
        let better = |a: &Format, b: &Format| {
            // direct file over HLS, then codec compatibility, then bitrate
            (is_hls(a), codec_rank(a), -(a.tbr.unwrap_or(0.0) as i64))
                < (is_hls(b), codec_rank(b), -(b.tbr.unwrap_or(0.0) as i64))
        };
        match by_height.iter_mut().find(|g| g.height == f.height) {
            Some(slot) if better(f, slot) => *slot = f,
            Some(_) => {}
            None => by_height.push(f),
        }
    }
    by_height.sort_by(|a, b| b.height.cmp(&a.height));

    let mut out = Vec::new();
    for v in by_height {
        let video_ext = v.ext.clone().unwrap_or_else(|| "mp4".into());
        let audio = if has(&v.acodec) { None } else { best_audio(is_hls(v), video_ext == "mp4") };
        let ext = match (audio.and_then(|a| a.ext.as_deref()), video_ext.as_str()) {
            (None, e) => e.to_string(),
            (Some("m4a" | "mp4"), "mp4") => "mp4".into(),
            (Some("webm"), "webm") => "webm".into(),
            _ => "mkv".into(),
        };
        let total = size(v).zip(audio.map_or(Some(0), size)).map(|(a, b)| a + b);
        let mut label = match v.height {
            Some(h) if v.fps.unwrap_or(0.0) > 30.5 => format!("{h}p{}", v.fps.unwrap_or(0.0).round()),
            Some(h) => format!("{h}p"),
            None => "Video".into(),
        };
        label.push_str(&format!(" · {}", codec_name(v)));
        if let Some(t) = total {
            label.push_str(&format!(" · {}", human(t)));
        }
        out.push(Choice {
            label,
            url: v.url.clone().unwrap_or_default(),
            audio_url: audio.and_then(|a| a.url.clone()),
            kind: if is_hls(v) { Kind::Hls } else { Kind::File },
            headers: v.http_headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            file_name: format!("{title}.{}", if is_hls(v) { "ts" } else { ext.as_str() }),
            size: total,
        });
    }
    if let Some(a) = best_audio(false, true) {
        let ext = a.ext.clone().unwrap_or_else(|| "m4a".into());
        out.push(Choice {
            label: format!("Audio only · {}{}", ext.to_uppercase(), size(a).map(|s| format!(" · {}", human(s))).unwrap_or_default()),
            url: a.url.clone().unwrap_or_default(),
            audio_url: None,
            kind: Kind::File,
            headers: a.http_headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            file_name: format!("{title}.{ext}"),
            size: size(a),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(id: &str, ext: &str, v: &str, a: &str, h: Option<u32>, tbr: f64, proto: &str) -> Format {
        Format {
            format_id: id.into(),
            url: Some(format!("https://cdn/{id}")),
            ext: Some(ext.into()),
            protocol: Some(proto.into()),
            vcodec: Some(v.into()),
            acodec: Some(a.into()),
            height: h,
            tbr: Some(tbr),
            abr: (v == "none").then_some(tbr),
            filesize: Some(10 << 20),
            ..Default::default()
        }
    }

    #[test]
    fn pairs_best_video_per_height_with_matching_audio() {
        let info = Info {
            title: Some("My: Clip".into()),
            formats: vec![
                f("140", "m4a", "none", "mp4a.40.2", None, 129.0, "https"),
                f("251", "webm", "none", "opus", None, 140.0, "https"),
                f("140-drc", "m4a", "none", "mp4a.40.2", None, 130.0, "https"),
                Format { audio_channels: Some(6), ..f("258", "m4a", "none", "mp4a.40.2", None, 387.0, "https") },
                Format { vcodec: None, acodec: None, ..f("773", "m4a", "", "", None, 336.0, "https") },
                f("299", "mp4", "avc1.64002A", "none", Some(1080), 4000.0, "https"),
                f("303", "webm", "vp9", "none", Some(1080), 3000.0, "https"),
                f("96", "mp4", "avc1", "mp4a", Some(1080), 5000.0, "m3u8_native"),
                f("18", "mp4", "avc1.42001E", "mp4a.40.2", Some(360), 500.0, "https"),
            ],
            own: Format::default(),
        };
        let c = choices(info);
        let labels: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["1080p · H.264 · 20 MB", "360p · H.264 · 10 MB", "Audio only · M4A · 10 MB"]);
        assert_eq!(c[0].url, "https://cdn/299");
        assert_eq!(c[0].audio_url.as_deref(), Some("https://cdn/140"), "stereo m4a, not 5.1 or the -drc copy");
        assert_eq!(c[0].file_name, "My_ Clip.mp4");
        assert_eq!(c[1].audio_url, None, "format 18 already has sound");
    }

    #[test]
    fn single_format_pages_still_give_a_choice() {
        let mut own = f("0", "mp4", "avc1", "mp4a", Some(720), 1000.0, "https");
        own.format_id = String::new();
        let c = choices(Info { title: Some("x".into()), formats: vec![], own });
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, Kind::File);
    }
}
