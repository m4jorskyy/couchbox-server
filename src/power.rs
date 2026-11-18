use crate::protocol::PowerAction;
use std::process::Command;

pub fn handle_power_action(action: PowerAction) {
    match action {
        PowerAction::ScreenOff => screen_off(),
        PowerAction::SleepSystem => sleep_system(),
    }
}

#[cfg(target_os = "windows")]
fn screen_off() {
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            "(Add-Type '[DllImport(\"user32.dll\")]public static extern int SendMessage(int hWnd,int hMsg,int wParam,int lParam);' -Name a -Pas)::SendMessage(-1,0x0112,0xF170,2)",
        ])
        .spawn();
}

#[cfg(target_os = "macos")]
fn screen_off() {
    let _ = Command::new("pmset").arg("displaysleepnow").spawn();
}

#[cfg(target_os = "linux")]
fn screen_off() {
    let _ = Command::new("xset").args(["dpms", "force", "off"]).spawn();
}

#[cfg(target_os = "windows")]
fn sleep_system() {
    let _ = Command::new("rundll32.exe")
        .args(["powrprof.dll,SetSuspendState", "0,1,0"])
        .spawn();
}

#[cfg(target_os = "macos")]
fn sleep_system() {
    let _ = Command::new("pmset").arg("sleepnow").spawn();
}

#[cfg(target_os = "linux")]
fn sleep_system() {
    let _ = Command::new("systemctl").arg("suspend").spawn();
}
