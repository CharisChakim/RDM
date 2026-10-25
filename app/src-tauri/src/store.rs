use crate::{App, Data, Settings, Status};
use tauri::{AppHandle, Manager};

fn file(app: &AppHandle) -> std::path::PathBuf {
    app.path().app_data_dir().expect("app data dir").join("downloads.json")
}

pub fn load(app: &AppHandle) -> Data {
    let mut data = std::fs::read(file(app))
        .ok()
        .and_then(|b| serde_json::from_slice::<Data>(&b).ok())
        .unwrap_or_else(|| Data {
            next_id: 1,
            settings: Settings {
                dir: app.path().download_dir().unwrap_or_else(|_| std::env::temp_dir()),
                connections: 8,
            },
            entries: vec![],
        });
    // Whatever was running when the app closed is now paused (like IDM, no auto-restart).
    for e in &mut data.entries {
        if e.status == Status::Downloading {
            e.status = Status::Paused;
        }
    }
    data
}

pub fn save(app: &AppHandle) {
    let path = file(app);
    let bytes = serde_json::to_vec_pretty(&*app.state::<App>().data.lock().unwrap()).expect("serialize");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, bytes);
}
