// Always a GUI program, debug builds too: a console window next to a tray app only confuses.
#![windows_subsystem = "windows"]
//! Native RDM window (Slint, software renderer: no browser engine) on top of `rdm-core`.

mod autostart;
mod tray;

use rdm_core::{Core, Hooks, Job, Kind, Settings, Status, View};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

slint::include_modules!();

static CORE: OnceLock<Arc<Core>> = OnceLock::new();
static REDRAW_PENDING: AtomicBool = AtomicBool::new(false);

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

struct Ui {
    win: AppWindow,
    rows: Rc<VecModel<Row>>,
    selected: HashSet<u64>,
    anchor: Option<u64>,
    filter: String,
    /// Ids of the rows as shown, for shift-click ranges and Ctrl+A.
    order: Vec<u64>,
    /// What the quality dialog offers, in the same order.
    pending: Vec<Job>,
}

fn core() -> &'static Arc<Core> {
    CORE.get().expect("core started")
}

fn with_ui(f: impl FnOnce(&mut Ui)) {
    UI.with(|u| {
        if let Some(ui) = u.borrow_mut().as_mut() {
            f(ui)
        }
    })
}

fn main() {
    // Like `IDMan.exe /d <url>`: a URL on the command line goes through the Add URL flow.
    let url_arg = std::env::args().skip(1).find(|a| a.starts_with("http://") || a.starts_with("https://"));
    if already_running(url_arg.as_deref()) {
        return;
    }
    // Same place the earlier (WebView) versions kept the list, so history carries over.
    let data_dir = dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("dev.rdm.app");
    let downloads = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
    let core = Core::start(
        data_dir,
        downloads,
        Hooks {
            changed: Box::new(request_redraw),
            show: Box::new(|| {
                let _ = slint::invoke_from_event_loop(show_window);
            }),
            open_url: Box::new(|url| {
                let _ = slint::invoke_from_event_loop(move || {
                    show_window();
                    submit_url(url);
                });
            }),
        },
    );
    autostart::apply(core.settings().autostart);
    CORE.set(core).ok();

    let win = AppWindow::new().expect("window");
    let rows = Rc::new(VecModel::default());
    win.set_rows(ModelRc::from(rows.clone()));
    wire(&win);
    win.window().on_close_requested(|| slint::CloseRequestResponse::HideWindow);
    UI.with(|u| {
        *u.borrow_mut() = Some(Ui {
            win: win.clone_strong(),
            rows,
            selected: HashSet::new(),
            anchor: None,
            filter: "all".into(),
            order: vec![],
            pending: vec![],
        })
    });
    let _tray = tray::create(show_window, quit);
    if !std::env::args().any(|a| a == "--hidden") {
        show_window();
    }
    if let Some(url) = url_arg {
        submit_url(url);
    }
    // Keeps running with the window closed: RDM lives in the tray until Quit.
    slint::run_event_loop_until_quit().expect("event loop");
}

/// A second launch hands over to the running RDM through its localhost API: show the window,
/// or open the URL it was given.
fn already_running(url: Option<&str>) -> bool {
    use std::io::{Read, Write};
    let addr = "127.0.0.1:47123".parse().expect("addr");
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return false };
    let (path, body) = match url {
        Some(u) => ("/open", format!("{{\"url\":\"{}\"}}", u.replace('\\', "\\\\").replace('"', "\\\""))),
        None => ("/show", String::new()),
    };
    let _ = write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.read(&mut [0u8; 64]); // wait for the answer so the request isn't cut off
    true
}

fn show_window() {
    with_ui(|ui| {
        let _ = ui.win.show();
        ui.win.window().with_winit_window(|w| {
            w.set_minimized(false);
            w.focus_window();
        });
    });
    refresh();
}

fn quit() {
    core().shutdown();
    let _ = slint::quit_event_loop();
}

/// Called from engine threads; coalesces bursts into one redraw on the UI thread.
fn request_redraw() {
    if !REDRAW_PENDING.swap(true, Ordering::AcqRel) {
        let _ = slint::invoke_from_event_loop(|| {
            REDRAW_PENDING.store(false, Ordering::Release);
            refresh();
        });
    }
}

// ---- formatting -------------------------------------------------------------------------------

