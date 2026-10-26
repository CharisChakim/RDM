#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod server;
mod store;
mod ytdlp;

use rdm_engine::{Handle, Kind, Outcome, Request, Variant};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Status {
    Downloading,
    Paused,
    Completed,
    Error,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    id: u64,
    req: Request,
    status: Status,
    error: Option<String>,
    path: Option<PathBuf>,
    total: Option<u64>,
    received: u64,
    added: u64,
    #[serde(skip)]
    handle: Option<Handle>,
    #[serde(skip)]
    speed: f64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    dir: PathBuf,
    connections: usize,
}

#[derive(Serialize, Deserialize)]
struct Data {
    next_id: u64,
    settings: Settings,
    entries: Vec<Entry>,
}

pub struct App {
    data: Mutex<Data>,
    client: rdm_engine::Client,
}

/// What the UI table shows for one download.
#[derive(Serialize, Clone)]
struct View {
    id: u64,
    url: String,
    name: String,
    path: Option<PathBuf>,
    status: Status,
    error: Option<String>,
    total: Option<u64>,
    received: u64,
    speed: f64,
    connections: usize,
    resumable: bool,
    segments: Vec<[u64; 3]>,
    pieces: Option<[u64; 2]>,
    added: u64,
}

impl Entry {
    fn view(&self) -> View {
        let p = self.handle.as_ref().map(|h| h.progress());
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| self.req.file_name.clone())
            .unwrap_or_else(|| self.req.url.clone());
        View {
            id: self.id,
            url: self.req.url.clone(),
            name,
            path: self.path.clone(),
            status: self.status,
            error: self.error.clone(),
            total: self.total,
            received: self.received,
            speed: self.speed,
            connections: p.as_ref().map_or(0, |p| p.connections),
            resumable: p.as_ref().map_or(self.req.kind == Kind::File, |p| p.resumable),
            pieces: p.as_ref().and_then(|p| p.pieces),
            segments: p.map(|p| p.segments).unwrap_or_default(),
            added: self.added,
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// Starts (or resumes) the engine for an entry and records the outcome when it ends.
fn launch(app: &AppHandle, id: u64) {
    let state = app.state::<App>();
    let req = {
        let mut data = state.data.lock().unwrap();
        let Some(e) = data.entries.iter_mut().find(|e| e.id == id) else { return };
        if e.handle.is_some() {
            return;
        }
        e.status = Status::Downloading;
        e.error = None;
        e.req.clone()
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<App>();
        let (handle, task) = rdm_engine::start(state.client.clone(), req);
        set(&app, id, |e| e.handle = Some(handle.clone()));
        let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
        let p = handle.progress();
        set(&app, id, |e| {
            e.handle = None;
            e.speed = 0.0;
            e.received = p.received;
            e.total = p.total.or(e.total);
            if let Some(path) = &p.path {
                e.path = Some(path.clone());
                // Pin the name so a later resume finds the same .part file.
                e.req.file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            }
            match result {
                Ok(Outcome::Completed(path)) => {
                    e.status = Status::Completed;
                    e.path = Some(path);
                }
                Ok(Outcome::Paused) => e.status = Status::Paused,
                Err(err) => {
                    e.status = Status::Error;
                    e.error = Some(format!("{err:#}"));
                }
            }
        });
        store::save(&app);
        emit(&app);
    });
}

fn set(app: &AppHandle, id: u64, f: impl FnOnce(&mut Entry)) {
    let state = app.state::<App>();
    let mut data = state.data.lock().unwrap();
    if let Some(e) = data.entries.iter_mut().find(|e| e.id == id) {
        f(e);
    }
}

fn emit(app: &AppHandle) {
    let views: Vec<View> = app.state::<App>().data.lock().unwrap().entries.iter().map(Entry::view).collect();
    app.emit("downloads", views).ok();
}

/// A new download as asked for by the UI or the browser extension.
#[derive(Deserialize)]
pub struct Job {
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    file_name: Option<String>,
    /// None = guess from the URL.
    #[serde(default)]
    kind: Option<Kind>,
    /// HLS: separate audio track of the chosen quality.
    #[serde(default)]
    audio_url: Option<String>,
}

pub fn add_download(app: &AppHandle, job: Job) -> u64 {
    let url = job.url.trim().to_string();
    let kind = job.kind.unwrap_or(if url.split('?').next().unwrap_or("").ends_with(".m3u8") {
        Kind::Hls
    } else {
        Kind::File
    });
    let state = app.state::<App>();
    let id = {
        let mut data = state.data.lock().unwrap();
        let id = data.next_id;
        data.next_id += 1;
        let req = Request {
            url,
            headers: job.headers,
            dir: data.settings.dir.clone(),
            file_name: job.file_name,
            connections: data.settings.connections,
            kind,
            audio_url: job.audio_url,
        };
        data.entries.push(Entry {
            id,
            req,
            status: Status::Downloading,
            error: None,
            path: None,
            total: None,
            received: 0,
            added: now(),
            handle: None,
            speed: 0.0,
        });
        id
    };
    launch(app, id);
    store::save(app);
    emit(app);
    id
}

#[tauri::command]
fn add(app: AppHandle, job: Job) -> u64 {
    add_download(&app, job)
}

#[tauri::command]
async fn page_formats(url: String) -> Result<Vec<ytdlp::Choice>, String> {
    ytdlp::resolve(url.trim(), None).await
}

#[tauri::command]
async fn hls_variants(state: State<'_, App>, url: String) -> Result<Vec<Variant>, String> {
    rdm_engine::hls_variants(&state.client, url.trim(), &[]).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn list(state: State<App>) -> Vec<View> {
    state.data.lock().unwrap().entries.iter().map(Entry::view).collect()
}

#[tauri::command]
fn pause(state: State<App>, ids: Vec<u64>) {
    for e in state.data.lock().unwrap().entries.iter().filter(|e| ids.contains(&e.id)) {
        if let Some(h) = &e.handle {
            h.pause();
        }
    }
}

#[tauri::command]
fn resume(app: AppHandle, ids: Vec<u64>) {
    for id in ids {
        launch(&app, id);
    }
    emit(&app);
}

#[tauri::command]
fn remove(app: AppHandle, ids: Vec<u64>, delete_files: bool) {
    let state = app.state::<App>();
    let removed: Vec<Entry> = {
        let mut data = state.data.lock().unwrap();
        let (gone, keep) = std::mem::take(&mut data.entries).into_iter().partition(|e| ids.contains(&e.id));
        data.entries = keep;
        gone
    };
    for e in removed {
        if let Some(h) = &e.handle {
            h.pause();
        }
        if let Some(path) = e.path {
            // Unfinished leftovers always go; the finished file only when asked.
            let part = PathBuf::from(format!("{}.part", path.display()));
            let state_file = PathBuf::from(format!("{}.part.json", path.display()));
            let _ = std::fs::remove_file(state_file);
            let _ = std::fs::remove_file(part);
            if delete_files {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    store::save(&app);
    emit(&app);
}

#[tauri::command]
fn open(app: AppHandle, state: State<App>, id: u64, folder: bool) -> Result<(), String> {
    let path = state.data.lock().unwrap().entries.iter().find(|e| e.id == id).and_then(|e| e.path.clone());
    let path = path.ok_or("file not known yet")?;
    let r = if folder { app.opener().reveal_item_in_dir(&path) } else { app.opener().open_path(path.to_string_lossy(), None::<&str>) };
    r.map_err(|e| e.to_string())
}

#[tauri::command]
fn get_settings(state: State<App>) -> Settings {
    state.data.lock().unwrap().settings.clone()
}

#[tauri::command]
fn set_settings(app: AppHandle, settings: Settings) {
    app.state::<App>().data.lock().unwrap().settings = Settings { connections: settings.connections.clamp(1, 32), ..settings };
    store::save(&app);
}

#[tauri::command]
async fn pick_dir(app: AppHandle) -> Option<PathBuf> {
    app.dialog().file().blocking_pick_folder().and_then(|p| p.into_path().ok())
}

/// Refreshes speeds and pushes the table to the UI twice a second.
async fn ticker(app: AppHandle) {
    let mut last: std::collections::HashMap<u64, u64> = Default::default();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tick.tick().await;
        {
            let state = app.state::<App>();
            let mut data = state.data.lock().unwrap();
            for e in data.entries.iter_mut() {
                let Some(h) = &e.handle else { continue };
                let p = h.progress();
                let prev = last.insert(e.id, p.received).unwrap_or(p.received);
                let instant = p.received.saturating_sub(prev) as f64 * 2.0;
                e.speed = if e.speed == 0.0 { instant } else { e.speed * 0.7 + instant * 0.3 };
                e.received = p.received;
                e.total = p.total.or(e.total);
                if p.path.is_some() {
                    e.path = p.path;
                }
            }
        }
        emit(&app);
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let data = store::load(&handle);
            app.manage(App { data: Mutex::new(data), client: rdm_engine::client() });
            tauri::async_runtime::spawn(ticker(handle.clone()));
            tauri::async_runtime::spawn(server::run(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![add, page_formats, hls_variants, list, pause, resume, remove, open, get_settings, set_settings, pick_dir])
        .run(tauri::generate_context!())
        .expect("error while running RDM");
}
