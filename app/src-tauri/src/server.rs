//! Localhost API the browser extension talks to (IDM uses ws://127.0.0.1:1001 for the same job).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use rdm_engine::Variant;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::{App, Job};

pub const PORT: u16 = 47123;

#[derive(Deserialize)]
struct Playlist {
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
}

pub async fn run(app: AppHandle) {
    let router = Router::new()
        .route("/ping", get(|| async { Json(json!({ "app": "rdm", "version": env!("CARGO_PKG_VERSION") })) }))
        .route("/download", post(download))
        .route("/hls/variants", post(variants))
        .route("/page/formats", post(page_formats))
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
    Json(job): Json<Job>,
) -> Result<Json<Value>, StatusCode> {
    if !from_extension(&headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    let id = crate::add_download(&app, job);
    if let Some(w) = app.get_webview_window("main") {
        w.show().ok();
        w.unminimize().ok();
        w.set_focus().ok();
    }
    Ok(Json(json!({ "id": id })))
}

/// Qualities of an HLS playlist, fetched with the browser's own headers (cookies, referer).
async fn variants(
    State(app): State<AppHandle>,
    headers: HeaderMap,
    Json(body): Json<Playlist>,
) -> Result<Json<Vec<Variant>>, (StatusCode, String)> {
    if !from_extension(&headers) {
        return Err((StatusCode::FORBIDDEN, String::new()));
    }
    let client = app.state::<App>().client.clone();
    rdm_engine::hls_variants(&client, &body.url, &body.headers)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("{e:#}")))
}

/// Qualities of a video page (YouTube, Vimeo...), resolved by yt-dlp with the browser's cookies.
async fn page_formats(
    headers: HeaderMap,
    Json(body): Json<Playlist>,
) -> Result<Json<Vec<crate::ytdlp::Choice>>, (StatusCode, String)> {
    if !from_extension(&headers) {
        return Err((StatusCode::FORBIDDEN, String::new()));
    }
    let cookie = body.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("cookie")).map(|(_, v)| v.as_str());
    crate::ytdlp::resolve(&body.url, cookie).await.map(Json).map_err(|e| (StatusCode::BAD_GATEWAY, e))
}

/// Web pages can POST to localhost too; browsers always attach their Origin, which pages cannot fake.
fn from_extension(h: &HeaderMap) -> bool {
    match h.get("origin").and_then(|v| v.to_str().ok()) {
        None => true,
        Some(o) => o.starts_with("chrome-extension://") || o.starts_with("moz-extension://"),
    }
}
