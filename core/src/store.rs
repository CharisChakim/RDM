use crate::{Data, Settings, Status};
use std::path::{Path, PathBuf};

pub fn load(file: &Path, default_dir: PathBuf) -> Data {
    let mut data = std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Data>(&b).ok())
        .unwrap_or_else(|| Data {
            next_id: 1,
            settings: Settings { dir: default_dir, connections: 8, autostart: true },
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

pub fn save(file: &Path, data: &Data) {
    let bytes = serde_json::to_vec_pretty(data).expect("serialize");
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, bytes);
}
