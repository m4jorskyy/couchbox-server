use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;

pub fn detect_cloudflared() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let candidates = vec![
        PathBuf::from("cloudflared.exe"),
        PathBuf::from(r"C:\Program Files (x86)\cloudflared\cloudflared.exe"),
        PathBuf::from(r"C:\Program Files\cloudflared\cloudflared.exe"),
    ];

    #[cfg(target_os = "macos")]
    let candidates = vec![
        PathBuf::from("/opt/homebrew/bin/cloudflared"),
        PathBuf::from("/usr/local/bin/cloudflared"),
    ];

    #[cfg(target_os = "linux")]
    let candidates = vec![
        PathBuf::from("/usr/local/bin/cloudflared"),
        PathBuf::from("/usr/bin/cloudflared"),
    ];

    for path in candidates {
        if path.exists() {
            return Some(path);
        }
    }

    // Fallback: check PATH via `which` / `where`
    #[cfg(target_os = "windows")]
    let probe = Command::new("where").arg("cloudflared").output();
    #[cfg(not(target_os = "windows"))]
    let probe = Command::new("which").arg("cloudflared").output();

    if let Ok(out) = probe {
        if out.status.success() {
            let line = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !line.is_empty() {
                return Some(PathBuf::from(line));
            }
        }
    }
    None
}

pub fn spawn_quick_tunnel<F>(bin: &PathBuf, port: u16, on_url: F) -> Result<Child, String>
where
    F: FnOnce(String) + Send + 'static,
{
    let mut child = Command::new(bin)
        .args([
            "tunnel",
            "--url",
            &format!("http://127.0.0.1:{}", port),
            "--no-autoupdate",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start cloudflared: {}", e))?;

    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            let mut callback = Some(on_url);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(url) = extract_trycloudflare_url(&line) {
                    if let Some(cb) = callback.take() {
                        cb(url);
                    }
                }
            }
        });
    }

    Ok(child)
}

fn extract_trycloudflare_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '|' || c == '"' || c == '\'')
        .unwrap_or(rest.len());
    let candidate = &rest[..end];
    if candidate.contains(".trycloudflare.com") {
        Some(candidate.trim_end_matches('/').to_string())
    } else {
        None
    }
}
