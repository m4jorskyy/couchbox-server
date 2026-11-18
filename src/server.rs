use crate::input::{InputController, InputOp};
use crate::launcher::{spawn_kiosk, spawn_kiosk_url, terminate_child};
use crate::power::handle_power_action;
use crate::protocol::{AppAction, NavAction, RemoteCommand, TvSnapshot};
use crate::state::SharedRuntime;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tauri::{AppHandle, Emitter, Manager};
use tower_http::cors::CorsLayer;

#[derive(Embed)]
#[folder = "../../WebstormProjects/couchbox-web/apps/remote-pwa/dist"]
struct RemotePwaAssets;

#[derive(Clone)]
pub struct AxumState {
    pub runtime: SharedRuntime,
    pub input: InputController,
    pub app_handle: AppHandle,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairRequest {
    pair_token: Option<String>,
    pin: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PairResponse {
    device_token: String,
    snapshot: TvSnapshot,
}

#[derive(Deserialize)]
struct WsQuery {
    token: String,
}

pub async fn start_server(
    runtime: SharedRuntime,
    input: InputController,
    app_handle: AppHandle,
    listener: tokio::net::TcpListener,
) {
    let state = AxumState {
        runtime,
        input,
        app_handle,
    };

    let router = Router::new()
        .route("/api/ping", get(ping_handler))
        .route("/api/pair", post(pair_handler))
        .route("/ws", get(ws_handler))
        .fallback(static_pwa_handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let _ = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await;
}

async fn ping_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true, "app": "CouchBox" }))
}

async fn pair_handler(
    State(state): State<AxumState>,
    Json(req): Json<PairRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .pair_with_qr_or_pin(req.pair_token.as_deref(), req.pin.as_deref())
    {
        Some(device_token) => {
            let snap = state.runtime.snapshot();
            let _ = state.app_handle.emit("tv:snapshot", &snap);
            (
                StatusCode::OK,
                Json(PairResponse {
                    device_token,
                    snapshot: snap,
                }),
            )
                .into_response()
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Invalid or expired QR token / PIN" })),
        )
            .into_response(),
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<AxumState>,
) -> impl IntoResponse {
    if !state.runtime.is_trusted(&query.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AxumState) {
    {
        let mut s = state.runtime.inner.lock().unwrap();
        s.connected_phones += 1;
    }
    let initial = state.runtime.broadcast();
    let _ = state.app_handle.emit("tv:snapshot", &initial);

    let (mut ws_tx, mut ws_rx) = socket.split();
    let mut sync_rx = state.runtime.sync_tx.subscribe();

    if let Ok(json) = serde_json::to_string(&serde_json::json!({
        "type": "state_sync",
        "snapshot": initial
    })) {
        let _ = ws_tx.send(Message::Text(json.into())).await;
    }

    let send_task = tokio::spawn(async move {
        while let Ok(snap) = sync_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&serde_json::json!({
                "type": "state_sync",
                "snapshot": snap
            })) {
                if ws_tx.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        if let Message::Text(raw) = msg {
            if let Ok(cmd) = serde_json::from_str::<RemoteCommand>(raw.as_str()) {
                dispatch_command(&state, cmd);
            }
        }
    }

    send_task.abort();

    {
        let mut s = state.runtime.inner.lock().unwrap();
        s.connected_phones = s.connected_phones.saturating_sub(1);
    }
    let updated = state.runtime.broadcast();
    let _ = state.app_handle.emit("tv:snapshot", &updated);
}

pub fn launch_app_by_id(
    runtime: &SharedRuntime,
    app_handle: &AppHandle,
    app_id: &str,
) -> Result<TvSnapshot, String> {
    let (browser, app_cfg) = {
        let mut s = runtime.inner.lock().unwrap();
        if let Some(mut child) = s.active_child.take() {
            terminate_child(&mut child);
        }
        let browser = s
            .browser
            .clone()
            .ok_or_else(|| "No Chrome, Edge, or Brave browser found on this laptop.".to_string())?;
        let app_cfg = s
            .config
            .apps
            .iter()
            .find(|a| a.id == app_id)
            .cloned()
            .ok_or_else(|| format!("Unknown app id: {}", app_id))?;
        (browser, app_cfg)
    };

    let child = spawn_kiosk(&browser, &app_cfg)?;

    {
        let mut s = runtime.inner.lock().unwrap();
        s.active_child = Some(child);
        s.active_app_id = Some(app_cfg.id.clone());
        s.active_app_name = app_cfg.name.clone();
    }

    // Move TV window out of the way so the kiosk browser receives immediate keyboard focus
    if let Some(win) = app_handle.get_webview_window("main") {
        let _ = win.minimize();
    }

    let snap = runtime.broadcast();
    let _ = app_handle.emit("tv:snapshot", &snap);
    Ok(snap)
}

pub fn open_custom_url(
    runtime: &SharedRuntime,
    app_handle: &AppHandle,
    url: &str,
) -> Result<TvSnapshot, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("URL cannot be empty".to_string());
    }
    let normalized = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    };

    let browser = {
        let mut s = runtime.inner.lock().unwrap();
        if let Some(mut child) = s.active_child.take() {
            terminate_child(&mut child);
        }
        s.browser
            .clone()
            .ok_or_else(|| "No Chrome, Edge, or Brave browser found on this laptop.".to_string())?
    };

    let ua_mode = if normalized.contains("youtube.com/tv") {
        "tv"
    } else {
        "default"
    };

    let child = spawn_kiosk_url(&browser, &normalized, ua_mode)?;

    {
        let mut s = runtime.inner.lock().unwrap();
        s.active_child = Some(child);
        s.active_app_id = Some("custom_url".to_string());
        s.active_app_name = normalized.clone();
    }

    if let Some(win) = app_handle.get_webview_window("main") {
        let _ = win.minimize();
    }

    let snap = runtime.broadcast();
    let _ = app_handle.emit("tv:snapshot", &snap);
    Ok(snap)
}

