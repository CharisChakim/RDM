# RDM: Rust Download Manager

A personal IDM-style download manager: a multi-connection Rust engine, a Tauri desktop UI, and a Chrome/Edge extension that takes over browser downloads and detects videos.

```
engine/      download engine (library) + tests
cli/         `rdm` command-line front end, used for benchmarks
app/         Tauri v2 desktop app (src-tauri/ = Rust, ui/ = HTML/JS)
extension/   Chrome/Edge MV3 extension
bench/       IDM vs RDM benchmark script + local test server (Range, per-connection throttle)
```

## How the engine works

- **Probe without an extra round trip.** The first request asks for `Range: bytes=0-`. If the server answers 206, that response becomes connection #1.
- **Dynamic segmentation, like IDM.** When a connection becomes idle, it takes the segment with the most bytes left, splits it in half and downloads the second half. The original connection stops at the new midpoint. Connections keep doing this until no remaining piece is worth splitting (1 MiB minimum).
- **No "assembling" phase.** IDM writes parts to temporary files and merges them at the end. RDM writes every segment directly into one sparse `.part` file using positioned writes, then renames it when done.
- **Resume.** Segment state is saved to `<name>.part.json` every second. On resume, the file size, ETag and Last-Modified must still match.
- **HLS (`.m3u8`).** RDM lists every quality in a master playlist. The app's Add URL dialog and the extension's video menu let you pick one, and the default is the best quality. When a quality has a separate audio track (`EXT-X-MEDIA TYPE=AUDIO`), RDM downloads that track too, preferring AAC when a video is listed once per audio codec. Segments download in parallel and are written in order. RDM handles AES-128 encryption and fMP4 (`EXT-X-MAP`). If `ffmpeg` is on PATH, the result is remuxed or muxed to `.mp4` without re-encoding. Without ffmpeg, video and audio are kept as separate files.

## Run

```bash
cargo test -p rdm-engine
```

```bash
cargo run --release -p rdm-cli -- https://example.com/file.zip -c 8 -d .
```

```bash
cargo run --release -p rdm-app
```

To build an installer (NSIS), run this from `app/`:

```bash
npx tauri build
```

The app listens on `127.0.0.1:47123` for the extension. It rejects requests that carry a web page `Origin`.

## Install the extension (Chrome / Edge)

1. Open `chrome://extensions` (or `edge://extensions`) and turn on **Developer mode**.
2. Click **Load unpacked** and pick the `extension/` folder.
3. Turn off IDM's browser extension so the two don't fight over downloads.

What the extension does:
- **Downloads:** each browser download is paused and handed to RDM with the browser's own Cookie, Referer and User-Agent. The browser copy is then cancelled. If RDM isn't running, the browser keeps the download.
- **Video:** media responses (`video/*`, `audio/*`, HLS playlists, ≥256 KB) are listed per tab. Each HLS stream is expanded into its qualities, and child playlists (single qualities, audio tracks) are hidden because they would only repeat those choices. Hovering a `<video>` shows a **Download this video** button with that list. The popup shows the same list.
- **Files linked on the page:** the popup scans the page for links to downloadable files (archives, installers, documents, audio and video, plus any `<a download>`). Tick the ones you want and send them to RDM together, like IDM's "Download all links".
- **Right-click → Download with RDM** works on links, images, video and audio.

## Known gaps vs IDM

- YouTube (SABR/UMP streams, signature cipher) and DASH (`.mpd`) are not supported.
- For HLS with several audio languages, the playlist's default track is used, with no language picker.
- Form-POST downloads are left to the browser.
- HLS downloads restart from the beginning instead of resuming.
- There is no queue, scheduler or speed limiter.
