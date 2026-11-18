mod input;
mod launcher;
mod power;
mod protocol;
mod server;
mod state;
mod tunnel;

use input::InputController;
use launcher::terminate_child;
use protocol::{StreamingApp, TvSnapshot};
use server::{launch_app_by_id, open_custom_url, return_to_home, start_server};
use state::{save_config, SharedRuntime};
use tauri::{AppHandle, Emitter, Manager, State};
use tunnel::{detect_cloudflared, spawn_quick_tunnel};

struct ManagedBackend {
    runtime: SharedRuntime,
}

#[tauri::command]
fn get_tv_snapshot(backend: State<ManagedBackend>) -> TvSnapshot {
    backend.runtime.snapshot()
}

#[tauri::command]
fn launch_app(
    app_handle: AppHandle,
    backend: State<ManagedBackend>,
    app_id: String,
) -> Result<TvSnapshot, String> {
    launch_app_by_id(&backend.runtime, &app_handle, &app_id)
}

#[tauri::command]
fn open_url(
    app_handle: AppHandle,
    backend: State<ManagedBackend>,
    url: String,
) -> Result<TvSnapshot, String> {
    open_custom_url(&backend.runtime, &app_handle, &url)
}

#[tauri::command]
fn return_home(app_handle: AppHandle, backend: State<ManagedBackend>) -> TvSnapshot {
    return_to_home(&backend.runtime, &app_handle)
}

#[tauri::command]
fn regenerate_pairing(app_handle: AppHandle, backend: State<ManagedBackend>) -> TvSnapshot {
    let snap = backend.runtime.regenerate_pairing();
    let _ = app_handle.emit("tv:snapshot", &snap);
    snap
}

#[tauri::command]
fn revoke_trusted_devices(app_handle: AppHandle, backend: State<ManagedBackend>) -> TvSnapshot {
    {
        let mut s = backend.runtime.inner.lock().unwrap();
        s.config.trusted_tokens.clear();
        save_config(&s.config);
    }
    let snap = backend.runtime.regenerate_pairing();
    let _ = app_handle.emit("tv:snapshot", &snap);
    snap
}

#[tauri::command]
fn add_custom_app(
    app_handle: AppHandle,
    backend: State<ManagedBackend>,
    name: String,
    url: String,
    accent: Option<String>,
    user_agent_mode: Option<String>,
) -> Result<TvSnapshot, String> {
    let clean_name = name.trim().to_string();
    let clean_url = url.trim().to_string();
    if clean_name.is_empty() || clean_url.is_empty() {
        return Err("Name and URL are required".to_string());
    }
    let normalized_url = if clean_url.starts_with("http://") || clean_url.starts_with("https://") {
        clean_url
    } else {
        format!("https://{}", clean_url)
    };

    let id = format!(
        "custom-{}",
        clean_name
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
    );

    {
        let mut s = backend.runtime.inner.lock().unwrap();
        s.config.apps.push(StreamingApp {
            id,
            name: clean_name,
            url: normalized_url,
            accent: accent.unwrap_or_else(|| "#38BDF8".to_string()),
            subtitle: "Custom streaming site".to_string(),
            user_agent_mode: user_agent_mode.unwrap_or_else(|| "default".to_string()),
            built_in: false,
        });
        save_config(&s.config);
    }

    let snap = backend.runtime.broadcast();
    let _ = app_handle.emit("tv:snapshot", &snap);
    Ok(snap)
}

#[tauri::command]
fn remove_custom_app(
    app_handle: AppHandle,
    backend: State<ManagedBackend>,
    app_id: String,
) -> TvSnapshot {
    {
        let mut s = backend.runtime.inner.lock().unwrap();
        s.config
            .apps
            .retain(|a| a.built_in || a.id != app_id);
        save_config(&s.config);
    }

    let snap = backend.runtime.broadcast();
    let _ = app_handle.emit("tv:snapshot", &snap);
    snap
}

