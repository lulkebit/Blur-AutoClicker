use super::rng::SmallRng;
use super::worker::{sleep_interruptible, RunControl};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualScreenRect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl VirtualScreenRect {
    #[inline]
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    #[inline]
    pub fn right(self) -> i32 {
        self.left + self.width
    }

    #[inline]
    pub fn bottom(self) -> i32 {
        self.top + self.height
    }

    #[inline]
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right() && y >= self.top && y < self.bottom()
    }

    #[cfg(target_os = "windows")]
    #[inline]
    fn normalize_x(&self, pixel_x: i32) -> i32 {
        let relative_x = pixel_x as f64 - self.left as f64;
        let ratio = relative_x / self.width as f64;
        (ratio * 65535.0).round() as i32
    }

    #[cfg(target_os = "windows")]
    #[inline]
    fn normalize_y(&self, pixel_y: i32) -> i32 {
        let relative_y = pixel_y as f64 - self.top as f64;
        let ratio = relative_y / self.height as f64;
        (ratio * 65535.0).round() as i32
    }

    #[inline]
    pub fn offset_from(self, origin: VirtualScreenRect) -> Self {
        Self::new(
            self.left - origin.left,
            self.top - origin.top,
            self.width,
            self.height,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventSpec {
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
    MiddleDown,
    MiddleUp,
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{MouseEventSpec, VirtualScreenRect};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    pub fn current_cursor_position() -> Option<(i32, i32)> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let mut point = POINT { x: 0, y: 0 };
        let ok = unsafe { GetCursorPos(&mut point) };
        if ok == 0 {
            None
        } else {
            Some((point.x, point.y))
        }
    }

    pub fn current_virtual_screen_rect() -> Option<VirtualScreenRect> {
        let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
        if width <= 0 || height <= 0 {
            return None;
        }

        Some(VirtualScreenRect::new(left, top, width, height))
    }

    pub fn current_monitor_rects() -> Option<Vec<VirtualScreenRect>> {
        use std::ptr;
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, MONITORINFO,
        };

        unsafe extern "system" fn enum_monitor_proc(
            monitor: isize,
            _hdc: isize,
            _clip_rect: *mut RECT,
            user_data: isize,
        ) -> i32 {
            let monitors = &mut *(user_data as *mut Vec<VirtualScreenRect>);
            let mut info = std::mem::zeroed::<MONITORINFO>();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;

            if GetMonitorInfoW(monitor, &mut info as *mut MONITORINFO as *mut _) == 0 {
                return 1;
            }

            let rect = info.rcMonitor;
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width > 0 && height > 0 {
                monitors.push(VirtualScreenRect::new(rect.left, rect.top, width, height));
            }

            1
        }

        let mut monitors = Vec::new();
        let ok = unsafe {
            EnumDisplayMonitors(
                0,
                ptr::null(),
                Some(enum_monitor_proc),
                &mut monitors as *mut Vec<VirtualScreenRect> as isize,
            )
        };

        if ok == 0 || monitors.is_empty() {
            return current_virtual_screen_rect().map(|screen| vec![screen]);
        }

        monitors.sort_by_key(|monitor: &VirtualScreenRect| (monitor.top, monitor.left));
        Some(monitors)
    }

    pub fn move_mouse(target_x: i32, target_y: i32) {
        if let Some(screen_rect) = current_virtual_screen_rect() {
            let end_x = screen_rect.normalize_x(target_x);
            let end_y = screen_rect.normalize_y(target_y);
            let movement = make_movement(end_x, end_y);
            unsafe { SendInput(1, &movement, std::mem::size_of::<INPUT>() as i32) };
            log::debug!("moved cursor x:{end_x}, y:{end_y}");
        }
    }

    fn mouse_event_flag(event: MouseEventSpec) -> u32 {
        match event {
            MouseEventSpec::LeftDown => MOUSEEVENTF_LEFTDOWN,
            MouseEventSpec::LeftUp => MOUSEEVENTF_LEFTUP,
            MouseEventSpec::RightDown => MOUSEEVENTF_RIGHTDOWN,
            MouseEventSpec::RightUp => MOUSEEVENTF_RIGHTUP,
            MouseEventSpec::MiddleDown => MOUSEEVENTF_MIDDLEDOWN,
            MouseEventSpec::MiddleUp => MOUSEEVENTF_MIDDLEUP,
        }
    }

    fn make_input(flags: u32, time: u32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: flags,
                    time,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn make_movement(end_x: i32, end_y: i32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                mi: MOUSEINPUT {
                    dx: end_x,
                    dy: end_y,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    pub fn send_mouse_event(event: MouseEventSpec, _click_state: i64) {
        let input = make_input(mouse_event_flag(event), 0);
        unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
    }

    pub fn send_batch(down: MouseEventSpec, up: MouseEventSpec, n: usize, _hold_ms: u32) {
        let mut inputs: Vec<INPUT> = Vec::with_capacity(n * 2);
        for _ in 0..n {
            inputs.push(make_input(mouse_event_flag(down), 0));
            inputs.push(make_input(mouse_event_flag(up), 0));
        }
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{MouseEventSpec, VirtualScreenRect};
    use core_graphics::display::{CGDisplay, CGPoint};
    use core_graphics::event::{
        CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
    };
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    fn event_source() -> Option<CGEventSource> {
        CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()
    }

    fn current_location() -> Option<CGPoint> {
        let source = event_source()?;
        let event = CGEvent::new(source).ok()?;
        Some(event.location())
    }

    fn display_rect(display: CGDisplay) -> VirtualScreenRect {
        let bounds = display.bounds();
        VirtualScreenRect::new(
            bounds.origin.x.round() as i32,
            bounds.origin.y.round() as i32,
            bounds.size.width.round() as i32,
            bounds.size.height.round() as i32,
        )
    }

    pub fn current_cursor_position() -> Option<(i32, i32)> {
        let point = current_location()?;
        Some((point.x.round() as i32, point.y.round() as i32))
    }

    pub fn current_monitor_rects() -> Option<Vec<VirtualScreenRect>> {
        let mut monitors: Vec<VirtualScreenRect> = CGDisplay::active_displays()
            .ok()?
            .into_iter()
            .map(|id| display_rect(CGDisplay::new(id)))
            .filter(|rect| rect.width > 0 && rect.height > 0)
            .collect();

        if monitors.is_empty() {
            monitors.push(display_rect(CGDisplay::main()));
        }

        monitors.sort_by_key(|monitor| (monitor.top, monitor.left));
        Some(monitors)
    }

    pub fn current_virtual_screen_rect() -> Option<VirtualScreenRect> {
        let monitors = current_monitor_rects()?;
        let left = monitors.iter().map(|m| m.left).min()?;
        let top = monitors.iter().map(|m| m.top).min()?;
        let right = monitors.iter().map(|m| m.right()).max()?;
        let bottom = monitors.iter().map(|m| m.bottom()).max()?;
        Some(VirtualScreenRect::new(
            left,
            top,
            right - left,
            bottom - top,
        ))
    }

    pub fn move_mouse(x: i32, y: i32) {
        let _ = CGDisplay::warp_mouse_cursor_position(CGPoint::new(x as f64, y as f64));
    }

    fn mouse_event_parts(event: MouseEventSpec) -> (CGEventType, CGMouseButton, Option<i64>) {
        match event {
            MouseEventSpec::LeftDown => (CGEventType::LeftMouseDown, CGMouseButton::Left, None),
            MouseEventSpec::LeftUp => (CGEventType::LeftMouseUp, CGMouseButton::Left, None),
            MouseEventSpec::RightDown => (CGEventType::RightMouseDown, CGMouseButton::Right, None),
            MouseEventSpec::RightUp => (CGEventType::RightMouseUp, CGMouseButton::Right, None),
            MouseEventSpec::MiddleDown => {
                (CGEventType::OtherMouseDown, CGMouseButton::Center, Some(2))
            }
            MouseEventSpec::MiddleUp => (CGEventType::OtherMouseUp, CGMouseButton::Center, Some(2)),
        }
    }

    pub fn send_mouse_event(event: MouseEventSpec, click_state: i64) {
        let source = match event_source() {
            Some(source) => source,
            None => return,
        };
        let location = match current_location() {
            Some(location) => location,
            None => return,
        };
        let (event_type, button, button_number) = mouse_event_parts(event);
        let mouse_event = match CGEvent::new_mouse_event(source, event_type, location, button) {
            Ok(event) => event,
            Err(_) => return,
        };

        mouse_event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
        if let Some(button_number) = button_number {
            mouse_event
                .set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button_number);
        }

        mouse_event.post(CGEventTapLocation::HID);
    }

    pub fn send_batch(down: MouseEventSpec, up: MouseEventSpec, n: usize, _hold_ms: u32) {
        for _ in 0..n {
            send_mouse_event(down, 1);
            send_mouse_event(up, 1);
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use super::{MouseEventSpec, VirtualScreenRect};

    pub fn current_cursor_position() -> Option<(i32, i32)> {
        None
    }

    pub fn current_virtual_screen_rect() -> Option<VirtualScreenRect> {
        None
    }

    pub fn current_monitor_rects() -> Option<Vec<VirtualScreenRect>> {
        None
    }

    pub fn move_mouse(_x: i32, _y: i32) {}

    pub fn send_mouse_event(_event: MouseEventSpec, _click_state: i64) {}

    pub fn send_batch(_down: MouseEventSpec, _up: MouseEventSpec, _n: usize, _hold_ms: u32) {}
}

pub fn current_cursor_position() -> Option<(i32, i32)> {
    platform::current_cursor_position()
}

pub fn current_virtual_screen_rect() -> Option<VirtualScreenRect> {
    platform::current_virtual_screen_rect()
}

pub fn current_monitor_rects() -> Option<Vec<VirtualScreenRect>> {
    platform::current_monitor_rects()
}

#[inline]
pub fn get_cursor_pos() -> (i32, i32) {
    current_cursor_position().unwrap_or((0, 0))
}

#[inline]
pub fn move_mouse(x: i32, y: i32) {
    platform::move_mouse(x, y);
}

#[inline]
pub fn send_mouse_event(event: MouseEventSpec, click_state: i64) {
    platform::send_mouse_event(event, click_state);
}

pub fn send_batch(down: MouseEventSpec, up: MouseEventSpec, n: usize, hold_ms: u32) {
    platform::send_batch(down, up, n, hold_ms);
}

fn dispatch_click<FSend, FSleep, FActive>(
    down: MouseEventSpec,
    up: MouseEventSpec,
    hold_ms: u32,
    click_state: i64,
    send_event: &mut FSend,
    sleep_for: &mut FSleep,
    is_active: &FActive,
) -> bool
where
    FSend: FnMut(MouseEventSpec, i64),
    FSleep: FnMut(Duration),
    FActive: Fn() -> bool,
{
    if !is_active() {
        return false;
    }

    send_event(down, click_state);
    if hold_ms > 0 {
        sleep_for(Duration::from_millis(hold_ms as u64));
        if !is_active() {
            send_event(up, click_state);
            return false;
        }
    }

    send_event(up, click_state);
    true
}

pub fn send_clicks(
    down: MouseEventSpec,
    up: MouseEventSpec,
    count: usize,
    hold_ms: u32,
    use_double_click_gap: bool,
    double_click_delay_ms: u32,
    control: &RunControl,
) {
    if count == 0 {
        return;
    }

    if !use_double_click_gap && count > 1 && hold_ms == 0 {
        send_batch(down, up, count, hold_ms);
        return;
    }

    let is_active = || control.is_active();
    let mut send_event = |event, click_state| send_mouse_event(event, click_state);
    let mut sleep_for = |duration| sleep_interruptible(duration, control);

    for index in 0..count {
        let click_state = if use_double_click_gap {
            (index + 1) as i64
        } else {
            1
        };

        if !dispatch_click(
            down,
            up,
            hold_ms,
            click_state,
            &mut send_event,
            &mut sleep_for,
            &is_active,
        ) {
            return;
        }

        if index + 1 < count && use_double_click_gap && double_click_delay_ms > 0 {
            sleep_interruptible(Duration::from_millis(double_click_delay_ms as u64), control);
        }
    }
}

#[inline]
pub fn get_button_flags(button: i32) -> (MouseEventSpec, MouseEventSpec) {
    match button {
        2 => (MouseEventSpec::RightDown, MouseEventSpec::RightUp),
        3 => (MouseEventSpec::MiddleDown, MouseEventSpec::MiddleUp),
        _ => (MouseEventSpec::LeftDown, MouseEventSpec::LeftUp),
    }
}

#[inline]
pub fn ease_in_out_quad(t: f64) -> f64 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}

#[inline]
pub fn cubic_bezier(t: f64, p0: f64, p1: f64, p2: f64, p3: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}

fn smooth_move_inner(
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
    duration_ms: u64,
    rng: &mut SmallRng,
    allow_overshoot: bool,
) {
    if duration_ms < 3 || (start_x == end_x && start_y == end_y) {
        move_mouse(end_x, end_y);
        return;
    }

    let (start_x, start_y) = (start_x as f64, start_y as f64);
    let (target_x, target_y) = (end_x as f64, end_y as f64);
    let delta_x = target_x - start_x;
    let delta_y = target_y - start_y;
    let distance = delta_x.hypot(delta_y);

    if distance < 3.0 {
        move_mouse(end_x, end_y);
        return;
    }

    let steps = if duration_ms <= 12 {
        (duration_ms / 3).clamp(1, 4) as usize
    } else {
        ((duration_ms / 8) as usize).clamp(4, 75)
    };

    let tick_duration = Duration::from_millis(duration_ms) / steps as u32;
    let start_time = Instant::now();

    let cp1_ratio = rng.next_f64() * 0.28 + 0.20;
    let cp2_ratio = rng.next_f64() * 0.24 + 0.55;

    let max_perp_offset = (distance * 0.29).min(76.0);

    let perp_x = -delta_y / distance;
    let perp_y = delta_x / distance;

    let offset_1 = (rng.next_f64() * 0.41 + 0.07)
        * max_perp_offset
        * (if rng.next_f64() >= 0.5 { 1.0 } else { -1.0 });
    let offset_2 = (rng.next_f64() * 0.41 + 0.07)
        * max_perp_offset
        * (if rng.next_f64() >= 0.5 { 1.0 } else { -1.0 });

    let control_1x = start_x + delta_x * cp1_ratio + perp_x * offset_1;
    let control_1y = start_y + delta_y * cp1_ratio + perp_y * offset_1;
    let control_2x = start_x + delta_x * cp2_ratio + perp_x * offset_2;
    let control_2y = start_y + delta_y * cp2_ratio + perp_y * offset_2;

    let mid_wobble = rng.next_f64() < 0.37 && duration_ms > 22;
    let wobble_step = if mid_wobble { steps / 2 } else { 0 };

    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let ease = ease_in_out_quad(t);

        let mut current_x = cubic_bezier(ease, start_x, control_1x, control_2x, target_x);
        let mut current_y = cubic_bezier(ease, start_y, control_1y, control_2y, target_y);

        if mid_wobble && i == wobble_step {
            let wobble = rng.next_f64() * 1.7 + 0.7;
            let sign = if rng.next_f64() >= 0.5 { 1.0 } else { -1.0 };
            current_x += perp_x * wobble * sign;
            current_y += perp_y * wobble * sign;
        }

        move_mouse(current_x as i32, current_y as i32);

        if i < steps {
            let elapsed = start_time.elapsed();
            let expected = tick_duration * (i + 1) as u32;

            if expected > elapsed {
                std::thread::sleep(expected - elapsed);
            }
        }
    }

    if allow_overshoot && duration_ms > 16 && rng.next_f64() < 0.47 {
        let overshoot_amount = rng.next_f64() * 6.3 + 2.2;
        let dir_x = delta_x / distance;
        let dir_y = delta_y / distance;

        let over_x = (target_x + dir_x * overshoot_amount) as i32;
        let over_y = (target_y + dir_y * overshoot_amount) as i32;

        let correction_ms = (duration_ms as f64 * 0.19).max(4.0) as u64;

        smooth_move_inner(end_x, end_y, over_x, over_y, correction_ms, rng, false);
        smooth_move_inner(
            over_x,
            over_y,
            end_x,
            end_y,
            (correction_ms * 2 / 3).max(3),
            rng,
            false,
        );
    }
}

pub fn smooth_move(
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
    duration_ms: u64,
    rng: &mut SmallRng,
) {
    smooth_move_inner(start_x, start_y, end_x, end_y, duration_ms, rng, true);
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::{dispatch_click, MouseEventSpec};

    #[test]
    fn dispatch_click_skips_events_when_run_is_already_stopped() {
        let events = RefCell::new(Vec::new());
        let mut send_event = |event, click_state| events.borrow_mut().push((event, click_state));
        let mut sleep_for = |_| {};
        let is_active = || false;

        let sent = dispatch_click(
            MouseEventSpec::LeftDown,
            MouseEventSpec::LeftUp,
            5,
            1,
            &mut send_event,
            &mut sleep_for,
            &is_active,
        );

        assert!(!sent);
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn dispatch_click_releases_button_when_run_stops_during_hold() {
        let events = RefCell::new(Vec::new());
        let mut send_event = |event, click_state| events.borrow_mut().push((event, click_state));
        let active = Cell::new(true);
        let mut sleep_for = |_| active.set(false);
        let is_active = || active.get();

        let sent = dispatch_click(
            MouseEventSpec::LeftDown,
            MouseEventSpec::LeftUp,
            5,
            1,
            &mut send_event,
            &mut sleep_for,
            &is_active,
        );

        assert!(!sent);
        assert_eq!(
            &*events.borrow(),
            &[(MouseEventSpec::LeftDown, 1), (MouseEventSpec::LeftUp, 1)]
        );
    }

    #[test]
    fn dispatch_click_sends_normal_down_and_up_when_run_stays_active() {
        let events = RefCell::new(Vec::new());
        let mut send_event = |event, click_state| events.borrow_mut().push((event, click_state));
        let mut sleep_for = |_| {};
        let is_active = || true;

        let sent = dispatch_click(
            MouseEventSpec::LeftDown,
            MouseEventSpec::LeftUp,
            5,
            2,
            &mut send_event,
            &mut sleep_for,
            &is_active,
        );

        assert!(sent);
        assert_eq!(
            &*events.borrow(),
            &[(MouseEventSpec::LeftDown, 2), (MouseEventSpec::LeftUp, 2)]
        );
    }
}
