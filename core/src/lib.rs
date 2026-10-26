//! Everything RDM does besides drawing: the download list, persistence, the localhost API for the
//! browser extension and the yt-dlp resolver. UIs call into `Core` and get told when to redraw.

mod server;
mod store;
pub mod ytdlp;

pub use rdm_engine::{Kind, Variant};
use rdm_engine::{Handle, Outcome, Request};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Status {
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub dir: PathBuf,
    pub connections: usize,
    /// Start hidden in the tray when Windows starts, so the browser extension always has RDM.
    #[serde(default = "yes")]
    pub autostart: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct Data {
    next_id: u64,
    settings: Settings,
    entries: Vec<Entry>,
}

/// What a UI shows for one download.
#[derive(Clone, Debug)]
pub struct View {
    pub id: u64,
    pub url: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub status: Status,
    pub error: Option<String>,
    pub total: Option<u64>,
    pub received: u64,
    pub speed: f64,
    pub connections: usize,
    pub resumable: bool,
    /// [start, received_up_to, end] per segment.
    pub segments: Vec<[u64; 3]>,
    pub pieces: Option<[u64; 2]>,
    pub added: u64,
}

/// A new download as asked for by a UI or the browser extension.
#[derive(Deserialize, Default, Clone, Debug)]
pub struct Job {
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub file_name: Option<String>,
    /// None = guess from the URL.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// Separate audio track of the chosen quality (HLS or video sites).
    #[serde(default)]
    pub audio_url: Option<String>,
}

/// How the core reaches back into the UI. Both may be called from any thread.
pub struct Hooks {
    /// The list or a download's progress changed: redraw.
    pub changed: Box<dyn Fn() + Send + Sync>,
    /// The browser handed over a download: bring the window up.
    pub show: Box<dyn Fn() + Send + Sync>,
    /// Another launch (`RDM.exe <url>`) passed a URL: start the add flow for it.
    pub open_url: Box<dyn Fn(String) + Send + Sync>,
}

pub struct Core {
    data: Mutex<Data>,
    client: rdm_engine::Client,
    store: PathBuf,
    rt: tokio::runtime::Runtime,
    hooks: Hooks,
}

impl Core {
    /// Loads the saved list from `data_dir` and starts the engine runtime, the progress ticker and
    /// the localhost API for the extension.
    pub fn start(data_dir: PathBuf, default_dir: PathBuf, hooks: Hooks) -> Arc<Core> {
        let store = data_dir.join("downloads.json");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let core = Arc::new(Core { data: Mutex::new(store::load(&store, default_dir)), client: rdm_engine::client(), store, rt, hooks });
        core.rt.spawn(ticker(core.clone()));
        core.rt.spawn(server::run(core.clone()));
        core
    }

    pub fn views(&self) -> Vec<View> {
        self.data.lock().unwrap().entries.iter().map(Entry::view).collect()
    }

    pub fn active(&self) -> bool {
        self.data.lock().unwrap().entries.iter().any(|e| e.handle.is_some())
    }

    pub fn add(self: &Arc<Self>, job: Job) -> u64 {
        let url = job.url.trim().to_string();
        let kind = job.kind.unwrap_or(if url.split('?').next().unwrap_or("").ends_with(".m3u8") {
            Kind::Hls
        } else {
            Kind::File
        });
        let id = {
            let mut data = self.data.lock().unwrap();
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
        self.launch(id);
        self.save();
        (self.hooks.changed)();
        id
    }

    pub fn pause(&self, ids: &[u64]) {
        for e in self.data.lock().unwrap().entries.iter().filter(|e| ids.contains(&e.id)) {
            if let Some(h) = &e.handle {
                h.pause();
            }
        }
    }

    pub fn resume(self: &Arc<Self>, ids: &[u64]) {
        for &id in ids {
            self.launch(id);
        }
        (self.hooks.changed)();
    }

    pub fn remove(&self, ids: &[u64], delete_files: bool) {
        let removed: Vec<Entry> = {
            let mut data = self.data.lock().unwrap();
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
                let _ = std::fs::remove_file(format!("{}.part.json", path.display()));
                let _ = std::fs::remove_file(format!("{}.part", path.display()));
                if delete_files {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        self.save();
        (self.hooks.changed)();
    }

    pub fn path_of(&self, id: u64) -> Option<PathBuf> {
        self.data.lock().unwrap().entries.iter().find(|e| e.id == id).and_then(|e| e.path.clone())
    }

    pub fn settings(&self) -> Settings {
        self.data.lock().unwrap().settings.clone()
    }

    pub fn set_settings(&self, settings: Settings) {
        self.data.lock().unwrap().settings = Settings { connections: settings.connections.clamp(1, 32), ..settings };
        self.save();
    }

    /// Pauses everything (each download saves its resume state) and writes the list.
    pub fn shutdown(&self) {
        self.pause(&self.views().iter().map(|v| v.id).collect::<Vec<_>>());
        std::thread::sleep(Duration::from_millis(300));
        self.save();
    }

    /// Runs async work on the engine runtime; `done` gets the result on a runtime thread.
    pub fn spawn<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
        done: impl FnOnce(T) + Send + 'static,
    ) {
        self.rt.spawn(async move { done(work.await) });
    }

    pub fn hls_variants(&self, url: String, done: impl FnOnce(Result<Vec<Variant>, String>) + Send + 'static) {
        let client = self.client.clone();
        self.spawn(
            async move { rdm_engine::hls_variants(&client, url.trim(), &[]).await.map_err(|e| format!("{e:#}")) },
            done,
        );
    }

    pub fn page_formats(&self, url: String, done: impl FnOnce(Result<Vec<ytdlp::Choice>, String>) + Send + 'static) {
        self.spawn(async move { ytdlp::resolve(url.trim(), None).await }, done);
    }

    /// Starts (or resumes) the engine for an entry and records the outcome when it ends.
    fn launch(self: &Arc<Self>, id: u64) {
        let req = {
            let mut data = self.data.lock().unwrap();
            let Some(e) = data.entries.iter_mut().find(|e| e.id == id) else { return };
            if e.handle.is_some() {
                return;
            }
            e.status = Status::Downloading;
            e.error = None;
            e.req.clone()
        };
        let core = self.clone();
        self.rt.spawn(async move {
            let (handle, task) = rdm_engine::start(core.client.clone(), req);
            core.set(id, |e| e.handle = Some(handle.clone()));
            let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
            let p = handle.progress();
            core.set(id, |e| {
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
            core.save();
            (core.hooks.changed)();
        });
    }

    fn set(&self, id: u64, f: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.data.lock().unwrap().entries.iter_mut().find(|e| e.id == id) {
            f(e);
        }
    }

    fn save(&self) {
        store::save(&self.store, &self.data.lock().unwrap());
    }
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

/// Refreshes speeds twice a second and asks the UI to redraw, but only while something downloads.
async fn ticker(core: Arc<Core>) {
    let mut last: std::collections::HashMap<u64, u64> = Default::default();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tick.tick().await;
        let active = {
            let mut data = core.data.lock().unwrap();
            let active = data.entries.iter().any(|e| e.handle.is_some());
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
            active
        };
        if active {
            (core.hooks.changed)();
        }
    }
}
