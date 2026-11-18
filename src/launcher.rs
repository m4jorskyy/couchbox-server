use crate::protocol::StreamingApp;
use crate::state::browser_profile_dir;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[derive(Debug, Clone)]
pub struct BrowserInfo {
    pub name: String,
    pub path: PathBuf,
}

const TV_USER_AGENT: &str = "Mozilla/5.0 (SMART-TV; LINUX; Tizen 6.5) AppleWebKit/537.36 (KHTML, like Gecko) 85.0.4183.93/6.5 TV Safari/537.36";

pub fn detect_browser() -> Option<BrowserInfo> {
    let candidates = browser_candidates();
    for (name, path) in candidates {
        if path.exists() {
            return Some(BrowserInfo {
                name: name.to_string(),
                path,
            });
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn browser_candidates() -> Vec<(&'static str, PathBuf)> {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let program_files = std::env::var("PROGRAMFILES").unwrap_or_else(|_| r"C:\Program Files".into());
    let program_files_x86 =
        std::env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());

    vec![
        (
            "Google Chrome",
            PathBuf::from(&program_files).join(r"Google\Chrome\Application\chrome.exe"),
        ),
        (
            "Google Chrome",
            PathBuf::from(&program_files_x86).join(r"Google\Chrome\Application\chrome.exe"),
        ),
        (
            "Google Chrome",
            PathBuf::from(&local_app_data).join(r"Google\Chrome\Application\chrome.exe"),
        ),
        (
            "Microsoft Edge",
            PathBuf::from(&program_files_x86).join(r"Microsoft\Edge\Application\msedge.exe"),
        ),
        (
            "Microsoft Edge",
            PathBuf::from(&program_files).join(r"Microsoft\Edge\Application\msedge.exe"),
        ),
        (
            "Brave",
            PathBuf::from(&program_files).join(r"BraveSoftware\Brave-Browser\Application\brave.exe"),
        ),
        (
            "Brave",
            PathBuf::from(&local_app_data).join(r"BraveSoftware\Brave-Browser\Application\brave.exe"),
        ),
    ]
}

#[cfg(target_os = "macos")]
fn browser_candidates() -> Vec<(&'static str, PathBuf)> {
    vec![
        (
            "Google Chrome",
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        ),
        (
            "Brave",
            PathBuf::from("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"),
        ),
        (
            "Microsoft Edge",
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        ),
        (
            "Chromium",
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        ),
    ]
}

#[cfg(target_os = "linux")]
fn browser_candidates() -> Vec<(&'static str, PathBuf)> {
    let bins = [
        ("Google Chrome", "google-chrome"),
        ("Google Chrome", "google-chrome-stable"),
        ("Chromium", "chromium"),
        ("Chromium", "chromium-browser"),
        ("Brave", "brave-browser"),
        ("Microsoft Edge", "microsoft-edge"),
    ];
    let mut found = Vec::new();
    for (name, bin) in bins {
        if let Ok(out) = Command::new("which").arg(bin).output() {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    found.push((name, PathBuf::from(p)));
                }
            }
        }
    }
    found
}

pub fn spawn_kiosk(browser: &BrowserInfo, app: &StreamingApp) -> Result<Child, String> {
    spawn_kiosk_url(browser, &app.url, &app.user_agent_mode)
}

pub fn spawn_kiosk_url(
    browser: &BrowserInfo,
    url: &str,
    user_agent_mode: &str,
) -> Result<Child, String> {
    let profile = browser_profile_dir();
    let mut cmd = Command::new(&browser.path);
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--autoplay-policy=no-user-gesture-required")
        .arg("--disable-infobars")
        .arg("--disable-session-crashed-bubble")
        .arg("--new-window")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if user_agent_mode == "tv" {
        cmd.arg(format!("--user-agent={}", TV_USER_AGENT));
    }

    // On macOS, combining --kiosk and --start-fullscreen triggers AppKit NSApplicationPresentationFullScreen warnings
    // and fights HTML5 video fullscreen (Twitch/Netflix). --app + --start-fullscreen avoids it cleanly.
    #[cfg(target_os = "macos")]
    {
        cmd.arg("--start-fullscreen")
            .arg(format!("--app={}", url));
    }

    #[cfg(not(target_os = "macos"))]
    {
        cmd.arg("--kiosk").arg(url);
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to launch {} ({}): {}", browser.name, Path::new(&browser.path).display(), e))
}

pub fn terminate_child(child: &mut Child) {
    let pid = child.id();

    #[cfg(target_os = "windows")]
    {
        // Kill the entire browser process tree cleanly for our dedicated profile instance
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = child.kill();
        // Best-effort wait so we don't leave a zombie
        let _ = child.wait();
        let _ = pid;
    }
}
