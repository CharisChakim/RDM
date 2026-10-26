# RDM: Rust Download Manager

A personal IDM-style download manager: a multi-connection Rust engine, a native desktop UI, and a Chrome/Edge extension that takes over browser downloads and detects videos.

```
engine/      download engine (library) + tests
core/        download list, persistence, localhost API for the extension, yt-dlp resolver
desktop/     native Windows app: Slint UI (software renderer, no browser engine), tray, autostart
cli/         `rdm` command-line front end, used for benchmarks
extension/   Chrome/Edge MV3 extension
installer/   NSIS script for the per-user installer
bench/       IDM vs RDM benchmark script + local test server (Range, per-connection throttle)
```

## How the engine works

- **Probe without an extra round trip.** The first request asks for `Range: bytes=0-`. If the server answers 206, that response becomes connection #1.
- **Dynamic segmentation, like IDM.** When a connection becomes idle, it takes the segment with the most bytes left, splits it in half and downloads the second half. The original connection stops at the new midpoint. Connections keep doing this until no remaining piece is worth splitting (256 KiB minimum). All connections open at once; IDM opens 4 and adds the rest about 1.5 s later. If a server answers 429/503 to an extra connection, that connection gives up and the others take over its part.
- **No "assembling" phase.** IDM writes parts to temporary files and merges them at the end. RDM writes every segment directly into one sparse `.part` file using positioned writes, then renames it when done.
- **Resume.** Segment state is saved to `<name>.part.json` every second. On resume, the file size, ETag and Last-Modified must still match.
- **HLS (`.m3u8`).** RDM lists every quality in a master playlist. The app's Add URL dialog and the extension's video menu let you pick one, and the default is the best quality. When a quality has a separate audio track (`EXT-X-MEDIA TYPE=AUDIO`), RDM downloads that track too, preferring AAC when a video is listed once per audio codec. Segments download in parallel and are written in order. RDM handles AES-128 encryption and fMP4 (`EXT-X-MAP`). If `ffmpeg` is on PATH, the result is remuxed or muxed to `.mp4` without re-encoding. Without ffmpeg, video and audio are kept as separate files.
- **Video pages: YouTube, Vimeo, Instagram, X, TikTok and others.** These sites don't expose plain media URLs, so RDM asks [yt-dlp](https://github.com/yt-dlp/yt-dlp) to resolve the page into stream URLs. It offers one choice per resolution, preferring H.264 and stereo AAC for compatibility, plus an audio-only option. The RDM engine then downloads video and audio at the same time, each multi-connection and resumable, and ffmpeg joins them. This is the same split IDM uses: something finds the streams, and the engine downloads them. Install yt-dlp with `winget install yt-dlp.yt-dlp`. For YouTube it uses Node as its JavaScript runtime.

## Run

```bash
cargo test -p rdm-engine
```

```bash
cargo run --release -p rdm-cli -- https://example.com/file.zip -c 8 -d .
```

```bash
cargo run --release -p rdm-desktop
```

To build the per-user installer (`target/release/RDM_<version>_x64-setup.exe`), build the app and then run NSIS (`winget install NSIS.NSIS`):

```bash
makensis installer/rdm.nsi
```

Use the release build or the installed app. The debug build (`target/debug/RDM.exe`) opens a console window on purpose, for logs.

`RDM.exe <url>` works like `IDMan.exe /d <url>`: the URL goes through Add URL, including the quality picker for streams and video pages. If RDM is already running, it hands the URL to the running copy.

### Background use

The extension only works while RDM is running, so RDM lives in the tray:
- **Closing the window keeps RDM running.** To exit, use **Quit** in the tray icon's menu.
- **Start with Windows** (on by default, toggle it in Options) launches RDM with `--hidden`, straight into the tray without a window.
- **Single instance.** Launching RDM again just shows the running one.

Measured idle footprint (private memory / CPU). The UI is Slint with its software renderer, so there's no WebView or GPU process:

| | RDM | IDM 6.42 |
|---|---|---|
| Started with Windows, tray only | 3 MB / 0% | 11 MB / 0% |
| Window open | 7 MB / 0% | 11 MB / 0% |
| Window closed to tray | 7 MB / 0% | — |

The earlier WebView2 UI (Tauri) needed about 99 MB with the window open.

The app listens on `127.0.0.1:47123` for the extension. It rejects requests that carry a web page `Origin`.

## Install the extension (Chrome / Edge)

1. Open `chrome://extensions` (or `edge://extensions`) and turn on **Developer mode**.
2. Click **Load unpacked** and pick the `extension/` folder.
3. If IDM is installed, turn off its browser integration, not just its extension: in IDM go to **Options → General** and untick **Use advanced browser integration**. IDM captures downloads system-wide through its network driver, so it grabs them before any extension sees them. In testing it took every `.zip` from a fresh Chrome profile with no IDM extension, even after `IDMan.exe` had been closed.

What the extension does:
- **Downloads:** each browser download is paused and handed to RDM with the browser's own Cookie, Referer and User-Agent. The browser copy is then cancelled. If RDM isn't running, the browser keeps the download.
- **Video:** media responses (`video/*`, `audio/*`, HLS playlists, ≥256 KB) are listed per tab. Each HLS stream is expanded into its qualities, and child playlists (single qualities, audio tracks) are hidden because they would only repeat those choices. Like IDM's panel, a **Download this video** button pins itself to the corner of a video as soon as it starts playing. It only does this for real videos (at least 320×180 and longer than 10 s, so not ads or animated thumbnails), it also shows on hover, and × hides it for that page. When playback starts, the qualities are fetched in the background, and the button shows the best one (for example `2160p60`). On YouTube, clicking opens the full list in about 60 ms instead of after the ~10 s yt-dlp needs. If the network shows nothing usable, as on YouTube, the list comes from the page itself through yt-dlp. The popup has the same list and a **Find qualities** button.
- **Files linked on the page:** the popup scans the page for links to downloadable files (archives, installers, documents, audio and video, plus any `<a download>`). Tick the ones you want and send them to RDM together, like IDM's "Download all links".
- **Right-click → Download with RDM** works on links, images, video and audio.

## Known gaps vs IDM

- YouTube and other video sites depend on yt-dlp, which has to be updated when sites change (`winget upgrade yt-dlp.yt-dlp`). DRM-protected streams (Netflix and similar) can't be downloaded.
- DASH `.mpd` playlists seen on the network aren't parsed directly. They only work through a page yt-dlp supports.
- For HLS with several audio languages, the playlist's default track is used, with no language picker.
- Form-POST downloads are left to the browser.
- HLS downloads restart from the beginning instead of resuming.
- There is no queue, scheduler or speed limiter.