const CATEGORIES: [(&str, &str); 5] = [
    ("compressed", "zip rar 7z gz gzip bz2 xz tar tgz arj lzh ace sit sitx sea z iso img"),
    ("documents", "pdf doc docx xls xlsx ppt pptx pps txt rtf odt ods epub csv"),
    ("music", "mp3 wav wma aac m4a ogg oga flac opus aif ra mpa"),
    ("programs", "exe msi msu apk bin dmg deb rpm appimage"),
    ("video", "mp4 mkv avi mov wmv flv webm m4v mpg mpeg mpe 3gp ogv ts rm rmvb qt asf"),
];

fn ext(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// 1-based index into CATEGORIES (0 = other), for the file-type chip colour.
fn category_of(name: &str) -> i32 {
    let e = ext(name);
    CATEGORIES.iter().position(|(_, exts)| exts.split(' ').any(|x| x == e)).map_or(0, |i| i as i32 + 1)
}

fn chip_label(name: &str) -> String {
    match ext(name) {
        e if e.is_empty() || e.len() > 4 => "FILE".into(),
        e => e.to_ascii_uppercase(),
    }
}

fn matches(v: &View, filter: &str) -> bool {
    match filter {
        "all" => true,
        "finished" => v.status == Status::Completed,
        "unfinished" => v.status != Status::Completed,
        f => CATEGORIES.iter().find(|(k, _)| *k == f).is_some_and(|(_, exts)| exts.split(' ').any(|e| e == ext(&v.name))),
    }
}

fn size(b: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let (mut b, mut i) = (b, 0);
    while b >= 1024.0 && i < units.len() - 1 {
        b /= 1024.0;
        i += 1;
    }
    match i {
        0 => format!("{b:.0} {}", units[i]),
        _ if b < 10.0 => format!("{b:.2} {}", units[i]),
        _ => format!("{b:.1} {}", units[i]),
    }
}

/// HLS has no byte total up front, so its percentage comes from pieces done.
fn pct(v: &View) -> Option<f64> {
    match (v.total, v.pieces) {
        (Some(t), _) if t > 0 => Some(v.received as f64 / t as f64),
        (_, Some([d, t])) if t > 0 => Some(d as f64 / t as f64),
        _ => None,
    }
}

fn status_text(v: &View) -> String {
    match v.status {
        Status::Completed => "Complete".into(),
        Status::Error => "Error".into(),
        Status::Paused => pct(v).map_or("Paused".into(), |p| format!("Paused {:.1}%", p * 100.0)),
        Status::Downloading => pct(v).map_or_else(|| size(v.received as f64), |p| format!("{:.1}%", p * 100.0)),
    }
}

fn eta(v: &View) -> String {
    let (Status::Downloading, Some(total)) = (v.status, v.total) else { return String::new() };
    if v.speed < 1.0 {
        return String::new();
    }
    let s = (total.saturating_sub(v.received) as f64 / v.speed) as u64;
    match (s / 3600, s % 3600 / 60, s % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

fn date(secs: u64, fmt: &str) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format(fmt).to_string())
        .unwrap_or_default()
}

fn kind_code(s: Status) -> i32 {
    match s {
        Status::Downloading => 0,
        Status::Paused => 1,
        Status::Completed => 2,
        Status::Error => 3,
    }
}

/// IDM-style bar: one block per connection segment, filled up to what it has received.
fn segments(v: &View) -> Vec<Seg> {
    if v.status == Status::Completed {
        return vec![Seg { x: 0.0, done: 1.0 }];
    }
    match v.total {
        Some(t) if t > 0 && !v.segments.is_empty() => v
            .segments
            .iter()
            .map(|[start, got, _]| Seg { x: *start as f32 / t as f32, done: got.saturating_sub(*start) as f32 / t as f32 })
            .collect(),
        _ => vec![Seg { x: 0.0, done: pct(v).unwrap_or(0.0) as f32 }],
    }
}

// ---- drawing ----------------------------------------------------------------------------------

fn refresh() {
    with_ui(|ui| {
        if !ui.win.window().is_visible() {
            return; // nothing to draw into; show_window refreshes
        }
        let views = core().views();
        ui.selected.retain(|id| views.iter().any(|v| v.id == *id));
        let mut shown: Vec<&View> = views.iter().filter(|v| matches(v, &ui.filter)).collect();
        shown.sort_by(|a, b| b.id.cmp(&a.id));
        ui.order = shown.iter().map(|v| v.id).collect();

        let rows: Vec<Row> = shown
            .iter()
            .map(|v| {
                let downloading = v.status == Status::Downloading;
                Row {
                    id: v.id as i32,
                    name: v.name.clone().into(),
                    ext: chip_label(&v.name).into(),
                    cat: category_of(&v.name),
                    size: v.total.or((v.status == Status::Completed).then_some(v.received)).map(|t| size(t as f64)).unwrap_or_default().into(),
                    status: status_text(v).into(),
                    kind: kind_code(v.status),
                    pct: pct(v).unwrap_or(0.0) as f32,
                    bar: downloading && pct(v).is_some(),
                    left: eta(v).into(),
                    rate: if downloading { format!("{}/s", size(v.speed)) } else { String::new() }.into(),
                    conns: if downloading { v.connections.to_string() } else { String::new() }.into(),
                    added: date(v.added, "%d/%m/%y %H:%M").into(),
                    tip: v.error.clone().unwrap_or_else(|| v.url.clone()).into(),
                    selected: ui.selected.contains(&v.id),
                }
            })
            .collect();
        ui.rows.set_vec(rows);

        let count = |f: &str| match views.iter().filter(|v| matches(v, f)).count() {
            0 => String::new(),
            n => n.to_string(),
        };
        let item = |key: &str, label: &str| Category { key: key.into(), label: label.into(), count: count(key).into(), header: false };
        let header = |label: &str| Category { key: "".into(), label: label.into(), count: "".into(), header: true };
        ui.win.set_categories(ModelRc::new(VecModel::from(vec![
            header("CATEGORIES"),
            item("all", "All Downloads"),
            item("compressed", "Compressed"),
            item("documents", "Documents"),
            item("music", "Music"),
            item("programs", "Programs"),
            item("video", "Video"),
            header("STATUS"),
            item("unfinished", "Unfinished"),
            item("finished", "Finished"),
        ])));
        ui.win.set_filter(ui.filter.clone().into());
        ui.win.set_empty(views.is_empty());

        let active: Vec<&View> = views.iter().filter(|v| v.status == Status::Downloading).collect();
        let speed: f64 = active.iter().map(|v| v.speed).sum();
        ui.win.set_totals(if active.is_empty() { String::new() } else { format!("{} active \u{b7} {}/s", active.len(), size(speed)) }.into());

        let sel: Vec<&View> = views.iter().filter(|v| ui.selected.contains(&v.id)).collect();
        ui.win.set_can_resume(sel.iter().any(|v| matches!(v.status, Status::Paused | Status::Error)));
        ui.win.set_can_stop(sel.iter().any(|v| v.status == Status::Downloading));
        ui.win.set_can_delete(!sel.is_empty());
        ui.win.set_can_open_folder(sel.len() == 1 && sel[0].path.is_some());

        let one = (sel.len() == 1).then(|| sel[0]);
        ui.win.set_detail(one.is_some());
        if let Some(v) = one {
            let mut stats = vec![match v.total {
                Some(t) => format!("{} of {}", size(v.received as f64), size(t as f64)),
                None => size(v.received as f64),
            }];
            if v.status == Status::Downloading {
                stats.push(format!("{}/s", size(v.speed)));
                stats.push(format!("{} connections", v.connections));
                stats.push(match v.pieces {
                    Some([d, t]) => format!("{d}/{t} stream pieces"),
                    None => format!("{} segments", v.segments.len()),
                });
            }
            if let Some(e) = &v.error {
                stats.push(e.clone());
            }
            if !v.resumable {
                stats.push("resume not supported by server".into());
            }
            ui.win.set_d_name(v.name.clone().into());
            ui.win.set_d_stats(stats.join(" \u{b7} ").into());
            ui.win.set_d_url(v.url.clone().into());
            ui.win.set_d_path(v.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into());
            ui.win.set_segs(ModelRc::new(VecModel::from(segments(v))));
        }
    });
}

// ---- actions ----------------------------------------------------------------------------------

const VIDEO_SITES: [&str; 13] = [
    "youtube.com", "youtu.be", "vimeo.com", "instagram.com", "x.com", "twitter.com", "tiktok.com",
    "facebook.com", "fb.watch", "dailymotion.com", "twitch.tv", "reddit.com", "bilibili.com",
];

fn is_video_page(url: &str) -> bool {
    let host = url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or("").to_ascii_lowercase();
    VIDEO_SITES.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

fn select_only(id: u64) {
    with_ui(|ui| {
        ui.selected = HashSet::from([id]);
        ui.anchor = Some(id);
    });
}

/// Fills the quality dialog once the playlist or page has been read.
fn offer(title: &str, result: Result<Vec<(String, String, Job)>, String>) {
    with_ui(|ui| {
        if ui.win.get_dialog() != "quality" {
            return; // cancelled meanwhile
        }
        ui.win.set_quality_title(title.into());
        match result {
            Ok(list) => {
                ui.win.set_quality_message("".into());
                ui.win.set_quality_index(0);
                ui.win.set_qualities(ModelRc::new(VecModel::from(
                    list.iter().map(|(label, note, _)| Quality { label: label.into(), note: note.into() }).collect::<Vec<_>>(),
                )));
                ui.pending = list.into_iter().map(|(_, _, job)| job).collect();
            }
            Err(e) => ui.win.set_quality_message(format!("Could not read the qualities: {e}").into()),
        }
    });
}

fn submit_url(url: String) {
    let url = url.trim().to_string();
    if url.is_empty() {
        return;
    }
    let is_hls = url.split('?').next().unwrap_or("").ends_with(".m3u8");
    if !is_hls && !is_video_page(&url) {
        with_ui(|ui| ui.win.set_dialog("".into()));
        let id = core().add(Job { url, ..Default::default() });
        select_only(id);
        refresh();
        return;
    }
    with_ui(|ui| {
        ui.pending.clear();
        ui.win.set_qualities(ModelRc::new(VecModel::from(Vec::<Quality>::new())));
        ui.win.set_quality_title("Choose quality".into());
        ui.win.set_quality_message("Reading available qualities\u{2026}".into());
        ui.win.set_dialog("quality".into());
    });
    if is_hls {
        core().hls_variants(url, |r| {
            let r = r.map(|vs| {
                vs.into_iter()
                    .map(|v| {
                        let note = if v.audio_url.is_some() { "separate audio, merged" } else { "" };
                        let job = Job { url: v.url, kind: Some(Kind::Hls), audio_url: v.audio_url, ..Default::default() };
                        (v.label, note.to_string(), job)
                    })
                    .collect()
            });
            let _ = slint::invoke_from_event_loop(move || offer("Choose quality", r));
        });
    } else {
        // A page, not a file: yt-dlp finds the streams, RDM downloads them.
        core().page_formats(url, |r| {
            let r = r.map(|cs| {
                cs.into_iter()
                    .map(|c| {
                        let note = if c.audio_url.is_some() { "video + audio, merged" } else { "" };
                        let job = Job { url: c.url, kind: Some(c.kind), audio_url: c.audio_url, headers: c.headers, file_name: Some(c.file_name) };
                        (c.label, note.to_string(), job)
                    })
                    .collect()
            });
            let _ = slint::invoke_from_event_loop(move || offer("Choose quality", r));
        });
    }
}

fn selected_ids() -> Vec<u64> {
    let mut ids = vec![];
    with_ui(|ui| ids = ui.selected.iter().copied().collect());
    ids
}

fn open_path(path: &std::path::Path, folder: bool) {
    let mut cmd = std::process::Command::new("explorer.exe");
    if folder {
        cmd.arg(format!("/select,{}", path.display()));
    } else {
        cmd.arg(path);
    }
    let _ = cmd.spawn();
}

fn wire(win: &AppWindow) {
    win.on_row_pressed(|id, ctrl, shift| {
        let id = id as u64;
        with_ui(|ui| {
            if shift && ui.anchor.is_some() {
                let pos = |x| ui.order.iter().position(|o| *o == x);
                if let (Some(a), Some(b)) = (pos(ui.anchor.unwrap()), pos(id)) {
                    ui.selected = ui.order[a.min(b)..=a.max(b)].iter().copied().collect();
                }
            } else if ctrl {
                if !ui.selected.remove(&id) {
                    ui.selected.insert(id);
                }
                ui.anchor = Some(id);
            } else {
                ui.selected = HashSet::from([id]);
                ui.anchor = Some(id);
            }
        });
        refresh();
    });
    win.on_row_activated(|id| {
        let id = id as u64;
        let Some(v) = core().views().into_iter().find(|v| v.id == id) else { return };
        match (v.status, v.path) {
            (Status::Completed, Some(p)) => open_path(&p, false),
            (Status::Paused | Status::Error, _) => core().resume(&[id]),
            _ => {}
        }
    });
    win.on_filter_chosen(|key| {
        with_ui(|ui| ui.filter = key.to_string());
        refresh();
    });
    win.on_add_clicked(|| {
        with_ui(|ui| {
            ui.win.set_add_url("".into());
            ui.win.set_dialog("add".into());
        })
    });
    win.on_add_submit(|url| submit_url(url.to_string()));
    win.on_quality_accept(|i| {
        let mut job = None;
        with_ui(|ui| {
            job = ui.pending.get(i as usize).cloned();
            ui.win.set_dialog("".into());
        });
        if let Some(job) = job {
            let id = core().add(job);
            select_only(id);
            refresh();
        }
    });
    win.on_resume_clicked(|| core().resume(&selected_ids()));
    win.on_stop_clicked(|| core().pause(&selected_ids()));
    win.on_delete_clicked(|| {
        with_ui(|ui| {
            if !ui.selected.is_empty() {
                ui.win.set_delete_count(ui.selected.len() as i32);
                ui.win.set_dialog("delete".into());
            }
        })
    });
    win.on_delete_confirm(|files| {
        let ids = selected_ids();
        with_ui(|ui| {
            ui.selected.clear();
            ui.win.set_dialog("".into());
        });
        core().remove(&ids, files);
    });
    win.on_clean_clicked(|| {
        let done: Vec<u64> = core().views().iter().filter(|v| v.status == Status::Completed).map(|v| v.id).collect();
        core().remove(&done, false);
    });
    win.on_folder_clicked(|| {
        if let [id] = selected_ids()[..] {
            if let Some(p) = core().path_of(id) {
                open_path(&p, true);
            }
        }
    });
    win.on_options_clicked(|| {
        let s = core().settings();
        with_ui(|ui| {
            ui.win.set_opt_dir(s.dir.display().to_string().into());
            ui.win.set_opt_conns(s.connections as i32);
            ui.win.set_opt_autostart(s.autostart);
            ui.win.set_dialog("options".into());
        });
    });
    win.on_options_save(|| {
        let mut settings = None;
        with_ui(|ui| {
            settings = Some(Settings {
                dir: ui.win.get_opt_dir().to_string().into(),
                connections: ui.win.get_opt_conns().clamp(1, 32) as usize,
                autostart: ui.win.get_opt_autostart(),
            });
            ui.win.set_dialog("".into());
        });
        if let Some(s) = settings {
            autostart::apply(s.autostart);
            core().set_settings(s);
        }
    });
    win.on_browse_clicked(|| {
        // The picker runs its own message loop, so don't hold the UI state while it is open.
        let start = core().settings().dir;
        if let Some(dir) = rfd::FileDialog::new().set_directory(start).pick_folder() {
            with_ui(|ui| ui.win.set_opt_dir(dir.display().to_string().into()));
        }
    });
    win.on_select_all(|| {
        with_ui(|ui| ui.selected = ui.order.iter().copied().collect());
        refresh();
    });
    win.on_paste(|| {
        let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
        let text = text.trim();
        if text.starts_with("http://") || text.starts_with("https://") {
            with_ui(|ui| {
                ui.win.set_add_url(text.into());
                ui.win.set_dialog("add".into());
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_video_pages() {
        assert!(is_video_page("https://www.youtube.com/watch?v=x"));
        assert!(is_video_page("https://youtu.be/x"));
        assert!(!is_video_page("https://example.com/youtube.com/file.zip"));
        assert!(!is_video_page("https://notyoutube.com/x"));
    }

    #[test]
    fn file_type_chips() {
        assert_eq!((chip_label("a.zip"), category_of("a.zip")), ("ZIP".to_string(), 1));
        assert_eq!((chip_label("Movie.MP4"), category_of("Movie.MP4")), ("MP4".to_string(), 5));
        assert_eq!((chip_label("noext"), category_of("noext")), ("FILE".to_string(), 0));
        assert_eq!(chip_label("x.appimage"), "FILE");
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(size(512.0), "512 B");
        assert_eq!(size(3.2 * 1048576.0), "3.20 MB");
        assert_eq!(size(5.66 * 1073741824.0), "5.66 GB");
    }
}
