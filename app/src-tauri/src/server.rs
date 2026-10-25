//! Localhost API the browser extension talks to (IDM uses ws://127.0.0.1:1001 for the same job).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use rdm_engine::Kind;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

pub const PORT: u16 = 47123;

#[derive(Deserialize)]
struct NewDownload {
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    file_name: Option<String>,
    #[serde(default)]
    kind: Kind,
}

pub async fn run(app: AppHandle) {
    let router = Router::new()
        .route("/ping", get(|| async { Json(json!({ "app": "rdm", "version": env!("CARGO_PKG_VERSION") })) }))
        .route("/download", post(download))
        .with_state(app);
    match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
        Ok(listener) => {
            axum::serve(listener, router).await.ok();
        }
        Err(e) => eprintln!("browser integration disabled, port {PORT} unavailable: {e}"),
    }
}

async fn download(
    State(app): State<AppHandle>,
    headers: HeaderMap,
    Json(body): Json<NewDownload>,
) -> Result<Json<Value>, StatusCode> {
    if !from_extension(&headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    let id = crate::add_download(&app, body.url, body.headers, body.file_name, body.kind);
    if let Some(w) = app.get_webview_window("main") {
        w.show().ok();
        w.unminimize().ok();
        w.set_focus().ok();
    }
    Ok(Json(json!({ "id": id })))
}

/// Web pages can POST to localhost too; browsers always attach their Origin, which pages cannot fake.
fn from_extension(h: &HeaderMap) -> bool {
    match h.get("origin").and_then(|v| v.to_str().ok()) {
        None => true,
        Some(o) => o.starts_with("chrome-extension://") || o.starts_with("moz-extension://"),
    }
}
