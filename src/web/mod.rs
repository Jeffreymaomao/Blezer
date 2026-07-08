//! axum web 層:內嵌 dashboard + JSON API,綁在單一 port。

use crate::config::{DEFAULT_DEVICE_WINDOW_SECS, DEFAULT_STATS_WINDOW_SECS};
use crate::db::{self, Db};
use crate::model::{DeviceJson, ObsPoint, Stats};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

#[derive(Clone)]
struct AppState {
    db: Db,
}

#[derive(Debug, Deserialize)]
struct WindowQuery {
    window: Option<i64>,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 啟動 HTTP server(長駐)。host 預設綁 127.0.0.1(只限本機)。
pub async fn serve(db: Db, host: &str, port: u16) -> anyhow::Result<()> {
    let state = AppState { db };
    let app = Router::new()
        .route("/", get(index))
        .route("/api/devices", get(devices))
        .route("/api/devices/{id}", get(device_history))
        .route("/api/stats", get(stats))
        .with_state(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Dashboard 已啟動:http://{addr}/");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

async fn devices(
    State(state): State<AppState>,
    Query(q): Query<WindowQuery>,
) -> Result<Json<Vec<DeviceJson>>, ApiError> {
    let window = q.window.unwrap_or(DEFAULT_DEVICE_WINDOW_SECS);
    let cutoff = now_secs() - window;
    let conn = state.db.lock().expect("db mutex poisoned");
    let rows = db::recent_devices(&conn, cutoff)?;
    Ok(Json(rows))
}

async fn device_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<WindowQuery>,
) -> Result<Json<Vec<ObsPoint>>, ApiError> {
    let window = q.window.unwrap_or(DEFAULT_STATS_WINDOW_SECS);
    let cutoff = now_secs() - window;
    let conn = state.db.lock().expect("db mutex poisoned");
    let rows = db::device_history(&conn, &id, cutoff)?;
    Ok(Json(rows))
}

async fn stats(
    State(state): State<AppState>,
    Query(q): Query<WindowQuery>,
) -> Result<Json<Stats>, ApiError> {
    let window = q.window.unwrap_or(DEFAULT_STATS_WINDOW_SECS);
    let now = now_secs();
    let conn = state.db.lock().expect("db mutex poisoned");
    let s = db::stats(&conn, window, now)?;
    Ok(Json(s))
}

/// 把 anyhow 錯誤包成 500,避免 handler 直接 panic。
struct ApiError(anyhow::Error);

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("error: {}", self.0)).into_response()
    }
}
