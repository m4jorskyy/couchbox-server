use crate::launcher::{detect_browser, BrowserInfo};
use crate::protocol::{PairingInfo, SetupStatus, StreamingApp, TvSnapshot};
use crate::tunnel::detect_cloudflared;
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistentConfig {
    pub apps: Vec<StreamingApp>,
    pub trusted_tokens: HashSet<String>,
}

impl Default for PersistentConfig {
    fn default() -> Self {
        Self {
            apps: default_apps(),
            trusted_tokens: HashSet::new(),
        }
    }
}

pub fn default_apps() -> Vec<StreamingApp> {
    vec![
        StreamingApp {
            id: "youtube".into(),
            name: "YouTube".into(),
            url: "https://www.youtube.com/tv".into(),
            accent: "#FF2E2E".into(),
            subtitle: "10-foot Smart TV interface".into(),
            user_agent_mode: "tv".into(),
            built_in: true,
        },
        StreamingApp {
            id: "netflix".into(),
            name: "Netflix".into(),
            url: "https://www.netflix.com".into(),
            accent: "#E50914".into(),
            subtitle: "Movies & series with DRM".into(),
            user_agent_mode: "default".into(),
            built_in: true,
        },
        StreamingApp {
            id: "twitch".into(),
            name: "Twitch".into(),
            url: "https://www.twitch.tv".into(),
            accent: "#9146FF".into(),
            subtitle: "Live streams & esports".into(),
            user_agent_mode: "default".into(),
            built_in: true,
        },
        StreamingApp {
            id: "spotify".into(),
            name: "Spotify".into(),
            url: "https://open.spotify.com".into(),
            accent: "#1DB954".into(),
            subtitle: "Music, playlists & podcasts".into(),
            user_agent_mode: "default".into(),
            built_in: true,
        },
    ]
}

pub fn app_data_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("CouchBox");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn browser_profile_dir() -> PathBuf {
    let dir = app_data_dir().join("tv-browser-profile");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn config_path() -> PathBuf {
    app_data_dir().join("config.json")
}

pub fn load_config() -> PersistentConfig {
    let path = config_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(mut cfg) = serde_json::from_str::<PersistentConfig>(&raw) {
            let mut changed = false;
            for builtin in default_apps() {
                if !cfg.apps.iter().any(|a| a.id == builtin.id) {
                    cfg.apps.push(builtin);
                    changed = true;
                }
            }
            if changed {
                save_config(&cfg);
            }
            return cfg;
        }
    }
    let cfg = PersistentConfig::default();
    save_config(&cfg);
    cfg
}

pub fn save_config(cfg: &PersistentConfig) {
    let path = config_path();
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(path, json);
    }
}

pub fn random_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

pub fn random_pin() -> String {
    let n: u32 = rand::thread_rng().gen_range(1000..=9999);
    n.to_string()
}

pub fn detect_lan_ip() -> String {
    local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

pub struct InnerState {
    pub config: PersistentConfig,
    pub one_time_token: String,
    pub pin: String,
    pub lan_ip: String,
    pub port: u16,
    pub connected_phones: usize,
    pub active_app_id: Option<String>,
    pub active_app_name: String,
    pub active_child: Option<Child>,
    pub browser: Option<BrowserInfo>,
    pub input_ready: bool,
    pub input_note: String,
    pub cloudflared_path: Option<PathBuf>,
    pub tunnel_child: Option<Child>,
    pub tunnel_url: Option<String>,
    pub tunnel_status: String,
}

#[derive(Clone)]
pub struct SharedRuntime {
    pub inner: Arc<Mutex<InnerState>>,
    pub sync_tx: broadcast::Sender<TvSnapshot>,
}

impl SharedRuntime {
    pub fn new(port: u16, input_ready: bool, input_note: String) -> Self {
        let config = load_config();
        let browser = detect_browser();
        let cloudflared_path = detect_cloudflared();
        let lan_ip = detect_lan_ip();
        let one_time_token = random_token(24);
        let pin = random_pin();
        let (sync_tx, _) = broadcast::channel(32);

        Self {
            inner: Arc::new(Mutex::new(InnerState {
                config,
                one_time_token,
                pin,
                lan_ip,
                port,
                connected_phones: 0,
                active_app_id: None,
                active_app_name: "Home".to_string(),
                active_child: None,
                browser,
                input_ready,
                input_note,
                cloudflared_path,
                tunnel_child: None,
                tunnel_url: None,
                tunnel_status: "off".to_string(),
            })),
            sync_tx,
        }
    }

    pub fn snapshot(&self) -> TvSnapshot {
        let s = self.inner.lock().unwrap();
        let lan_base = format!("http://{}:{}", s.lan_ip, s.port);
        let base_url = s.tunnel_url.clone().unwrap_or(lan_base);
        let qr_url = format!("{}/?pair={}", base_url, s.one_time_token);

        let (browser_found, browser_name, browser_path) = match &s.browser {
            Some(b) => (true, b.name.clone(), Some(b.path.to_string_lossy().to_string())),
            None => (false, "Not detected".to_string(), None),
        };

        TvSnapshot {
            active_app_id: s.active_app_id.clone(),
            active_app_name: s.active_app_name.clone(),
            apps: s.config.apps.clone(),
            connected_phones: s.connected_phones,
            trusted_device_count: s.config.trusted_tokens.len(),
            pairing: PairingInfo {
                lan_ip: s.lan_ip.clone(),
                port: s.port,
                base_url,
                qr_url,
                pin: s.pin.clone(),
                tunnel_url: s.tunnel_url.clone(),
                tunnel_status: s.tunnel_status.clone(),
            },
            setup: SetupStatus {
                os: std::env::consts::OS.to_string(),
                browser_found,
                browser_name,
                browser_path,
                input_ready: s.input_ready,
                input_note: s.input_note.clone(),
                cloudflared_found: s.cloudflared_path.is_some(),
            },
        }
    }

    pub fn broadcast(&self) -> TvSnapshot {
        let snap = self.snapshot();
        let _ = self.sync_tx.send(snap.clone());
        snap
    }

    pub fn regenerate_pairing(&self) -> TvSnapshot {
        {
            let mut s = self.inner.lock().unwrap();
            s.one_time_token = random_token(24);
            s.pin = random_pin();
            s.lan_ip = detect_lan_ip();
            s.cloudflared_path = detect_cloudflared();
        }
        self.broadcast()
    }

    pub fn pair_with_qr_or_pin(&self, pair_token: Option<&str>, pin: Option<&str>) -> Option<String> {
        let mut s = self.inner.lock().unwrap();
        let valid_qr = pair_token.map(|t| !t.is_empty() && t == s.one_time_token).unwrap_or(false);
        let valid_pin = pin.map(|p| !p.is_empty() && p.trim() == s.pin).unwrap_or(false);

        if !valid_qr && !valid_pin {
            return None;
        }

        let device_token = random_token(32);
        s.config.trusted_tokens.insert(device_token.clone());
        save_config(&s.config);

        s.one_time_token = random_token(24);
        s.pin = random_pin();
        drop(s);
        self.broadcast();
        Some(device_token)
    }

    pub fn is_trusted(&self, token: &str) -> bool {
        let s = self.inner.lock().unwrap();
        s.config.trusted_tokens.contains(token)
    }
}
