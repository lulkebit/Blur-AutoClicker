use crate::engine::worker::now_epoch_ms;
use crate::engine::worker::start_clicker_inner;
use crate::engine::worker::stop_clicker_inner;
use crate::engine::worker::toggle_clicker_inner;
use crate::ClickerState;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicBool, AtomicU64};
#[cfg(target_os = "windows")]
use std::time::Duration;
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
#[cfg(target_os = "windows")]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, MSG,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_MOUSEWHEEL, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[cfg(target_os = "windows")]
pub const VK_SCROLL_UP_PSEUDO: i32 = -1;
#[cfg(target_os = "windows")]
pub const VK_SCROLL_DOWN_PSEUDO: i32 = -2;
#[cfg(target_os = "windows")]
pub const VK_NUMPAD_ENTER_PSEUDO: i32 = -3;

#[cfg(target_os = "windows")]
static SCROLL_UP_AT: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "windows")]
static SCROLL_DOWN_AT: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "windows")]
static NUMPAD_ENTER_DOWN: AtomicBool = AtomicBool::new(false);

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

pub fn keyboard_key_code_from_binding(binding: &HotkeyBinding) -> Option<u16> {
    keyboard_key_token_to_vk(&binding.key_token)
}

pub fn parse_keyboard_key_code(key: &str) -> Result<u16, String> {
    let (_, key_token) = parse_hotkey_main_key(key, key)?;
    keyboard_key_token_to_vk(&key_token)
        .ok_or_else(|| format!("'{}' is not a keyboard key", key.trim()))
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
        "numpadenter" => Some((
            HotkeyMainKey::WindowsVk(VK_NUMPAD_ENTER_PSEUDO),
            String::from("numpadenter"),
        )),
        "numpad0" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad0),
            String::from("numpad0"),
        )),
        "numpad1" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad1),
            String::from("numpad1"),
        )),
        "numpad2" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad2),
            String::from("numpad2"),
        )),
        "numpad3" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad3),
            String::from("numpad3"),
        )),
        "numpad4" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad4),
            String::from("numpad4"),
        )),
        "numpad5" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad5),
            String::from("numpad5"),
        )),
        "numpad6" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad6),
            String::from("numpad6"),
        )),
        "numpad7" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad7),
            String::from("numpad7"),
        )),
        "numpad8" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad8),
            String::from("numpad8"),
        )),
        "numpad9" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad9),
            String::from("numpad9"),
        )),
        "numpadadd" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadAdd),
            String::from("numpadadd"),
        )),
        "numpadsubtract" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadSubtract),
            String::from("numpadsubtract"),
        )),
        "numpadmultiply" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadMultiply),
            String::from("numpadmultiply"),
        )),
        "numpaddivide" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadDivide),
            String::from("numpaddivide"),
        )),
        "numpaddecimal" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadDecimal),
            String::from("numpaddecimal"),
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
        "numpad0" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad0),
            String::from("numpad0"),
        )),
        "numpad1" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad1),
            String::from("numpad1"),
        )),
        "numpad2" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad2),
            String::from("numpad2"),
        )),
        "numpad3" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad3),
            String::from("numpad3"),
        )),
        "numpad4" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad4),
            String::from("numpad4"),
        )),
        "numpad5" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad5),
            String::from("numpad5"),
        )),
        "numpad6" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad6),
            String::from("numpad6"),
        )),
        "numpad7" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad7),
            String::from("numpad7"),
        )),
        "numpad8" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad8),
            String::from("numpad8"),
        )),
        "numpad9" => Some((
            HotkeyMainKey::Keyboard(Code::Numpad9),
            String::from("numpad9"),
        )),
        "numpadadd" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadAdd),
            String::from("numpadadd"),
        )),
        "numpadsubtract" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadSubtract),
            String::from("numpadsubtract"),
        )),
        "numpadmultiply" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadMultiply),
            String::from("numpadmultiply"),
        )),
        "numpaddivide" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadDivide),
            String::from("numpaddivide"),
        )),
        "numpaddecimal" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadDecimal),
            String::from("numpaddecimal"),
        )),
        "numpadenter" => Some((
            HotkeyMainKey::Keyboard(Code::NumpadEnter),
            String::from("numpadenter"),
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

fn keyboard_key_token_to_vk(token: &str) -> Option<u16> {
    let lower = token.trim().to_ascii_lowercase();

    match lower.as_str() {
        "backspace" => Some(0x08),
        "tab" => Some(0x09),
        "enter" | "numpadenter" => Some(0x0D),
        "pause" => Some(0x13),
        "capslock" => Some(0x14),
        "escape" => Some(0x1B),
        "space" => Some(0x20),
        "pageup" => Some(0x21),
        "pagedown" => Some(0x22),
        "end" => Some(0x23),
        "home" => Some(0x24),
        "left" => Some(0x25),
        "up" => Some(0x26),
        "right" => Some(0x27),
        "down" => Some(0x28),
        "printscreen" => Some(0x2C),
        "insert" => Some(0x2D),
        "delete" => Some(0x2E),
        "menu" => Some(0x5D),
        "numlock" => Some(0x90),
        "scrolllock" => Some(0x91),
        "numpadadd" => Some(0x6B),
        "numpadsubtract" => Some(0x6D),
        "numpadmultiply" => Some(0x6A),
        "numpaddivide" => Some(0x6F),
        "numpaddecimal" => Some(0x6E),
        ";" => Some(0xBA),
        "=" => Some(0xBB),
        "," => Some(0xBC),
        "-" => Some(0xBD),
        "." => Some(0xBE),
        "/" => Some(0xBF),
        "`" => Some(0xC0),
        "[" => Some(0xDB),
        "\\" => Some(0xDC),
        "]" => Some(0xDD),
        "'" => Some(0xDE),
        "intlbackslash" => Some(0xE2),
        _ => {
            if let Some(digit) = lower.strip_prefix("numpad") {
                if digit.len() == 1 {
                    let ch = digit.as_bytes()[0];
                    if ch.is_ascii_digit() {
                        return Some(0x60 + u16::from(ch - b'0'));
                    }
                }
            }

            if lower.starts_with('f') && lower.len() <= 3 {
                if let Ok(number) = lower[1..].parse::<u16>() {
                    if (1..=24).contains(&number) {
                        return Some(0x70 + number - 1);
                    }
                }
            }

            if lower.len() == 1 {
                let ch = lower.as_bytes()[0];
                if ch.is_ascii_lowercase() {
                    return Some(u16::from(ch.to_ascii_uppercase()));
                }
                if ch.is_ascii_digit() {
                    return Some(u16::from(ch));
                }
            }

            None
        }
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
            let (binding, strict) = {
                let state = app.state::<ClickerState>();
                let binding = state.registered_hotkey.lock().unwrap().clone();
                let strict = state.settings.lock().unwrap().strict_hotkey_modifiers;
                (binding, strict)
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
                .map(|binding| is_hotkey_binding_pressed(binding, strict))
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
pub fn is_hotkey_binding_pressed(binding: &HotkeyBinding, strict: bool) -> bool {
    let main_vk = match &binding.main_key {
        HotkeyMainKey::Keyboard(_) => return false,
        HotkeyMainKey::WindowsVk(vk) => *vk,
    };
    let ctrl_down = is_vk_down(VK_CONTROL as i32);
    let alt_down = is_vk_down(VK_MENU as i32);
    let shift_down = is_vk_down(VK_SHIFT as i32);
    let super_down = is_vk_down(VK_LWIN as i32) || is_vk_down(VK_RWIN as i32);

    if !modifiers_match(binding, ctrl_down, alt_down, shift_down, super_down, strict) {
        return false;
    }

    is_main_key_active(main_vk)
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn modifiers_match(
    binding: &HotkeyBinding,
    ctrl_down: bool,
    alt_down: bool,
    shift_down: bool,
    super_down: bool,
    strict: bool,
) -> bool {
    if binding.ctrl && !ctrl_down {
        return false;
    }
    if binding.alt && !alt_down {
        return false;
    }
    if binding.shift && !shift_down {
        return false;
    }
    if binding.super_key && !super_down {
        return false;
    }

    if strict {
        if ctrl_down && !binding.ctrl {
            return false;
        }
        if alt_down && !binding.alt {
            return false;
        }
        if shift_down && !binding.shift {
            return false;
        }
        if super_down && !binding.super_key {
            return false;
        }
    }

    true
}

#[cfg(target_os = "windows")]
/// For normal VKs this uses `GetAsyncKeyState`. Pseudo-VKs use hook-maintained state.
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
        VK_NUMPAD_ENTER_PSEUDO => NUMPAD_ENTER_DOWN.load(Ordering::SeqCst),
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
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), 0, 0);
        if mouse_hook == 0 {
            log::error!("[Hotkeys] Failed to install WH_MOUSE_LL hook");
        }

        let keyboard_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), 0, 0);
        if keyboard_hook == 0 {
            log::error!("[Hotkeys] Failed to install WH_KEYBOARD_LL hook");
        }

        if mouse_hook == 0 && keyboard_hook == 0 {
            return;
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {}
    });
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn keyboard_hook_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 {
        let info = &*(l_param as *const KBDLLHOOKSTRUCT);
        if info.vkCode as i32 == VK_RETURN as i32 && (info.flags & LLKHF_EXTENDED) != 0 {
            match w_param as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    NUMPAD_ENTER_DOWN.store(true, Ordering::SeqCst);
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    NUMPAD_ENTER_DOWN.store(false, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    }

    CallNextHookEx(0, code, w_param, l_param)
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn mouse_hook_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && w_param == WM_MOUSEWHEEL as usize {
        #[repr(C)]
        struct MsllHookStruct {
            pt_x: i32,
            pt_y: i32,
            mouse_data: u32,
            flags: u32,
            time: u32,
            extra_info: usize,
        }

        let info = &*(l_param as *const MsllHookStruct);
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

#[cfg(test)]
mod tests {
    use super::{format_hotkey_binding, modifiers_match, parse_hotkey_binding};

    #[test]
    fn numpad_tokens_round_trip() {
        for token in [
            "numpad0",
            "numpad1",
            "numpad2",
            "numpad3",
            "numpad4",
            "numpad5",
            "numpad6",
            "numpad7",
            "numpad8",
            "numpad9",
            "numpadadd",
            "numpadsubtract",
            "numpadmultiply",
            "numpaddivide",
            "numpaddecimal",
            "numpadenter",
        ] {
            let hotkey = format!("ctrl+shift+{token}");
            let binding = parse_hotkey_binding(&hotkey).expect("token should parse");
            assert_eq!(binding.key_token, token);
            assert_eq!(format_hotkey_binding(&binding), hotkey);
        }
    }

    #[test]
    fn empty_hotkeys_are_rejected() {
        assert!(parse_hotkey_binding("").is_err());
        assert!(parse_hotkey_binding("ctrl+").is_err());
    }

    #[test]
    fn extra_modifiers_do_not_block_hotkeys_in_relaxed_mode() {
        let binding = parse_hotkey_binding("f11").expect("hotkey should parse");
        assert!(modifiers_match(&binding, false, false, true, false, false));
        assert!(modifiers_match(&binding, true, true, true, true, false));
    }

    #[test]
    fn extra_modifiers_block_hotkeys_in_strict_mode() {
        let binding = parse_hotkey_binding("f11").expect("hotkey should parse");
        assert!(!modifiers_match(&binding, false, false, true, false, true));
        assert!(!modifiers_match(&binding, true, true, true, true, true));
        assert!(modifiers_match(&binding, false, false, false, false, true));
    }
}