#[tauri::command]
fn toggle_remote_tunnel(
    app_handle: AppHandle,
    backend: State<ManagedBackend>,
    enable: bool,
) -> Result<TvSnapshot, String> {
    if !enable {
        {
            let mut s = backend.runtime.inner.lock().unwrap();
            if let Some(mut child) = s.tunnel_child.take() {
                terminate_child(&mut child);
            }
            s.tunnel_url = None;
            s.tunnel_status = "off".to_string();
        }
        let snap = backend.runtime.broadcast();
        let _ = app_handle.emit("tv:snapshot", &snap);
        return Ok(snap);
    }

    let (bin, port) = {
        let mut s = backend.runtime.inner.lock().unwrap();
        if s.tunnel_child.is_some() {
            return Ok(backend.runtime.snapshot());
        }
        let detected = s.cloudflared_path.clone().or_else(detect_cloudflared);
        s.cloudflared_path = detected.clone();
        let bin = detected.ok_or_else(|| {
            if cfg!(target_os = "windows") {
                "cloudflared not found. Install it with: winget install Cloudflare.cloudflared"
                    .to_string()
            } else if cfg!(target_os = "macos") {
                "cloudflared not found. Install it with: brew install cloudflared".to_string()
            } else {
                "cloudflared not found. Install Cloudflare cloudflared to enable remote 5G access."
                    .to_string()
            }
        })?;
        s.tunnel_status = "starting".to_string();
        (bin, s.port)
    };

    let runtime_cb = backend.runtime.clone();
    let handle_cb = app_handle.clone();
    let child = spawn_quick_tunnel(&bin, port, move |public_url| {
        {
            let mut s = runtime_cb.inner.lock().unwrap();
            s.tunnel_url = Some(public_url);
            s.tunnel_status = "ready".to_string();
        }
        let snap = runtime_cb.broadcast();
        let _ = handle_cb.emit("tv:snapshot", &snap);
    })?;

    {
        let mut s = backend.runtime.inner.lock().unwrap();
        s.tunnel_child = Some(child);
    }

    let snap = backend.runtime.broadcast();
    let _ = app_handle.emit("tv:snapshot", &snap);
    Ok(snap)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let std_listener = std::net::TcpListener::bind("0.0.0.0:47832")
        .or_else(|_| std::net::TcpListener::bind("0.0.0.0:0"))
        .expect("Failed to bind LAN HTTP/WebSocket server port");
    std_listener
        .set_nonblocking(true)
        .expect("Failed to set listener nonblocking");
    let bound_port = std_listener.local_addr().unwrap().port();

    let runtime = SharedRuntime::new(bound_port, true, "Initializing...".to_string());
    let runtime_for_server = runtime.clone();

    tauri::Builder::default()
        .manage(ManagedBackend { runtime })
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let (input_controller, input_ready, input_note) =
                InputController::new(app_handle.clone());
            {
                let mut s = runtime_for_server.inner.lock().unwrap();
                s.input_ready = input_ready;
                s.input_note = input_note;
            }
            tauri::async_runtime::spawn(async move {
                if let Ok(tokio_listener) = tokio::net::TcpListener::from_std(std_listener) {
                    start_server(
                        runtime_for_server,
                        input_controller,
                        app_handle,
                        tokio_listener,
                    )
                    .await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_tv_snapshot,
            launch_app,
            open_url,
            return_home,
            regenerate_pairing,
            revoke_trusted_devices,
            add_custom_app,
            remove_custom_app,
            toggle_remote_tunnel,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let backend = window.state::<ManagedBackend>();
                let mut s = backend.runtime.inner.lock().unwrap();
                if let Some(mut child) = s.active_child.take() {
                    launcher::terminate_child(&mut child);
                }
                if let Some(mut tunnel) = s.tunnel_child.take() {
                    launcher::terminate_child(&mut tunnel);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running CouchBox TV");
}
