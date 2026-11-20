use crate::controller::{launch_app_by_id, open_custom_url, return_to_home, sync_active_child};
use crate::input::{InputController, InputOp};
use crate::power::handle_power_action;
use crate::protocol::{AppAction, NavAction, RemoteCommand, RemoteSync};
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
use tauri::{AppHandle, Emitter};
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
    state: RemoteSync,
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
                    state: RemoteSync::from(&snap),
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
        "state": RemoteSync::from(&initial)
    })) {
        let _ = ws_tx.send(Message::Text(json.into())).await;
    }

    let send_task = tokio::spawn(async move {
        while let Ok(snap) = sync_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&serde_json::json!({
                "type": "state_sync",
                "state": RemoteSync::from(&snap)
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

fn dispatch_command(state: &AxumState, cmd: RemoteCommand) {
    let (on_home, active_app_id) = sync_active_child(&state.runtime);

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
            // On YouTube TV, results update live as you type; pressing Return clicks
            // whichever letter is highlighted on the on-screen keyboard instead of submitting.
            let effective_submit = if active_app_id.as_deref() == Some("youtube") {
                false
            } else {
                submit
            };
            state.input.send(InputOp::Text {
                mode,
                value,
                submit: effective_submit,
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
