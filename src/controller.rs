use crate::launcher::{spawn_kiosk, spawn_kiosk_url, terminate_child};
use crate::protocol::TvSnapshot;
use crate::state::{browser_profile_dir, SharedRuntime};
use tauri::{AppHandle, Emitter, Manager};

/// Returns `(on_home, active_app_id)` after reconciling whether a manually closed browser exited.
pub fn sync_active_child(runtime: &SharedRuntime) -> (bool, Option<String>) {
    let mut s = runtime.inner.lock().unwrap();
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

    let profile_dir = browser_profile_dir();
    let child = spawn_kiosk(&browser, &app_cfg, &profile_dir)?;

    {
        let mut s = runtime.inner.lock().unwrap();
        s.active_child = Some(child);
        s.active_app_id = Some(app_cfg.id.clone());
        s.active_app_name = app_cfg.name.clone();
    }

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

    let profile_dir = browser_profile_dir();
    let child = spawn_kiosk_url(&browser, &normalized, ua_mode, &profile_dir)?;

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
