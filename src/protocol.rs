use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingApp {
    pub id: String,
    pub name: String,
    pub url: String,
    pub accent: String,
    pub subtitle: String,
    pub user_agent_mode: String, // "default" | "tv"
    pub built_in: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteCommand {
    Nav {
        action: NavAction,
    },
    Trackpad {
        action: TrackpadAction,
        dx: Option<i32>,
        dy: Option<i32>,
    },
    Text {
        mode: TextMode,
        value: String,
        #[serde(default)]
        submit: bool,
    },
    Media {
        action: MediaAction,
    },
    App {
        action: AppAction,
        app_id: Option<String>,
        url: Option<String>,
    },
    Power {
        action: PowerAction,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavAction {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
    BrowserBack,
    Home,
    Fullscreen,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackpadAction {
    Move,
    Click,
    RightClick,
    DoubleClick,
    Scroll,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMode {
    Type,
    Key,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaAction {
    PlayPause,
    SeekBack,
    SeekForward,
    Mute,
    VolUp,
    VolDown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppAction {
    Launch,
    CloseActive,
    OpenUrl,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerAction {
    ScreenOff,
    SleepSystem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    pub lan_ip: String,
    pub port: u16,
    pub base_url: String,
    pub qr_url: String,
    pub pin: String,
    pub tunnel_url: Option<String>,
    pub tunnel_status: String, // "off" | "starting" | "ready" | "error"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub os: String,
    pub browser_found: bool,
    pub browser_name: String,
    pub browser_path: Option<String>,
    pub input_ready: bool,
    pub input_note: String,
    pub cloudflared_found: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvSnapshot {
    pub active_app_id: Option<String>,
    pub active_app_name: String,
    pub apps: Vec<StreamingApp>,
    pub connected_phones: usize,
    pub trusted_device_count: usize,
    pub pairing: PairingInfo,
    pub setup: SetupStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSync {
    pub active_app_id: Option<String>,
    pub active_app_name: String,
    pub apps: Vec<StreamingApp>,
    pub connected_phones: usize,
}

impl From<&TvSnapshot> for RemoteSync {
    fn from(snap: &TvSnapshot) -> Self {
        Self {
            active_app_id: snap.active_app_id.clone(),
            active_app_name: snap.active_app_name.clone(),
            apps: snap.apps.clone(),
            connected_phones: snap.connected_phones,
        }
    }
}
