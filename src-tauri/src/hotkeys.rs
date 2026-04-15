use crate::ClickerState;
#[cfg(target_os = "windows")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
#[cfg(target_os = "windows")]
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, MSG, WH_MOUSE_LL, WM_MOUSEWHEEL,
};

use crate::engine::worker::now_epoch_ms;
use crate::engine::worker::start_clicker_inner;
use crate::engine::worker::stop_clicker_inner;
use crate::engine::worker::toggle_clicker_inner;

#[cfg(target_os = "windows")]
pub const VK_SCROLL_UP_PSEUDO: i32 = -1;
#[cfg(target_os = "windows")]
pub const VK_SCROLL_DOWN_PSEUDO: i32 = -2;

#[cfg(target_os = "windows")]
static SCROLL_UP_AT: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "windows")]
static SCROLL_DOWN_AT: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "windows")]
const SCROLL_WINDOW_MS: u64 = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyMainKey {
    Keyboard(Code),
    #[cfg(target_os = "windows")]
    WindowsVk(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
    pub main_key: HotkeyMainKey,
    pub key_token: String,
}

impl HotkeyBinding {
    pub fn shortcut(&self) -> Option<Shortcut> {
        let code = match &self.main_key {
            HotkeyMainKey::Keyboard(code) => *code,
            #[cfg(target_os = "windows")]
            HotkeyMainKey::WindowsVk(_) => return None,
        };

        let mut modifiers = Modifiers::empty();

        if self.ctrl {
            modifiers |= Modifiers::CONTROL;
        }
        if self.alt {
            modifiers |= Modifiers::ALT;
        }
        if self.shift {
            modifiers |= Modifiers::SHIFT;
        }
        if self.super_key {
            modifiers |= Modifiers::SUPER;
        }

        Some(Shortcut::new(Some(modifiers), code))
    }

    #[cfg(target_os = "windows")]
    pub fn uses_manual_listener(&self) -> bool {
        matches!(&self.main_key, HotkeyMainKey::WindowsVk(_))
    }
}

pub fn register_hotkey_inner(app: &AppHandle, hotkey: String) -> Result<String, String> {
    if hotkey.trim().is_empty() {
        let previous = {
            let state = app.state::<ClickerState>();
            let previous = state.registered_hotkey.lock().unwrap().take();
            previous
        };

        if let Some(previous_shortcut) = previous.and_then(|binding| binding.shortcut()) {
            let _ = app.global_shortcut().unregister(previous_shortcut);
        }

        let state = app.state::<ClickerState>();
        state.suppress_hotkey_until_ms.store(0, Ordering::SeqCst);
        state
            .suppress_hotkey_until_release
            .store(false, Ordering::SeqCst);
        return Ok(String::new());
    }

    let binding = parse_hotkey_binding(&hotkey)?;
    let previous = {
        let state = app.state::<ClickerState>();
        let previous = state.registered_hotkey.lock().unwrap().clone();
        previous
    };

    if previous.as_ref() != Some(&binding) {
        if let Some(previous_shortcut) = previous.as_ref().and_then(HotkeyBinding::shortcut) {
            let _ = app.global_shortcut().unregister(previous_shortcut);
        }

        if let Err(error) = bind_shortcut(app, &binding) {
            if let Some(previous_binding) = previous.as_ref() {
                let _ = bind_shortcut(app, previous_binding);
            }
            return Err(error);
        }
    }

    let state = app.state::<ClickerState>();
    state
        .suppress_hotkey_until_ms
        .store(now_epoch_ms().saturating_add(250), Ordering::SeqCst);
    state
        .suppress_hotkey_until_release
        .store(true, Ordering::SeqCst);
    *state.registered_hotkey.lock().unwrap() = Some(binding.clone());

    Ok(format_hotkey_binding(&binding))
}

fn bind_shortcut(app: &AppHandle, binding: &HotkeyBinding) -> Result<(), String> {
    let Some(shortcut) = binding.shortcut() else {
        return Ok(());
    };

    app.global_shortcut()
        .on_shortcut(shortcut, move |app_handle, _shortcut, event| {
            handle_shortcut_event(app_handle, event.state);
        })
        .map_err(|e| e.to_string())
}

fn handle_shortcut_event(app: &AppHandle, event_state: ShortcutState) {
    let state = app.state::<ClickerState>();

    if matches!(event_state, ShortcutState::Released)
        && state.suppress_hotkey_until_release.load(Ordering::SeqCst)
    {
        state
            .suppress_hotkey_until_release
            .store(false, Ordering::SeqCst);
        return;
    }

    if state.hotkey_capture_active.load(Ordering::SeqCst) {
        return;
    }

    match event_state {
        ShortcutState::Pressed => {
            if state.suppress_hotkey_until_release.load(Ordering::SeqCst) {
                return;
            }

            let suppress_until = state.suppress_hotkey_until_ms.load(Ordering::SeqCst);
            if now_epoch_ms() < suppress_until {
                return;
            }

            handle_hotkey_pressed(app);
        }
        ShortcutState::Released => {
            handle_hotkey_released(app);
        }
    }
}

pub fn normalize_hotkey(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .replace("control", "ctrl")
        .replace("command", "super")
        .replace("meta", "super")
        .replace("win", "super")
}

pub fn parse_hotkey_binding(hotkey: &str) -> Result<HotkeyBinding, String> {
    let normalized = normalize_hotkey(hotkey);
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut super_key = false;
    let mut main_key: Option<(HotkeyMainKey, String)> = None;

    for token in normalized.split('+').map(str::trim) {
        if token.is_empty() {
            return Err(format!("Invalid hotkey '{hotkey}': found empty key token"));
        }

        match token {
            "alt" | "option" => alt = true,
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "super" | "command" | "cmd" | "meta" | "win" => super_key = true,
            _ => {
                if main_key
                    .replace(parse_hotkey_main_key(token, hotkey)?)
                    .is_some()
                {
                    return Err(format!(
                        "Invalid hotkey '{hotkey}': use modifiers first and only one main key"
                    ));
                }
            }
        }
    }

    let (main_key, key_token) =
        main_key.ok_or_else(|| format!("Invalid hotkey '{hotkey}': missing main key"))?;

    Ok(HotkeyBinding {
        ctrl,
        alt,
        shift,
        super_key,
        main_key,
        key_token,
    })
}

pub fn parse_hotkey_main_key(
    token: &str,
    original_hotkey: &str,
) -> Result<(HotkeyMainKey, String), String> {
    let lower = token.trim().to_lowercase();

    #[cfg(target_os = "windows")]
    let mapped = match lower.as_str() {
        "mouseleft" | "mouse1" => Some((
            HotkeyMainKey::WindowsVk(VK_LBUTTON as i32),
            String::from("mouseleft"),
        )),
        "mouseright" | "mouse2" => Some((
            HotkeyMainKey::WindowsVk(VK_RBUTTON as i32),
            String::from("mouseright"),
        )),
        "mousemiddle" | "mouse3" | "scrollbutton" | "middleclick" => Some((
            HotkeyMainKey::WindowsVk(VK_MBUTTON as i32),
            String::from("mousemiddle"),
        )),
        "mouse4" | "mouseback" | "xbutton1" => Some((
            HotkeyMainKey::WindowsVk(VK_XBUTTON1 as i32),
            String::from("mouse4"),
        )),
        "mouse5" | "mouseforward" | "xbutton2" => Some((
            HotkeyMainKey::WindowsVk(VK_XBUTTON2 as i32),
            String::from("mouse5"),
        )),
        "scrollup" | "wheelup" => Some((
            HotkeyMainKey::WindowsVk(VK_SCROLL_UP_PSEUDO),
            String::from("scrollup"),
        )),
        "scrolldown" | "wheeldown" => Some((
            HotkeyMainKey::WindowsVk(VK_SCROLL_DOWN_PSEUDO),
            String::from("scrolldown"),
        )),
        "<" | ">" | "intlbackslash" | "oem102" | "nonusbackslash" => Some((
            HotkeyMainKey::Keyboard(Code::IntlBackslash),
            String::from("IntlBackslash"),
        )),
        "space" | "spacebar" => Some((HotkeyMainKey::Keyboard(Code::Space), String::from("space"))),
        "tab" => Some((HotkeyMainKey::Keyboard(Code::Tab), String::from("tab"))),
        "enter" => Some((HotkeyMainKey::Keyboard(Code::Enter), String::from("enter"))),
        "backspace" => Some((
            HotkeyMainKey::Keyboard(Code::Backspace),
            String::from("backspace"),
        )),
        "delete" => Some((
            HotkeyMainKey::Keyboard(Code::Delete),
            String::from("delete"),
        )),
        "insert" => Some((
            HotkeyMainKey::Keyboard(Code::Insert),
            String::from("insert"),
        )),
        "home" => Some((HotkeyMainKey::Keyboard(Code::Home), String::from("home"))),
        "end" => Some((HotkeyMainKey::Keyboard(Code::End), String::from("end"))),
        "pageup" => Some((
            HotkeyMainKey::Keyboard(Code::PageUp),
            String::from("pageup"),
        )),
        "pagedown" => Some((
            HotkeyMainKey::Keyboard(Code::PageDown),
            String::from("pagedown"),
        )),
        "up" => Some((HotkeyMainKey::Keyboard(Code::ArrowUp), String::from("up"))),
        "down" => Some((
            HotkeyMainKey::Keyboard(Code::ArrowDown),
            String::from("down"),
        )),
        "left" => Some((
            HotkeyMainKey::Keyboard(Code::ArrowLeft),
            String::from("left"),
        )),
        "right" => Some((
            HotkeyMainKey::Keyboard(Code::ArrowRight),
            String::from("right"),
        )),
        "esc" | "escape" => Some((
            HotkeyMainKey::Keyboard(Code::Escape),
            String::from("escape"),
        )),
        "/" | "slash" => Some((HotkeyMainKey::Keyboard(Code::Slash), String::from("/"))),
        "\\" | "backslash" => Some((HotkeyMainKey::Keyboard(Code::Backslash), String::from("\\"))),
        ";" | "semicolon" => Some((HotkeyMainKey::Keyboard(Code::Semicolon), String::from(";"))),
        "'" | "quote" => Some((HotkeyMainKey::Keyboard(Code::Quote), String::from("'"))),
        "[" | "bracketleft" => Some((
            HotkeyMainKey::Keyboard(Code::BracketLeft),
            String::from("["),
        )),
        "]" | "bracketright" => Some((
            HotkeyMainKey::Keyboard(Code::BracketRight),
            String::from("]"),
        )),
        "-" | "minus" => Some((HotkeyMainKey::Keyboard(Code::Minus), String::from("-"))),
        "=" | "equal" => Some((HotkeyMainKey::Keyboard(Code::Equal), String::from("="))),
        "`" | "backquote" => Some((HotkeyMainKey::Keyboard(Code::Backquote), String::from("`"))),
        "," | "comma" => Some((HotkeyMainKey::Keyboard(Code::Comma), String::from(","))),
        "." | "period" => Some((HotkeyMainKey::Keyboard(Code::Period), String::from("."))),
        _ => None,
    };

    #[cfg(not(target_os = "windows"))]
    let mapped = match lower.as_str() {
        "mouseleft" | "mouse1" | "mouseright" | "mouse2" | "mousemiddle" | "mouse3"
        | "scrollbutton" | "middleclick" | "mouse4" | "mouseback" | "xbutton1" | "mouse5"
        | "mouseforward" | "xbutton2" | "scrollup" | "wheelup" | "scrolldown" | "wheeldown" => {
            return Err(format!(
                "The hotkey '{token}' in '{original_hotkey}' is currently only supported on Windows."
            ));
        }
        "<" | ">" | "intlbackslash" | "oem102" | "nonusbackslash" => Some((
            HotkeyMainKey::Keyboard(Code::IntlBackslash),
            String::from("IntlBackslash"),
        )),
        "space" | "spacebar" => Some((HotkeyMainKey::Keyboard(Code::Space), String::from("space"))),
        "tab" => Some((HotkeyMainKey::Keyboard(Code::Tab), String::from("tab"))),
        "enter" => Some((HotkeyMainKey::Keyboard(Code::Enter), String::from("enter"))),
        "backspace" => Some((
            HotkeyMainKey::Keyboard(Code::Backspace),
            String::from("backspace"),
        )),
        "delete" => Some((
            HotkeyMainKey::Keyboard(Code::Delete),
            String::from("delete"),
        )),
        "insert" => Some((
            HotkeyMainKey::Keyboard(Code::Insert),
            String::from("insert"),
        )),
        "home" => Some((HotkeyMainKey::Keyboard(Code::Home), String::from("home"))),
        "end" => Some((HotkeyMainKey::Keyboard(Code::End), String::from("end"))),
        "pageup" => Some((
            HotkeyMainKey::Keyboard(Code::PageUp),
            String::from("pageup"),
        )),
        "pagedown" => Some((
            HotkeyMainKey::Keyboard(Code::PageDown),
            String::from("pagedown"),
        )),
        "up" => Some((HotkeyMainKey::Keyboard(Code::ArrowUp), String::from("up"))),
        "down" => Some((
            HotkeyMainKey::Keyboard(Code::ArrowDown),
            String::from("down"),
        )),
        "left" => Some((
            HotkeyMainKey::Keyboard(Code::ArrowLeft),
            String::from("left"),
        )),
        "right" => Some((
            HotkeyMainKey::Keyboard(Code::ArrowRight),
            String::from("right"),
        )),
        "esc" | "escape" => Some((
            HotkeyMainKey::Keyboard(Code::Escape),
            String::from("escape"),
        )),
        "/" | "slash" => Some((HotkeyMainKey::Keyboard(Code::Slash), String::from("/"))),
        "\\" | "backslash" => Some((HotkeyMainKey::Keyboard(Code::Backslash), String::from("\\"))),
        ";" | "semicolon" => Some((HotkeyMainKey::Keyboard(Code::Semicolon), String::from(";"))),
        "'" | "quote" => Some((HotkeyMainKey::Keyboard(Code::Quote), String::from("'"))),
        "[" | "bracketleft" => Some((
            HotkeyMainKey::Keyboard(Code::BracketLeft),
            String::from("["),
        )),
        "]" | "bracketright" => Some((
            HotkeyMainKey::Keyboard(Code::BracketRight),
            String::from("]"),
        )),
        "-" | "minus" => Some((HotkeyMainKey::Keyboard(Code::Minus), String::from("-"))),
        "=" | "equal" => Some((HotkeyMainKey::Keyboard(Code::Equal), String::from("="))),
        "`" | "backquote" => Some((HotkeyMainKey::Keyboard(Code::Backquote), String::from("`"))),
        "," | "comma" => Some((HotkeyMainKey::Keyboard(Code::Comma), String::from(","))),
        "." | "period" => Some((HotkeyMainKey::Keyboard(Code::Period), String::from("."))),
        _ => None,
    };

    if let Some(binding) = mapped {
        return Ok(binding);
    }

    if lower.starts_with('f') && lower.len() <= 3 {
        if let Ok(number) = lower[1..].parse::<u8>() {
            if let Some(code) = function_key_code(number) {
                return Ok((HotkeyMainKey::Keyboard(code), lower));
            }
        }
    }

    if let Some(letter) = lower.strip_prefix("key") {
        if letter.len() == 1 {
            return parse_hotkey_main_key(letter, original_hotkey);
        }
    }

    if let Some(digit) = lower.strip_prefix("digit") {
        if digit.len() == 1 {
            return parse_hotkey_main_key(digit, original_hotkey);
        }
    }

    if lower.len() == 1 {
        let ch = lower.as_bytes()[0];
        if ch.is_ascii_lowercase() {
            return Ok((HotkeyMainKey::Keyboard(letter_code(ch)), lower));
        }

        if ch.is_ascii_digit() {
            return Ok((HotkeyMainKey::Keyboard(digit_code(ch)), lower));
        }
    }

    Err(format!(
        "Couldn't recognize '{token}' as a valid key in '{original_hotkey}'"
    ))
}

fn function_key_code(number: u8) -> Option<Code> {
    match number {
        1 => Some(Code::F1),
        2 => Some(Code::F2),
        3 => Some(Code::F3),
        4 => Some(Code::F4),
        5 => Some(Code::F5),
        6 => Some(Code::F6),
        7 => Some(Code::F7),
        8 => Some(Code::F8),
        9 => Some(Code::F9),
        10 => Some(Code::F10),
        11 => Some(Code::F11),
        12 => Some(Code::F12),
        13 => Some(Code::F13),
        14 => Some(Code::F14),
        15 => Some(Code::F15),
        16 => Some(Code::F16),
        17 => Some(Code::F17),
        18 => Some(Code::F18),
        19 => Some(Code::F19),
        20 => Some(Code::F20),
        21 => Some(Code::F21),
        22 => Some(Code::F22),
        23 => Some(Code::F23),
        24 => Some(Code::F24),
        _ => None,
    }
}

fn letter_code(ch: u8) -> Code {
    match ch {
        b'a' => Code::KeyA,
        b'b' => Code::KeyB,
        b'c' => Code::KeyC,
        b'd' => Code::KeyD,
        b'e' => Code::KeyE,
        b'f' => Code::KeyF,
        b'g' => Code::KeyG,
        b'h' => Code::KeyH,
        b'i' => Code::KeyI,
        b'j' => Code::KeyJ,
        b'k' => Code::KeyK,
        b'l' => Code::KeyL,
        b'm' => Code::KeyM,
        b'n' => Code::KeyN,
        b'o' => Code::KeyO,
        b'p' => Code::KeyP,
        b'q' => Code::KeyQ,
        b'r' => Code::KeyR,
        b's' => Code::KeyS,
        b't' => Code::KeyT,
        b'u' => Code::KeyU,
        b'v' => Code::KeyV,
        b'w' => Code::KeyW,
        b'x' => Code::KeyX,
        b'y' => Code::KeyY,
        b'z' => Code::KeyZ,
        _ => unreachable!(),
    }
}

fn digit_code(ch: u8) -> Code {
    match ch {
        b'0' => Code::Digit0,
        b'1' => Code::Digit1,
        b'2' => Code::Digit2,
        b'3' => Code::Digit3,
        b'4' => Code::Digit4,
        b'5' => Code::Digit5,
        b'6' => Code::Digit6,
        b'7' => Code::Digit7,
        b'8' => Code::Digit8,
        b'9' => Code::Digit9,
        _ => unreachable!(),
    }
}

pub fn format_hotkey_binding(binding: &HotkeyBinding) -> String {
    let mut parts: Vec<String> = Vec::new();

    if binding.ctrl {
        parts.push(String::from("ctrl"));
    }
    if binding.alt {
        parts.push(String::from("alt"));
    }
    if binding.shift {
        parts.push(String::from("shift"));
    }
    if binding.super_key {
        parts.push(String::from("super"));
    }

    parts.push(binding.key_token.clone());
    parts.join("+")
}

#[cfg(not(target_os = "windows"))]
pub fn start_hotkey_listener(_app: AppHandle) {}

#[cfg(target_os = "windows")]
pub fn start_hotkey_listener(app: AppHandle) {
    std::thread::spawn(move || {
        let mut was_pressed = false;

        loop {
            let binding = {
                let state = app.state::<ClickerState>();
                let binding = state.registered_hotkey.lock().unwrap().clone();
                binding
            };

            let uses_manual_listener = binding
                .as_ref()
                .map(HotkeyBinding::uses_manual_listener)
                .unwrap_or(false);

            if !uses_manual_listener {
                was_pressed = false;
                std::thread::sleep(Duration::from_millis(12));
                continue;
            }

            let currently_pressed = binding
                .as_ref()
                .map(is_hotkey_binding_pressed)
                .unwrap_or(false);

            let suppress_until = app
                .state::<ClickerState>()
                .suppress_hotkey_until_ms
                .load(Ordering::SeqCst);
            let suppress_until_release = app
                .state::<ClickerState>()
                .suppress_hotkey_until_release
                .load(Ordering::SeqCst);
            let hotkey_capture_active = app
                .state::<ClickerState>()
                .hotkey_capture_active
                .load(Ordering::SeqCst);

            if hotkey_capture_active {
                was_pressed = currently_pressed;
                std::thread::sleep(Duration::from_millis(12));
                continue;
            }

            if suppress_until_release {
                if currently_pressed {
                    was_pressed = true;
                    std::thread::sleep(Duration::from_millis(12));
                    continue;
                }

                app.state::<ClickerState>()
                    .suppress_hotkey_until_release
                    .store(false, Ordering::SeqCst);
                was_pressed = false;
                std::thread::sleep(Duration::from_millis(12));
                continue;
            }

            if now_epoch_ms() < suppress_until {
                was_pressed = currently_pressed;
                std::thread::sleep(Duration::from_millis(12));
                continue;
            }

            if currently_pressed && !was_pressed {
                handle_hotkey_pressed(&app);
            } else if !currently_pressed && was_pressed {
                handle_hotkey_released(&app);
            }

            was_pressed = currently_pressed;
            std::thread::sleep(Duration::from_millis(12));
        }
    });
}

pub fn handle_hotkey_pressed(app: &AppHandle) {
    let mode = {
        let state = app.state::<ClickerState>();
        let mode = state.settings.lock().unwrap().mode.clone();
        mode
    };

    if mode == "Toggle" {
        let _ = toggle_clicker_inner(app);
    } else if mode == "Hold" {
        let _ = start_clicker_inner(app);
    }
}

pub fn handle_hotkey_released(app: &AppHandle) {
    let mode = {
        let state = app.state::<ClickerState>();
        let mode = state.settings.lock().unwrap().mode.clone();
        mode
    };

    if mode == "Hold" {
        let _ = stop_clicker_inner(app, Some(String::from("Stopped from hold hotkey")));
    }
}

#[cfg(target_os = "windows")]
pub fn is_hotkey_binding_pressed(binding: &HotkeyBinding) -> bool {
    let main_vk = match &binding.main_key {
        HotkeyMainKey::Keyboard(_) => return false,
        HotkeyMainKey::WindowsVk(vk) => *vk,
    };

    let ctrl_down = is_vk_down(VK_CONTROL as i32);
    let alt_down = is_vk_down(VK_MENU as i32);
    let shift_down = is_vk_down(VK_SHIFT as i32);
    let super_down = is_vk_down(VK_LWIN as i32) || is_vk_down(VK_RWIN as i32);

    if ctrl_down != binding.ctrl
        || alt_down != binding.alt
        || shift_down != binding.shift
        || super_down != binding.super_key
    {
        return false;
    }

    is_main_key_active(main_vk)
}

#[cfg(target_os = "windows")]
fn is_main_key_active(vk: i32) -> bool {
    match vk {
        VK_SCROLL_UP_PSEUDO => {
            let ts = SCROLL_UP_AT.load(Ordering::SeqCst);
            if ts == 0 {
                return false;
            }
            let now = now_epoch_ms();
            now.saturating_sub(ts) < SCROLL_WINDOW_MS
        }
        VK_SCROLL_DOWN_PSEUDO => {
            let ts = SCROLL_DOWN_AT.load(Ordering::SeqCst);
            if ts == 0 {
                return false;
            }
            let now = now_epoch_ms();
            now.saturating_sub(ts) < SCROLL_WINDOW_MS
        }
        _ => is_vk_down(vk),
    }
}

#[cfg(target_os = "windows")]
pub fn is_vk_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

#[cfg(not(target_os = "windows"))]
pub fn start_scroll_hook() {}

#[cfg(target_os = "windows")]
pub fn start_scroll_hook() {
    std::thread::spawn(|| unsafe {
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), 0, 0);
        if hook == 0 {
            log::error!("[Hotkeys] Failed to install WH_MOUSE_LL hook");
            return;
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {}
    });
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn mouse_hook_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && w_param == WM_MOUSEWHEEL as usize {
        #[allow(clippy::upper_case_acronyms)]
        #[repr(C)]
        struct MSLLHOOKSTRUCT {
            pt_x: i32,
            pt_y: i32,
            mouse_data: u32,
            flags: u32,
            time: u32,
            extra_info: usize,
        }

        let info = &*(l_param as *const MSLLHOOKSTRUCT);
        let delta = (info.mouse_data >> 16) as i16;
        let now = now_epoch_ms();
        if delta > 0 {
            SCROLL_UP_AT.store(now, Ordering::SeqCst);
        } else if delta < 0 {
            SCROLL_DOWN_AT.store(now, Ordering::SeqCst);
        }
    }

    CallNextHookEx(0, code, w_param, l_param)
}
