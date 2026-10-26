//! Localhost API the browser extension talks to (IDM uses ws://127.0.0.1:1001 for the same job).

use crate::{ytdlp, Core, Job};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use rdm_engine::Variant;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub const PORT: u16 = 47123;

#[derive(Deserialize)]
struct Playlist {
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
}

pub async fn run(core: Arc<Core>) {
    let router = Router::new()
        .route("/ping", get(|| async { Json(json!({ "app": "rdm", "version": env!("CARGO_PKG_VERSION") })) }))
        .route("/download", post(download))
        .route("/hls/variants", post(variants))
        .route("/page/formats", post(page_formats))
        .route("/show", post(show))
        .route("/open", post(open))
        .with_state(core);
    match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
        Ok(listener) => {
            axum::serve(listener, router).await.ok();
        }
        Err(e) => eprintln!("browser integration disabled, port {PORT} unavailable: {e}"),
    }
}

async fn download(
    State(core): State<Arc<Core>>,
    headers: HeaderMap,
    Json(job): Json<Job>,
) -> Result<Json<Value>, StatusCode> {
    if !from_extension(&headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    let id = core.add(job);
    (core.hooks.show)();
    Ok(Json(json!({ "id": id })))
}

/// A second RDM launch asks the running one to show itself, then exits.
async fn show(State(core): State<Arc<Core>>, headers: HeaderMap) -> StatusCode {
    if !from_extension(&headers) {
        return StatusCode::FORBIDDEN;
    }
    (core.hooks.show)();
    StatusCode::NO_CONTENT
}

/// `RDM.exe <url>` while RDM already runs: the running one handles the URL like Add URL.
async fn open(State(core): State<Arc<Core>>, headers: HeaderMap, Json(body): Json<Playlist>) -> StatusCode {
    if !from_extension(&headers) {
        return StatusCode::FORBIDDEN;
    }
    (core.hooks.open_url)(body.url);
    StatusCode::NO_CONTENT
}

/// Qualities of an HLS playlist, fetched with the browser's own headers (cookies, referer).
async fn variants(
    State(core): State<Arc<Core>>,
    headers: HeaderMap,
    Json(body): Json<Playlist>,
) -> Result<Json<Vec<Variant>>, (StatusCode, String)> {
    if !from_extension(&headers) {
        return Err((StatusCode::FORBIDDEN, String::new()));
    }
    rdm_engine::hls_variants(&core.client, &body.url, &body.headers)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("{e:#}")))
}

/// Qualities of a video page (YouTube, Vimeo...), resolved by yt-dlp with the browser's cookies.
async fn page_formats(
    headers: HeaderMap,
    Json(body): Json<Playlist>,
) -> Result<Json<Vec<ytdlp::Choice>>, (StatusCode, String)> {
    if !from_extension(&headers) {
        return Err((StatusCode::FORBIDDEN, String::new()));
    }
    let cookie = body.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("cookie")).map(|(_, v)| v.as_str());
    ytdlp::resolve(&body.url, cookie).await.map(Json).map_err(|e| (StatusCode::BAD_GATEWAY, e))
}

/// Web pages can POST to localhost too; browsers always attach their Origin, which pages cannot fake.
fn from_extension(h: &HeaderMap) -> bool {
    match h.get("origin").and_then(|v| v.to_str().ok()) {
        None => true,
        Some(o) => o.starts_with("chrome-extension://") || o.starts_with("moz-extension://"),
    }
}