pub fn return_to_home(runtime: &SharedRuntime, app_handle: &AppHandle) -> TvSnapshot {
    {
        let mut s = runtime.inner.lock().unwrap();
        if let Some(mut child) = s.active_child.take() {
            terminate_child(&mut child);
        }
        s.active_app_id = None;
        s.active_app_name = "Home".to_string();
    }

    if let Some(win) = app_handle.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }

    let snap = runtime.broadcast();
    let _ = app_handle.emit("tv:snapshot", &snap);
    snap
}

fn dispatch_command(state: &AxumState, cmd: RemoteCommand) {
    let (on_home, active_app_id) = {
        let mut s = state.runtime.inner.lock().unwrap();
        // If user closed the browser manually, sync back to Home automatically
        let child_exited = s
            .active_child
            .as_mut()
            .map(|c| matches!(c.try_wait(), Ok(Some(_))))
            .unwrap_or(false);
        if child_exited {
            s.active_child = None;
            s.active_app_id = None;
            s.active_app_name = "Home".to_string();
        }
        (s.active_app_id.is_none(), s.active_app_id.clone())
    };

    match cmd {
        RemoteCommand::Nav { action } => match action {
            NavAction::Home => {
                return_to_home(&state.runtime, &state.app_handle);
            }
            other => {
                if on_home {
                    let mapped = match other {
                        NavAction::BrowserBack => NavAction::Back,
                        x => x,
                    };
                    let _ = state.app_handle.emit("remote:nav", mapped);
                } else {
                    // YouTube TV is a single-page TV app where Escape is native Back
                    let mapped = match other {
                        NavAction::BrowserBack if active_app_id.as_deref() == Some("youtube") => {
                            NavAction::Back
                        }
                        x => x,
                    };
                    state.input.send(InputOp::Nav(mapped));
                }
            }
        },
        RemoteCommand::Trackpad { action, dx, dy } => {
            state.input.send(InputOp::Trackpad {
                action,
                dx: dx.unwrap_or(0),
                dy: dy.unwrap_or(0),
            });
        }
        RemoteCommand::Text {
            mode,
            value,
            submit,
        } => {
            state.input.send(InputOp::Text {
                mode,
                value,
                submit,
            });
        }
        RemoteCommand::Media { action } => {
            state.input.send(InputOp::Media(action));
        }
        RemoteCommand::App {
            action,
            app_id,
            url,
        } => match action {
            AppAction::Launch => {
                if let Some(id) = app_id {
                    let _ = launch_app_by_id(&state.runtime, &state.app_handle, &id);
                }
            }
            AppAction::CloseActive => {
                return_to_home(&state.runtime, &state.app_handle);
            }
            AppAction::OpenUrl => {
                if let Some(u) = url {
                    let _ = open_custom_url(&state.runtime, &state.app_handle, &u);
                }
            }
        },
        RemoteCommand::Power { action } => {
            handle_power_action(action);
        }
    }
}

async fn static_pwa_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let asset_path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = RemotePwaAssets::get(asset_path) {
        let mime = mime_guess::from_path(asset_path).first_or_octet_stream();
        return (
            [(header::CONTENT_TYPE, mime.as_ref().to_string())],
            file.data.into_owned(),
        )
            .into_response();
    }

    if let Some(index) = RemotePwaAssets::get("index.html") {
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            index.data.into_owned(),
        )
            .into_response();
    }

    (
        StatusCode::NOT_FOUND,
        "Remote PWA not built yet. Run `npm run build:remote`.",
    )
        .into_response()
}
