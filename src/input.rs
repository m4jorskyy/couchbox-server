use crate::protocol::{MediaAction, NavAction, TextMode, TrackpadAction};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
#[cfg(target_os = "macos")]
use std::cell::RefCell;
#[cfg(not(target_os = "macos"))]
use std::sync::mpsc;
#[cfg(not(target_os = "macos"))]
use std::thread;
use tauri::AppHandle;

#[derive(Debug, Clone)]
pub enum InputOp {
    Nav(NavAction),
    Trackpad {
        action: TrackpadAction,
        dx: i32,
        dy: i32,
    },
    Text {
        mode: TextMode,
        value: String,
        submit: bool,
    },
    Media(MediaAction),
}

#[cfg(target_os = "macos")]
thread_local! {
    static MAIN_THREAD_ENIGO: RefCell<Option<Enigo>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct InputController {
    #[cfg(target_os = "macos")]
    app_handle: AppHandle,
    #[cfg(not(target_os = "macos"))]
    tx: mpsc::Sender<InputOp>,
}

impl InputController {
    #[cfg(target_os = "macos")]
    pub fn new(app_handle: AppHandle) -> (Self, bool, String) {
        // On macOS, Apple's HIToolbox/TSM (used by enigo for Unicode keys & text typing)
        // MUST run on the main thread, otherwise macOS aborts with a queue assertion crash.
        let note = "Ready (runs on macOS main thread; grant Accessibility permission in System Settings if browser ignores input)".to_string();
        (Self { app_handle }, true, note)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn new(_app_handle: AppHandle) -> (Self, bool, String) {
        let (tx, rx) = mpsc::channel::<InputOp>();
        let (init_tx, init_rx) = mpsc::channel::<(bool, String)>();

        thread::spawn(move || {
            let mut enigo_opt = match Enigo::new(&Settings::default()) {
                Ok(e) => {
                    let note = if cfg!(target_os = "windows") {
                        "Ready (native Windows SendInput active)".to_string()
                    } else {
                        "Ready (OS input active)".to_string()
                    };
                    let _ = init_tx.send((true, note));
                    Some(e)
                }
                Err(err) => {
                    let _ = init_tx.send((false, format!("Input engine unavailable: {}", err)));
                    None
                }
            };

            while let Ok(op) = rx.recv() {
                if let Some(enigo) = enigo_opt.as_mut() {
                    execute_op(enigo, op);
                } else if let Ok(mut e) = Enigo::new(&Settings::default()) {
                    execute_op(&mut e, op);
                    enigo_opt = Some(e);
                }
            }
        });

        let (ready, note) = init_rx
            .recv()
            .unwrap_or((false, "Input worker failed to start".to_string()));

        (Self { tx }, ready, note)
    }

    #[cfg(target_os = "macos")]
    pub fn send(&self, op: InputOp) {
        let _ = self.app_handle.run_on_main_thread(move || {
            MAIN_THREAD_ENIGO.with(|cell| {
                let mut borrow = cell.borrow_mut();
                if borrow.is_none() {
                    *borrow = Enigo::new(&Settings::default()).ok();
                }
                if let Some(enigo) = borrow.as_mut() {
                    execute_op(enigo, op);
                }
            });
        });
    }

    #[cfg(not(target_os = "macos"))]
    pub fn send(&self, op: InputOp) {
        let _ = self.tx.send(op);
    }
}

fn execute_op(enigo: &mut Enigo, op: InputOp) {
    match op {
        InputOp::Nav(action) => match action {
            NavAction::Up => {
                let _ = enigo.key(Key::UpArrow, Direction::Click);
            }
            NavAction::Down => {
                let _ = enigo.key(Key::DownArrow, Direction::Click);
            }
            NavAction::Left => {
                let _ = enigo.key(Key::LeftArrow, Direction::Click);
            }
            NavAction::Right => {
                let _ = enigo.key(Key::RightArrow, Direction::Click);
            }
            NavAction::Ok => {
                let _ = enigo.key(Key::Return, Direction::Click);
            }
            NavAction::Back => {
                let _ = enigo.key(Key::Escape, Direction::Click);
            }
            NavAction::BrowserBack => {
                #[cfg(target_os = "macos")]
                {
                    let _ = enigo.key(Key::Meta, Direction::Press);
                    let _ = enigo.key(Key::LeftArrow, Direction::Click);
                    let _ = enigo.key(Key::Meta, Direction::Release);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = enigo.key(Key::Alt, Direction::Press);
                    let _ = enigo.key(Key::LeftArrow, Direction::Click);
                    let _ = enigo.key(Key::Alt, Direction::Release);
                }
            }
            NavAction::Fullscreen => {
                let _ = enigo.key(Key::Unicode('f'), Direction::Click);
            }
            NavAction::Home => {}
        },
        InputOp::Trackpad { action, dx, dy } => match action {
            TrackpadAction::Move => {
                let _ = enigo.move_mouse(dx, dy, Coordinate::Rel);
            }
            TrackpadAction::Click => {
                let _ = enigo.button(Button::Left, Direction::Click);
            }
            TrackpadAction::RightClick => {
                let _ = enigo.button(Button::Right, Direction::Click);
            }
            TrackpadAction::DoubleClick => {
                let _ = enigo.button(Button::Left, Direction::Click);
                let _ = enigo.button(Button::Left, Direction::Click);
            }
            TrackpadAction::Scroll => {
                if dy != 0 {
                    let _ = enigo.scroll(dy, Axis::Vertical);
                }
                if dx != 0 {
                    let _ = enigo.scroll(dx, Axis::Horizontal);
                }
            }
        },
        InputOp::Text {
            mode,
            value,
            submit,
        } => match mode {
            TextMode::Type => {
                for ch in value.chars() {
                    match ch {
                        ' ' => {
                            let _ = enigo.key(Key::Space, Direction::Click);
                        }
                        '\n' | '\r' => {
                            let _ = enigo.key(Key::Return, Direction::Click);
                        }
                        '\t' => {
                            let _ = enigo.key(Key::Tab, Direction::Click);
                        }
                        'A'..='Z' => {
                            let lower = ch.to_ascii_lowercase();
                            let _ = enigo.key(Key::Shift, Direction::Press);
                            let _ = enigo.key(Key::Unicode(lower), Direction::Click);
                            let _ = enigo.key(Key::Shift, Direction::Release);
                        }
                        c if c.is_ascii_lowercase() || c.is_ascii_digit() => {
                            let _ = enigo.key(Key::Unicode(c), Direction::Click);
                        }
                        other => {
                            let mut buf = [0u8; 4];
                            let _ = enigo.text(other.encode_utf8(&mut buf));
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(18));
                }
                if submit {
                    let _ = enigo.key(Key::Return, Direction::Click);
                }
            }
            TextMode::Key => match value.as_str() {
                "backspace" => {
                    let _ = enigo.key(Key::Backspace, Direction::Click);
                }
                "enter" => {
                    let _ = enigo.key(Key::Return, Direction::Click);
                }
                "space" => {
                    let _ = enigo.key(Key::Space, Direction::Click);
                }
                "escape" => {
                    let _ = enigo.key(Key::Escape, Direction::Click);
                }
                "tab" => {
                    let _ = enigo.key(Key::Tab, Direction::Click);
                }
                "clear_all" => {
                    #[cfg(target_os = "macos")]
                    let modifier = Key::Meta;
                    #[cfg(not(target_os = "macos"))]
                    let modifier = Key::Control;

                    let _ = enigo.key(modifier, Direction::Press);
                    let _ = enigo.key(Key::Unicode('a'), Direction::Click);
                    let _ = enigo.key(modifier, Direction::Release);
                    let _ = enigo.key(Key::Backspace, Direction::Click);
                }
                _ => {
                    if !value.is_empty() {
                        let _ = enigo.text(&value);
                    }
                }
            },
        },
        InputOp::Media(action) => match action {
            MediaAction::PlayPause => {
                let _ = enigo.key(Key::Space, Direction::Click);
            }
            MediaAction::SeekBack => {
                let _ = enigo.key(Key::LeftArrow, Direction::Click);
            }
            MediaAction::SeekForward => {
                let _ = enigo.key(Key::RightArrow, Direction::Click);
            }
            MediaAction::Mute => system_mute(enigo),
            MediaAction::VolUp => system_volume_step(enigo, true),
            MediaAction::VolDown => system_volume_step(enigo, false),
        },
    }
}

#[cfg(target_os = "macos")]
fn system_mute(_enigo: &mut Enigo) {
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            "set volume output muted not (output muted of (get volume settings))",
        ])
        .spawn();
}

#[cfg(target_os = "macos")]
fn system_volume_step(_enigo: &mut Enigo, up: bool) {
    let expr = if up {
        "set volume output volume ((output volume of (get volume settings)) + 6)"
    } else {
        "set volume output volume ((output volume of (get volume settings)) - 6)"
    };
    let _ = std::process::Command::new("osascript")
        .args(["-e", expr])
        .spawn();
}

#[cfg(not(target_os = "macos"))]
fn system_mute(enigo: &mut Enigo) {
    let _ = enigo.key(Key::VolumeMute, Direction::Click);
}

#[cfg(not(target_os = "macos"))]
fn system_volume_step(enigo: &mut Enigo, up: bool) {
    let key = if up { Key::VolumeUp } else { Key::VolumeDown };
    let _ = enigo.key(key, Direction::Click);
}
