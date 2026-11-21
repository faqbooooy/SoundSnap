#![windows_subsystem = "windows"]
#![no_std]
#![no_main]

extern crate alloc;

mod audio;
mod gfx;
mod palette;
mod profile_app;
mod profile_ui;
mod profiles;
mod runtime;
mod settings;
mod ui;

use alloc::{string::String, vec::Vec};
use core::{cell::RefCell, mem::{size_of, zeroed}, ptr::{null, null_mut}};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST}},
    System::{Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE},
        LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{HiDpi::{GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI},
        Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};
use audio::{Audio, Device, Volume};
use gfx::{contains, GraphicsSession, Surface};
use palette::{Palette, DARK, LIGHT};
use profile_app::ProfileState;
use settings::{Mode, Settings, Shortcut};
use ui::{Action, Panel, SettingsView};

const TRAY_MESSAGE: u32 = WM_APP + 1;
const TRAY_WHEEL: u32 = WM_APP + 2;
const TRAY_MUTE: u32 = WM_APP + 3;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const TOAST_TIMER: usize = 1;
const HINT_TIMER: usize = 2;
const TRAY_TIMER: usize = 3;
const VOLUME_HOVER_TIMER: usize = 4;
const EDIT_ANIMATION_TIMER: usize = 5;
const EDIT_WIGGLE: [f32; 13] = [0.,1.4,2.3,1.1,-1.2,-2.1,-0.8,0.9,1.35,0.55,-0.55,-0.5,0.];
const SHORTCUT_HINT: &str = "Нажмите на сочетание, чтобы изменить его";
const THEME_HINT: &str = "Применяется сразу";
const CLASS: &str = "SoftAudioSwitch.Native";
const HOST_TITLE: &str = "SoftAudioSwitch.Host";
const TRAY_READY: &str = "SoftAudioSwitch.TrayReady";
const EX_CUSTOM: u32 = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
const EX_PASSIVE: u32 = EX_CUSTOM | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;

// Only the owner thread dispatches window messages. Core Audio callbacks post back to it.
static mut APP: *const RefCell<App> = null();
static mut MOUSE_HOST: HWND = null_mut();
static mut MIDDLE_CAPTURED: bool = false;

pub fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain([0]).collect() }

#[no_mangle]
pub extern "C" fn main() -> i32 {
    let mode = runtime::argument();
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hr = CoInitializeEx(null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32);
        if hr < 0 { return 3; }
        let result = match mode.as_deref() {
            Some("--core-check") => core_check(),
            Some("--render-check") => render_check(),
            Some("--tray-check") => {
                let host = FindWindowW(wide(CLASS).as_ptr(), wide(HOST_TITLE).as_ptr());
                if !host.is_null() && !GetPropW(host, wide(TRAY_READY).as_ptr()).is_null() { 0 } else { 7 }
            }
            _ => run(mode.as_deref() == Some("--demo-devices")),
        };
        CoUninitialize();
        result
    }
}

fn core_check() -> i32 {
    let mut audio = Audio::default();
    if let Ok(devices) = audio.devices() {
        if devices.is_empty() || (audio.volume().is_ok() && audio.sessions(None).is_ok()) { return 0; }
    }
    4
}

fn render_check() -> i32 {
    let Some(_session) = GraphicsSession::new() else { return 5 };
    let mut form = SettingsView { shortcut: Shortcut { key: 0x78, mods: 3 }, startup: false,
        capture: false, feedback: SHORTCUT_HINT, theme_hint: THEME_HINT };
    let devices = [Device { id: "sample".into(), name: "Динамики (Realtek(R) Audio)".into(), selected: true },
        Device { id: "sample-2".into(), name: "Динамики (Logitech PRO X Gaming Headset)".into(), selected: false }];
    for palette in [&LIGHT, &DARK] {
        let model = Panel {
            devices: &devices, excluded: &[], start: 0, rows: 2, error: false,
            volume: Some(Volume { level: 72, muted: false }),
            editing: false, expanded: false, wiggle: 0., profile: None,
        };
        let Some(mut panel) = Surface::new(ui::PANEL_WIDTH, model.height()) else { return 5 };
        let Some(mut settings) = Surface::new(ui::PANEL_WIDTH, ui::SETTINGS_HEIGHT) else { return 5 };
        let Some(mut toast) = Surface::new(ui::TOAST_WIDTH,ui::TOAST_HEIGHT) else { return 5 };
        if !panel.draw(1., |g| ui::panel(g, palette, model, None, None))
            || !settings.draw(1., |g| ui::settings(g, palette, &form, Mode::System, None, None))
            || !toast.draw(1., |g| ui::toast(g, palette, "ЗВУК ПЕРЕКЛЮЧЁН", "Динамики")) {
            return 5;
        }
        if !panel.rounded_alpha() || !settings.rounded_alpha() || !toast.rounded_alpha() { return 6; }
        form.startup = true;
    }
    0
}

unsafe fn run(demo: bool) -> i32 {
    let mutex = CreateMutexW(null(), 1, wide("Local\\SoftAudioSwitch").as_ptr());
    let last_error = GetLastError();
    if mutex.is_null() { return 1; }
    if last_error == ERROR_ALREADY_EXISTS { CloseHandle(mutex); return 0; }
    let code = run_single_instance(demo);
    CloseHandle(mutex);
    code
}

unsafe fn run_single_instance(demo: bool) -> i32 {
    let Some(_graphics) = GraphicsSession::new() else { return 2 };
    let instance = GetModuleHandleW(null());
    let class = wide(CLASS);
    let arrow = LoadCursorW(null_mut(), IDC_ARROW);
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc), hInstance: instance, hCursor: arrow, lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    if RegisterClassW(&window_class) == 0 { return 2; }
    let host = make_window(instance, 0, HOST_TITLE);
    if host.is_null() { return 2; }
    let mut app = App::new(instance, host);
    if demo { app.demo_devices = demo_devices(); }
    app.audio.watch(host);
    app.refresh_volume();
    if !app.update_tray(NIM_ADD, true) { SetTimer(host, TRAY_TIMER, 1000, None); }
    app.set_hotkey(app.settings.shortcut);
    app.sync_profile_hotkeys();
    let slot = RefCell::new(app);
    APP = &slot;
    MOUSE_HOST = host;
    let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(tray_mouse_proc), instance, 0);
    let mut message: MSG = zeroed();
    let status = loop {
        match GetMessageW(&mut message, null_mut(), 0, 0) {
            0 => break 0,
            -1 => break 2,
            _ => { TranslateMessage(&message); DispatchMessageW(&message); }
        }
    };
    if !mouse_hook.is_null() { UnhookWindowsHookEx(mouse_hook); }
    MOUSE_HOST = null_mut();
    APP = null();
    drop(slot);
    status
}

unsafe extern "system" fn tray_mouse_proc(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        match message as u32 {
            WM_MBUTTONUP if MIDDLE_CAPTURED => {
                MIDDLE_CAPTURED = false;
                return 1;
            }
            WM_MOUSEWHEEL | WM_MBUTTONDOWN => {
                let input = &*(data as *const MSLLHOOKSTRUCT);
                if tray_icon_at(MOUSE_HOST, input.pt).is_some() {
                    if message as u32 == WM_MOUSEWHEEL {
                        let delta = (input.mouseData >> 16) as i16;
                        if PostMessageW(MOUSE_HOST, TRAY_WHEEL, delta as isize as usize, 0) != 0 { return 1; }
                    } else if PostMessageW(MOUSE_HOST, TRAY_MUTE, 0, 0) != 0 {
                        MIDDLE_CAPTURED = true;
                        return 1;
                    }
                }
            }
            _ => (),
        }
    }
    CallNextHookEx(null_mut(), code, message, data)
}

unsafe fn tray_icon_at(host: HWND, point: POINT) -> Option<RECT> {
    let window = WindowFromPoint(point);
    if window.is_null() { return None; }
    let mut process = 0;
    let mut shell_process = 0;
    GetWindowThreadProcessId(window, &mut process);
    GetWindowThreadProcessId(GetShellWindow(), &mut shell_process);
    if process == 0 || process != shell_process { return None; }

    let icon = NOTIFYICONIDENTIFIER {
        cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: host, uID: 1, guidItem: zeroed(),
    };
    let mut bounds: RECT = zeroed();
    (Shell_NotifyIconGetRect(&icon, &mut bounds) == 0 && point_in_rect(point, bounds)).then_some(bounds)
}

fn point_in_rect(point: POINT, bounds: RECT) -> bool {
    point.x >= bounds.left && point.x < bounds.right && point.y >= bounds.top && point.y < bounds.bottom
}
unsafe fn make_window(instance: HINSTANCE, ex_style: u32, title: &str) -> HWND {
    let hwnd = CreateWindowExW(ex_style, wide(CLASS).as_ptr(), wide(title).as_ptr(), WS_POPUP,
        0, 0, 1, 1, null_mut(), null_mut(), instance, null());
    if !hwnd.is_null() && ex_style & WS_EX_LAYERED != 0 {
        let disabled = 1i32;
        let no_border = 0xFFFFFFFEu32;
        DwmSetWindowAttribute(hwnd, 2, (&disabled as *const i32).cast(), 4);
        DwmSetWindowAttribute(hwnd, 3, (&disabled as *const i32).cast(), 4);
        DwmSetWindowAttribute(hwnd, 34, (&no_border as *const u32).cast(), 4);
    }
    hwnd
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let value = APP.as_ref().and_then(|slot| slot.try_borrow_mut().ok())
        .and_then(|mut app| app.message(hwnd, msg, wp, lp));
    value.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowKind { Hidden, Popup, Settings, Profiles, Menu }

struct Layer {
    hwnd: HWND,
    pixels: Option<Surface>,
}
impl Layer {
    fn new() -> Self { Self { hwnd: null_mut(), pixels: None } }
    fn ensure(&mut self, instance: HINSTANCE, passive: bool, title: &str) -> bool {
        if self.hwnd.is_null() {
            self.hwnd = unsafe { make_window(instance, if passive { EX_PASSIVE } else { EX_CUSTOM }, title) };
        } else {
            unsafe { SetWindowTextW(self.hwnd, wide(title).as_ptr()); }
        }
        !self.hwnd.is_null()
    }
    fn paint(&mut self, position: POINT, w: i32, h: i32, scale: f32, paint: impl FnOnce(&gfx::Painter)) -> bool {
        let width = ceil_pixels(w as f32 * scale);
        let height = ceil_pixels(h as f32 * scale);
        if self.pixels.as_ref().is_none_or(|surface| surface.width != width || surface.height != height) {
            self.pixels = Surface::new(width, height);
        }
        self.pixels.as_mut().is_some_and(|surface| surface.draw(scale, paint) && surface.present(self.hwnd, position))
    }
    fn hide(&mut self) {
        if !self.hwnd.is_null() { unsafe { ShowWindow(self.hwnd, SW_HIDE); } }
        self.pixels = None;
    }
}
impl Drop for Layer {
    fn drop(&mut self) {
        if !self.hwnd.is_null() { unsafe { DestroyWindow(self.hwnd); } }
    }
}

struct Notice { title: String, text: String, monitor: POINT }
struct DemoDevice { device: Device, hidden: bool }

fn demo_devices() -> Vec<DemoDevice> {
    [
        "Наушники Bluetooth",
        "Акустика рабочего стола",
        "Монитор Studio Display (HDMI)",
        "USB ЦАП — линейный выход",
        "Наушники для игр",
        "Домашний кинотеатр (HDMI)",
        "Портативная колонка",
        "Динамики ноутбука",
        "Геймпад — аудиовыход",
        "Монитор конференц-зала",
        "Виртуальный выход для записи",
    ].into_iter().enumerate().map(|(index, name)| DemoDevice {
        device: Device {
            id: alloc::format!("demo:{index}"),
            name: alloc::format!("[Демо] {name}"),
            selected: false,
        },
        hidden: index >= 8,
    }).collect()
}

struct App {
    instance: HINSTANCE, host: HWND, taskbar_message: u32,
    icon: HICON, tray_exists: bool, dark: bool, taskbar_dark: bool, tray_muted: bool,
    settings: Settings, hotkey_id: i32, hotkey: Option<Shortcut>,
    profiles: ProfileState,
    audio: Audio, devices: Vec<Device>, device_error: bool, volume: Option<Volume>,
    demo_devices: Vec<DemoDevice>,
    editing_devices: bool, other_expanded: bool, excluded: Vec<bool>,
    edit_frame: usize, wheel_remainder: i32,
    window: Layer, toast: Layer, hint: Layer, notice: Option<Notice>, kind: WindowKind,
    volume_popup: Layer, volume_anchor: Option<RECT>,
    form: Option<SettingsView>, hover: Option<Action>, focus: Option<Action>, pressed: Option<Action>,
    width: i32, height: i32, scale: f32, rows: usize, start: usize, area: RECT,
    position: POINT, anchor: POINT, tracking_mouse: bool,
}

impl App {
    fn new(instance: HINSTANCE, host: HWND) -> Self {
        let settings = Settings::load();
        Self {
            instance, host, taskbar_message: unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
            icon: null_mut(), tray_exists: false, dark: settings::dark(settings.theme), taskbar_dark: settings::taskbar_dark(),
            tray_muted: false, volume_popup: Layer::new(), volume_anchor: None,
            settings, hotkey_id: 0, hotkey: None, profiles: ProfileState::new(), audio: Audio::default(), devices: Vec::new(),
            demo_devices: Vec::new(),
            editing_devices: false, other_expanded: false, excluded: Vec::new(),
            edit_frame: 0, wheel_remainder: 0,
            device_error: false, volume: None, window: Layer::new(), toast: Layer::new(), hint: Layer::new(),
            notice: None, kind: WindowKind::Hidden, form: None, hover: None, focus: None, pressed: None,
            width: 0, height: 0, scale: 1., rows: 4, start: 0, area: unsafe { zeroed() },
            position: POINT {x:0,y:0}, anchor: POINT {x:0,y:0}, tracking_mouse: false,
        }
    }

    fn update_tray(&mut self, operation: u32, redraw: bool) -> bool {
        unsafe {
            let new_icon = if redraw || self.icon.is_null() {
                let size = GetSystemMetrics(SM_CXSMICON).clamp(16, 64).max(32);
                gfx::tray_icon(self.taskbar_dark, self.tray_muted, size)
            } else { self.icon };
            if new_icon.is_null() { return false; }
            let mut data: NOTIFYICONDATAW = zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.host;
            data.uID = 1;
            data.uFlags = NIF_MESSAGE | NIF_ICON;
            data.uCallbackMessage = TRAY_MESSAGE;
            data.hIcon = new_icon;
            if Shell_NotifyIconW(operation, &data) == 0 {
                if new_icon != self.icon { DestroyIcon(new_icon); }
                return false;
            }
            if new_icon != self.icon && !self.icon.is_null() { DestroyIcon(self.icon); }
            self.icon = new_icon;
            self.tray_exists = true;
            SetPropW(self.host, wide(TRAY_READY).as_ptr(), 1usize as _);
            true
        }
    }

    fn set_hotkey(&mut self, shortcut: Shortcut) -> bool {
        if self.hotkey == Some(shortcut) { return true; }
        let id = if self.hotkey_id == 1 { 2 } else { 1 };
        unsafe {
            if RegisterHotKey(self.host, id, shortcut.mods | MOD_NOREPEAT, shortcut.key) == 0 { return false; }
            if self.hotkey_id != 0 { UnregisterHotKey(self.host, self.hotkey_id); }
        }
        self.hotkey = Some(shortcut);
        self.hotkey_id = id;
        true
    }

    fn palette(&self) -> &'static Palette { if self.dark { &DARK } else { &LIGHT } }
    fn sync_theme(&mut self) {
        let tray = settings::taskbar_dark();
        if self.taskbar_dark != tray {
            self.taskbar_dark = tray;
            if !self.update_tray(NIM_MODIFY, true) { self.update_tray(NIM_ADD, true); }
        }
        let dark = settings::dark(self.settings.theme);
        let changed = dark != self.dark;
        self.dark = dark;
        if changed {
            self.draw_window();
            self.draw_toast();
            self.draw_volume_popup();
            self.hide_hint();
        }
    }

    fn window_model(&self) -> Panel<'_> {
        Panel { devices: &self.devices, excluded: &self.excluded, start: self.start, rows: self.rows,
            error: self.device_error, volume: self.volume,
            editing: self.editing_devices, expanded: self.other_expanded,
            wiggle: EDIT_WIGGLE[self.edit_frame], profile: self.profiles.name() }
    }
    fn refresh_devices(&mut self) {
        let old_focus = match self.focus { Some(Action::Device(i)) => self.devices.get(i).map(|d| d.id.clone()), _ => None };
        let result = self.audio.devices();
        self.device_error = result.is_err();
        let devices = result.unwrap_or_default().into_iter().map(|device| {
            let excluded = settings::device_hidden(&device.id);
            (device, excluded)
        }).chain(self.demo_devices.iter().map(|demo| (demo.device.clone(), demo.hidden)));
        (self.devices, self.excluded) = devices.unzip();
        self.refresh_volume();
        self.start = self.start.min(self.devices.len().saturating_sub(self.rows));
        if let Some(id) = old_focus {
            self.focus = self.devices.iter().position(|d| d.id.eq_ignore_ascii_case(&id)).map(Action::Device)
                .filter(|action| self.controls().iter().any(|(item, _)| item == action));
        }
    }
    fn refresh_volume(&mut self) {
        self.volume = self.audio.volume().ok();
        let muted = self.volume.is_some_and(|volume| volume.muted);
        if muted != self.tray_muted {
            self.tray_muted = muted;
            if self.tray_exists && !self.update_tray(NIM_MODIFY, true) {
                self.update_tray(NIM_ADD, true);
            }
        }
        self.draw_volume_popup();
    }
    fn switch_next(&mut self) {
        if self.kind == WindowKind::Settings || (self.kind == WindowKind::Profiles && (self.profiles.typing || self.profiles.capturing)) {
            return;
        }
        match self.audio.devices() {
            Ok(mut devices) => {
                devices.retain(|device| !settings::device_hidden(&device.id));
                let index = devices.iter().position(|d| d.selected).map_or(0, |i| i + 1);
                if let Some(device) = devices.get(index).or_else(|| devices.first()).filter(|d| !d.selected) {
                    self.select_device(device);
                } else {
                    self.notify("Переключение недоступно",
                        if devices.is_empty() { "Нет основных устройств" } else { "Доступно одно основное устройство" });
                }
            }
            Err(_) => self.notify("Не удалось переключить звук", "Попробуйте выбрать выход вручную"),
        }
    }
    fn select_device(&mut self, device: &Device) {
        if self.demo_devices.iter().any(|demo| demo.device.id == device.id) {
            self.notify("Демонстрационное устройство", "Выход звука не изменился");
            return;
        }
        match self.audio.select(&device.id) {
            Ok(true) => {
                self.deactivate_profile();
                self.refresh_volume();
                self.notify("Звук переключён", &device.name);
                if self.kind == WindowKind::Popup {
                    for row in &mut self.devices { row.selected = row.id.eq_ignore_ascii_case(&device.id); }
                    self.draw_window();
                }
            }
            Ok(false) => self.notify("Смена выхода не подтверждена", "Проверьте активное устройство"),
            Err(_) => self.notify("Не удалось выбрать устройство", "Проверьте, что оно подключено"),
        }
    }
    fn volume_at(&mut self, x: f32) {
        self.set_volume_level(ui::volume_level(x));
    }
    fn set_volume_level(&mut self, level: i32) {
        let Some(volume) = self.volume else { return };
        if level == volume.level { return; }
        let _ = self.audio.set_volume(level);
        self.refresh_volume();
        self.draw_window();
    }
    fn tray_wheel(&mut self, delta: i32) {
        self.wheel_remainder += delta;
        let steps = self.wheel_remainder / WHEEL_DELTA as i32;
        self.wheel_remainder %= WHEEL_DELTA as i32;
        if steps == 0 { return; }
        if let Ok(volume) = self.audio.volume() {
            let level = (volume.level + steps * 2).clamp(0, 100);
            if level != volume.level { let _ = self.audio.set_volume(level); }
            self.refresh_after_tray_audio();
        }
        self.show_volume_popup();
    }
    fn tray_mute(&mut self) {
        let _ = self.audio.toggle_mute();
        self.refresh_after_tray_audio();
        self.show_volume_popup();
    }
    fn refresh_after_tray_audio(&mut self) {
        self.refresh_volume();
        if self.kind == WindowKind::Popup {
            self.draw_window();
        }
    }
    fn show_volume_popup(&mut self) {
        if self.volume_anchor.is_some() || self.kind == WindowKind::Menu { return; }
        let Some(bounds) = (unsafe { tray_icon_at(self.host, cursor()) }) else { return };
        self.volume_anchor = Some(bounds);
        self.refresh_volume();
        unsafe { SetTimer(self.host, VOLUME_HOVER_TIMER, 125, None); }
    }
    fn hide_volume_popup(&mut self) {
        if self.volume_anchor.take().is_some() {
            unsafe { KillTimer(self.host, VOLUME_HOVER_TIMER); }
            self.volume_popup.hide();
        }
    }
    fn draw_volume_popup(&mut self) {
        let Some(bounds) = self.volume_anchor else { return };
        let Some(volume) = self.volume else {
            self.volume_popup.hide();
            return;
        };
        let center = POINT { x: (bounds.left + bounds.right) / 2, y: (bounds.top + bounds.bottom) / 2 };
        let (work, scale) = screen(center);
        let width = ceil_pixels(ui::TRAY_VOLUME_WIDTH as f32 * scale);
        let height = ceil_pixels(ui::TRAY_VOLUME_HEIGHT as f32 * scale);
        let gap = ceil_pixels(8. * scale);
        let y = if center.y < (work.top + work.bottom) / 2 { bounds.bottom + gap }
            else { bounds.top - height - gap };
        let position = POINT {
            x: (center.x - width / 2).clamp(work.left, (work.right - width).max(work.left)),
            y: y.clamp(work.top, (work.bottom - height).max(work.top)),
        };
        let palette = self.palette();
        if self.volume_popup.ensure(self.instance, true, "SoundSnap — громкость")
            && self.volume_popup.paint(position, ui::TRAY_VOLUME_WIDTH, ui::TRAY_VOLUME_HEIGHT, scale,
                |g| ui::tray_volume(g, palette, volume)) {
            unsafe { ShowWindow(self.volume_popup.hwnd, SW_SHOWNOACTIVATE); }
        }
    }
    fn refresh_device_page(&mut self, focus: Action) {
        self.hover = None;
        self.focus = None;
        self.hide_hint();
        self.refresh_devices();
        self.focus = Some(focus);
        self.geometry();
        self.draw_window();
    }

    fn toggle_device_editor(&mut self) {
        self.editing_devices = !self.editing_devices;
        self.start = 0;
        self.stop_device_animation();
        if self.editing_devices {
            let mut enabled = 0i32;
            unsafe {
                if SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut enabled as *mut i32).cast(), 0) != 0
                    && enabled != 0 {
                    SetTimer(self.host, EDIT_ANIMATION_TIMER, 22, None);
                }
            }
        }
        self.refresh_device_page(Action::EditDevices);
    }

    fn stop_device_animation(&mut self) {
        unsafe { KillTimer(self.host, EDIT_ANIMATION_TIMER); }
        self.edit_frame = 0;
    }

    fn show(&mut self, kind: WindowKind, anchor: POINT) {
        self.hide_volume_popup();
        let opening = self.kind == WindowKind::Hidden;
        self.sync_theme();
        self.leave_page();
        self.anchor = anchor;
        let (area, scale) = screen(anchor);
        self.area = area;
        self.scale = scale;
        self.kind = kind;
        let title = match kind {
            WindowKind::Popup => {
                self.editing_devices = false;
                self.other_expanded = false;
                self.rows = ui::rows_for_height((area.bottom - area.top) as f32 / scale);
                self.refresh_devices();
                self.start = self.devices.iter().position(|row| row.selected)
                    .map_or(0, |index| index.saturating_sub(self.rows - 1)).min(self.devices.len().saturating_sub(self.rows));
                "Звук — устройства вывода"
            }
            WindowKind::Settings => {
                self.form = Some(SettingsView { shortcut: self.settings.shortcut, startup: settings::launched_on_startup(),
                    capture: false, feedback: SHORTCUT_HINT, theme_hint: THEME_HINT });
                "Настройки звука"
            }
            WindowKind::Profiles => {
                self.reset_profile_view();
                "SoundSnap — микшер и профили"
            }
            WindowKind::Menu => "Звук — меню",
            WindowKind::Hidden => return,
        };
        self.sync_profile_timer();
        if !self.window.ensure(self.instance, false, title) {
            self.hide();
            return;
        }
        self.geometry();
        if self.draw_window() {
            unsafe {
                if opening { ShowWindow(self.window.hwnd, SW_SHOW); }
                if GetForegroundWindow() != self.window.hwnd { SetForegroundWindow(self.window.hwnd); }
            }
        } else {
            self.hide();
        }
    }
    fn geometry(&mut self) {
        (self.width, self.height) = match self.kind {
            WindowKind::Popup => (ui::PANEL_WIDTH, self.window_model().height()),
            WindowKind::Settings => (ui::PANEL_WIDTH, ui::SETTINGS_HEIGHT),
            WindowKind::Profiles => (ui::PANEL_WIDTH, self.profile_model().height()),
            WindowKind::Menu => (ui::MENU_WIDTH, ui::MENU_HEIGHT),
            WindowKind::Hidden => return,
        };
        let w = ceil_pixels(self.width as f32 * self.scale);
        let h = ceil_pixels(self.height as f32 * self.scale);
        let area = self.area;
        let anchor = self.anchor;
        let (x,y) = match self.kind {
            WindowKind::Menu => (anchor.x - w + 5, anchor.y - h - 3),
            _ => (anchor.x - w + 12, if anchor.y >= (area.top + area.bottom) / 2 { anchor.y - h - 10 } else { anchor.y + 10 }),
        };
        self.position = POINT { x: x.clamp(area.left,(area.right - w).max(area.left)),
                                y: y.clamp(area.top,(area.bottom - h).max(area.top)) };
    }
    fn draw_window(&mut self) -> bool {
        let palette = self.palette();
        let panel = Panel { devices: &self.devices, excluded: &self.excluded, start: self.start, rows: self.rows,
            error: self.device_error, volume: self.volume,
            editing: self.editing_devices, expanded: self.other_expanded,
            wiggle: EDIT_WIGGLE[self.edit_frame], profile: self.profiles.name() };
        let profiles = self.profiles.view(self.profile_height_budget());
        let (hover, focus, mode, kind, form) = (self.hover,self.focus,self.settings.theme,self.kind,self.form.as_ref());
        if kind == WindowKind::Hidden || self.window.hwnd.is_null() { return false; }
        self.window.paint(self.position, self.width, self.height, self.scale, |g| match kind {
            WindowKind::Popup => ui::panel(g, palette, panel, hover, focus),
            WindowKind::Settings => { if let Some(form) = form { ui::settings(g, palette, form, mode, hover, focus); } }
            WindowKind::Profiles => profile_ui::draw(g,palette,&profiles,hover,focus),
            WindowKind::Menu => ui::menu(g, palette, hover, focus),
            WindowKind::Hidden => (),
        })
    }
    fn leave_page(&mut self) {
        if self.kind == WindowKind::Popup {
            self.stop_device_animation();
            self.devices.clear();
            self.excluded.clear();
        }
        if self.kind == WindowKind::Profiles { self.profiles.close(); }
        self.form = None;
        self.hover = None;
        self.focus = None;
        self.pressed = None;
        unsafe { if GetCapture() == self.window.hwnd { ReleaseCapture(); } }
        self.hide_hint();
    }
    fn hide(&mut self) {
        if self.kind == WindowKind::Hidden { return; }
        self.leave_page();
        self.kind = WindowKind::Hidden;
        self.sync_profile_timer();
        self.tracking_mouse = false;
        self.window.hide();
    }

    fn notify(&mut self, title: &str, text: &str) {
        self.notice = Some(Notice {title: title.to_uppercase(), text: text.into(), monitor: cursor()});
        self.draw_toast();
        unsafe { SetTimer(self.host, TOAST_TIMER, 1500, None); }
    }
    fn draw_toast(&mut self) {
        let Some(note) = &self.notice else { return; };
        let palette = self.palette();
        let (area, scale) = screen(note.monitor);
        let position = POINT { x: (area.right - ceil_pixels((ui::TOAST_WIDTH + 16) as f32 * scale)).max(area.left),
            y: (area.bottom - ceil_pixels((ui::TOAST_HEIGHT + 16) as f32 * scale)).max(area.top) };
        if self.toast.ensure(self.instance, true, "Звук — уведомление")
            && self.toast.paint(position, ui::TOAST_WIDTH, ui::TOAST_HEIGHT, scale, |g| ui::toast(g,palette,&note.title,&note.text)) {
            unsafe { ShowWindow(self.toast.hwnd, SW_SHOWNOACTIVATE); }
        }
    }
    fn hide_hint(&mut self) {
        unsafe { KillTimer(self.host, HINT_TIMER); }
        self.hint.hide();
    }
    fn show_hint(&mut self) {
        unsafe { KillTimer(self.host, HINT_TIMER); }
        if self.kind != WindowKind::Popup || self.pressed.is_some() { return; }
        let text = match self.hover {
            Some(Action::Device(index) | Action::QuickDevice(index)) => {
                let Some(device) = self.devices.get(index) else { return };
                let limit = if self.editing_devices { 27 } else { 32 };
                if !matches!(self.hover, Some(Action::QuickDevice(_))) && device.name.chars().count() < limit {
                    return;
                }
                &device.name
            }
            _ => return,
        };
        let palette = self.palette();
        let position = cursor();
        let width = (text.chars().count() as i32 * 7 + 20).clamp(110, 450);
        let position = POINT { x: position.x.min(self.area.right - (width as f32*self.scale) as i32).max(self.area.left),
            y: (position.y + 19).min(self.area.bottom - (30.*self.scale) as i32).max(self.area.top) };
        if self.hint.ensure(self.instance, true, "Устройство вывода")
            && self.hint.paint(position, width, 28, self.scale, |g| {
                g.use_panel_typography();
                let r = gfx::rect(0.5,0.5,width as f32-1.,27.);
                g.rounded(r,5.,palette.surface,Some(palette.border));
                g.text(text,gfx::rect(7.,0.,width as f32-14.,28.),12.,false,0,palette.ink);
            }) {
            unsafe { ShowWindow(self.hint.hwnd, SW_SHOWNOACTIVATE); }
        }
    }

    fn controls(&self) -> Vec<(Action, windows_sys::Win32::Graphics::GdiPlus::RectF)> {
        match self.kind {
            WindowKind::Popup => ui::panel_controls(&self.window_model()),
            WindowKind::Settings => ui::settings_controls().into(),
            WindowKind::Profiles => profile_ui::controls(&self.profile_model()),
            WindowKind::Menu => ui::menu_controls().into(),
            WindowKind::Hidden => Vec::new(),
        }
    }
    fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.controls().into_iter().find(|(_, r)| contains(*r,x,y)).map(|(action,_)| action)
    }
    fn invoke(&mut self, action: Action) {
        match action {
            Action::Device(index) if self.editing_devices => {
                if let (Some(device), Some(&excluded)) = (self.devices.get(index), self.excluded.get(index)) {
                    let hidden = !excluded;
                    let saved = if let Some(demo) = self.demo_devices.iter_mut().find(|demo| demo.device.id == device.id) {
                        demo.hidden = hidden;
                        true
                    } else {
                        settings::set_device_hidden(&device.id, hidden)
                    };
                    if saved {
                        self.excluded[index] = hidden;
                        self.draw_window();
                    } else {
                        self.notify("Не удалось сохранить список", "Попробуйте ещё раз");
                    }
                }
            }
            Action::Device(index) | Action::QuickDevice(index) => {
                if let Some(device) = self.devices.get(index).filter(|row| !row.selected).cloned() { self.select_device(&device); }
            }
            Action::EditDevices => self.toggle_device_editor(),
            Action::OtherDevices => {
                self.other_expanded = !self.other_expanded;
                self.start = 0;
                self.refresh_device_page(Action::OtherDevices);
            }
            Action::Settings => self.show(WindowKind::Settings,self.anchor),
            Action::Open | Action::Back | Action::Cancel => self.show(WindowKind::Popup,self.anchor),
            Action::Close => self.hide(),
            Action::Exit => unsafe { PostQuitMessage(0); },
            Action::Mute => {
                let _ = self.audio.toggle_mute();
                self.refresh_volume();
                self.draw_window();
            }
            Action::Startup => if let Some(form) = self.form.as_mut() { form.startup = !form.startup; self.draw_window(); },
            Action::Shortcut => if let Some(form) = self.form.as_mut() {
                form.capture = true;
                form.feedback = "Esc — отменить ввод";
                self.draw_window();
            },
            Action::Theme(index) => {
                let mut next = self.settings;
                next.theme = ui::MODES[index as usize];
                let applied = next.theme == self.settings.theme || next.save();
                if let Some(form) = self.form.as_mut() {
                    form.theme_hint = if applied { THEME_HINT } else { "Не удалось сохранить тему." };
                }
                if applied { self.settings = next; self.sync_theme(); }
                self.draw_window();
            }
            Action::Save => self.save_form(),
            Action::Volume | Action::ProfileMaster | Action::ProfileFallbackLevel | Action::AppVolume(_) => (),
            action => self.profile_action(action),
        }
    }
    fn save_form(&mut self) {
        let Some(form) = &self.form else { return; };
        let (shortcut,startup) = (form.shortcut,form.startup);
        let previous = self.hotkey;
        if !self.set_hotkey(shortcut) {
            self.form_error("Клавиша недоступна. Выберите другую.");
            return;
        }
        let old_startup = settings::startup_value();
        let startup_changed = startup != settings::launched_on_startup();
        let mut next = self.settings;
        next.shortcut = shortcut;
        if (startup_changed && !settings::set_startup(startup)) ||
            (next.shortcut != self.settings.shortcut && !next.save()) {
            if startup_changed { settings::set_startup_value(old_startup.as_deref()); }
            if let Some(old) = previous { self.set_hotkey(old); } else { self.clear_hotkey(); }
            self.form_error("Не удалось сохранить настройки.");
            return;
        }
        self.settings = next;
        self.show(WindowKind::Popup, self.anchor);
    }
    fn form_error(&mut self, text: &'static str) {
        if let Some(form) = self.form.as_mut() {
            form.capture = false;
            form.feedback = text;
        }
        self.draw_window();
    }
    fn clear_hotkey(&mut self) {
        if self.hotkey_id != 0 { unsafe { UnregisterHotKey(self.host,self.hotkey_id); } }
        self.hotkey_id = 0;
        self.hotkey = None;
    }

    fn scroll(&mut self, delta: i32) {
        if self.kind == WindowKind::Profiles && self.pressed.is_none() {
            self.profile_scroll(delta);
            return;
        }
        if self.kind != WindowKind::Popup || self.pressed.is_some() || !(self.editing_devices || self.other_expanded) {
            return;
        }
        let start = if delta < 0 { (self.start + 1).min(self.devices.len().saturating_sub(self.rows)) }
            else { self.start.saturating_sub(1) };
        if start != self.start {
            self.start = start;
            self.hover = None;
            self.focus = None;
            self.hide_hint();
            self.draw_window();
        }
    }
    fn move_focus(&mut self, step: i32) {
        let actions: Vec<_> = self.controls().into_iter().map(|(action,_)| action).collect();
        if actions.is_empty() { return; }
        if self.kind == WindowKind::Popup {
            if let Some(Action::Device(index)) = self.focus {
                let end = self.start + self.rows;
                if step > 0 && index + 1 == end && end < self.devices.len() {
                    self.start += 1;
                    self.focus = Some(Action::Device(index+1));
                    self.draw_window();
                    return;
                } else if step < 0 && index == self.start && index > 0 {
                    self.start -= 1;
                    self.focus = Some(Action::Device(index-1));
                    self.draw_window();
                    return;
                }
            }
        }
        let index = self.focus.and_then(|focus| actions.iter().position(|item| *item == focus))
            .map_or(if step < 0 {actions.len()-1} else {0}, |i| (i as i32 + step).rem_euclid(actions.len() as i32) as usize);
        self.focus = Some(actions[index]);
        self.draw_window();
    }

    fn key(&mut self, vk: u32) -> bool {
        if self.kind == WindowKind::Profiles {
            if self.profiles.capturing {
                match vk as u16 {
                    VK_ESCAPE => self.stop_shortcut_capture(),
                    VK_BACK => self.profile_shortcut(None),
                    VK_SHIFT | VK_CONTROL | VK_MENU | VK_LWIN | VK_RWIN | 0xa0..=0xa5 => (),
                    key => {
                        let mods = u32::from(down(VK_MENU)) | (u32::from(down(VK_CONTROL)) << 1)
                            | (u32::from(down(VK_SHIFT)) << 2);
                        if key != VK_F12 && (mods != 0 || (0x70..=0x87).contains(&key)) {
                            self.profile_shortcut(Some(Shortcut { key: vk, mods }));
                        }
                    }
                }
                return true;
            }
            if self.profiles.typing {
                if vk as u16 == VK_ESCAPE { self.profile_char(13); }
                return true;
            }
            if let Some(action) = self.focus.filter(|action| is_profile_slider(*action)) {
                if let Some(level) = self.profile_slider_level(action) {
                    let delta = match vk as u16 {
                        VK_LEFT | VK_DOWN => -1, VK_RIGHT | VK_UP => 1,
                        VK_PRIOR => 10, VK_NEXT => -10, VK_HOME => -100, VK_END => 100,
                        _ => 0,
                    };
                    if delta != 0 {
                        self.profile_slider(action,level+delta);
                        return true;
                    }
                }
            }
            if matches!(vk as u16,VK_PRIOR | VK_NEXT) {
                let delta = if vk as u16 == VK_NEXT { -1 } else { 1 };
                let rows = self.profile_model().rows();
                for _ in 0..rows { self.profile_scroll(delta); }
                return true;
            }
        }
        if self.kind == WindowKind::Settings && self.form.as_ref().is_some_and(|form| form.capture) {
            let form = self.form.as_mut().unwrap();
            if vk == u32::from(VK_ESCAPE) {
                form.capture = false; form.feedback = SHORTCUT_HINT;
            } else if matches!(vk, 0x10..=0x12 | 0x5b..=0x5c | 0xa0..=0xa5) { return true; }
            else {
                let mods = u32::from(down(VK_MENU)) | u32::from(down(VK_CONTROL)) << 1 | u32::from(down(VK_SHIFT)) << 2;
                if vk == u32::from(VK_F12) || (mods == 0 && !(0x70..=0x87).contains(&vk)) {
                    form.feedback = "Выберите F-клавишу (кроме F12) или сочетание.";
                } else {
                    form.shortcut = Shortcut { key: vk, mods };
                    form.capture = false;
                    form.feedback = SHORTCUT_HINT;
                }
            }
            self.draw_window();
            return true;
        }
        match vk as u16 {
            VK_ESCAPE if self.kind == WindowKind::Settings => self.invoke(Action::Back),
            VK_ESCAPE if self.kind == WindowKind::Profiles => self.profile_back(),
            VK_ESCAPE if self.kind == WindowKind::Popup && self.editing_devices => self.invoke(Action::EditDevices),
            VK_ESCAPE => self.hide(),
            VK_LEFT if self.kind == WindowKind::Settings && down(VK_MENU) => self.invoke(Action::Back),
            VK_TAB => self.move_focus(if down(VK_SHIFT) {-1} else {1}),
            VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT if self.focus == Some(Action::Volume) => {
                if let Some(volume) = self.volume {
                    let next = match vk as u16 { VK_LEFT | VK_DOWN => volume.level - 1, VK_RIGHT | VK_UP => volume.level + 1,
                        VK_HOME => 0, VK_END => 100, VK_PRIOR => volume.level + 10, _ => volume.level - 10 };
                    self.set_volume_level(next.clamp(0,100));
                }
            }
            VK_UP | VK_LEFT => self.move_focus(-1),
            VK_DOWN | VK_RIGHT => self.move_focus(1),
            VK_RETURN | VK_SPACE => if let Some(focus) = self.focus { self.invoke(focus); },
            _ => return false,
        }
        true
    }

    fn message(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if hwnd == self.host {
            match msg {
                TRAY_MESSAGE => match lp as u32 & 0xffff {
                    WM_MOUSEMOVE => self.show_volume_popup(),
                    WM_LBUTTONUP | NIN_KEYSELECT => self.switch_next(),
                    WM_MBUTTONUP => self.tray_mute(),
                    WM_RBUTTONUP | WM_CONTEXTMENU if self.kind == WindowKind::Settings => unsafe {
                        SetForegroundWindow(self.window.hwnd);
                    },
                    WM_RBUTTONUP | WM_CONTEXTMENU => self.show(WindowKind::Menu,cursor()),
                    _ => (),
                },
                TRAY_WHEEL => self.tray_wheel(wp as isize as i32),
                TRAY_MUTE => self.tray_mute(),
                WM_HOTKEY if self.hotkey_id != 0 && wp as i32 == self.hotkey_id => self.switch_next(),
                WM_HOTKEY => self.profile_hotkey(wp as i32),
                WM_SETTINGCHANGE | WM_THEMECHANGED => self.sync_theme(),
                audio::WM_ENDPOINTS => {
                    unsafe {
                        let mut ignored: MSG = zeroed();
                        while PeekMessageW(&mut ignored,self.host,audio::WM_ENDPOINTS,audio::WM_ENDPOINTS,PM_REMOVE) != 0 {}
                    }
                    self.profile_endpoint_changed();
                    if self.kind == WindowKind::Popup {
                        let old_height = self.height;
                        self.refresh_devices();
                        if old_height != self.window_model().height() { self.geometry(); }
                        self.draw_window();
                    } else {
                        self.refresh_volume();
                        if self.kind == WindowKind::Profiles { self.draw_window(); }
                    }
                }
                audio::WM_VOLUME => {
                    unsafe {
                        let mut ignored: MSG = zeroed();
                        while PeekMessageW(&mut ignored,self.host,audio::WM_VOLUME,audio::WM_VOLUME,PM_REMOVE) != 0 {}
                    }
                    self.refresh_after_tray_audio();
                }
                WM_TIMER if wp == TOAST_TIMER => {
                    unsafe { KillTimer(self.host, TOAST_TIMER); }
                    self.notice = None;
                    self.toast.hide();
                }
                WM_TIMER if wp == HINT_TIMER => self.show_hint(),
                WM_TIMER if wp == profile_app::TIMER => self.profile_tick(),
                WM_TIMER if wp == EDIT_ANIMATION_TIMER => {
                    if self.kind == WindowKind::Popup && self.editing_devices {
                        self.edit_frame += 1;
                        if self.edit_frame + 1 >= EDIT_WIGGLE.len() { self.stop_device_animation(); }
                        self.draw_window();
                    } else {
                        self.stop_device_animation();
                    }
                }
                WM_TIMER if wp == VOLUME_HOVER_TIMER => {
                    if self.volume_anchor.is_some_and(|bounds| !point_in_rect(cursor(), bounds)) {
                        self.hide_volume_popup();
                    }
                }
                WM_TIMER if wp == TRAY_TIMER => {
                    if self.update_tray(NIM_ADD, true) { unsafe { KillTimer(self.host, TRAY_TIMER); } }
                }
                WM_CLOSE => unsafe { PostQuitMessage(0); },
                id if self.taskbar_message != 0 && id == self.taskbar_message => {
                    self.hide_volume_popup();
                    self.tray_exists = false;
                    unsafe { RemovePropW(self.host, wide(TRAY_READY).as_ptr()); }
                    if self.update_tray(NIM_ADD, true) { unsafe { KillTimer(self.host, TRAY_TIMER); } }
                    else { unsafe { SetTimer(self.host, TRAY_TIMER, 1000, None); } }
                }
                _ => return None,
            }
            return Some(0);
        }
        if hwnd != self.window.hwnd || self.kind == WindowKind::Hidden { return None; }
        let (x,y) = logical_point(lp,self.scale);
        match msg {
            WM_ACTIVATE if wp & 0xffff == WA_INACTIVE as usize
                && self.kind != WindowKind::Settings
                && !(self.kind == WindowKind::Profiles && matches!(self.profiles.page,
                    profile_ui::Page::Edit | profile_ui::Page::Apps | profile_ui::Page::PickApps
                    | profile_ui::Page::Output | profile_ui::Page::Fallback)) => self.hide(),
            WM_CLOSE => self.hide(),
            WM_CHAR if self.kind == WindowKind::Profiles && self.profiles.typing => self.profile_char(wp as u16),
            WM_KEYDOWN | WM_SYSKEYDOWN => { if !self.key(wp as u32) { return None; } }
            WM_MOUSEWHEEL => self.scroll((wp >> 16) as i16 as i32),
            WM_MOUSEMOVE => {
                if self.pressed == Some(Action::Volume) { self.volume_at(x); return Some(0); }
                if let Some(action) = self.pressed.filter(|action| is_profile_slider(*action)) {
                    self.profile_slider(action,profile_ui::level(x));
                    return Some(0);
                }
                if !self.tracking_mouse {
                    let mut tracking = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    self.tracking_mouse = unsafe { TrackMouseEvent(&mut tracking) } != 0;
                }
                let item = self.hit(x,y);
                if item != self.hover {
                    self.hover = item;
                    self.hide_hint();
                    if matches!(item, Some(Action::Device(_) | Action::QuickDevice(_))) {
                        unsafe { SetTimer(self.host,HINT_TIMER,650,None); }
                    }
                    self.draw_window();
                }
                unsafe { SetCursor(LoadCursorW(null_mut(), if item.is_some() { IDC_HAND } else { IDC_ARROW })); }
            }
            WM_MOUSELEAVE => {
                self.tracking_mouse = false;
                if self.hover.take().is_some() { self.hide_hint(); self.draw_window(); }
            }
            WM_LBUTTONDOWN => {
                self.hide_hint();
                self.pressed = self.hit(x,y);
                self.focus = self.pressed.filter(|action| *action == Action::Volume || is_profile_slider(*action));
                if self.pressed.is_some() { unsafe { SetCapture(hwnd); } }
                if self.pressed == Some(Action::Volume) { self.volume_at(x); }
                if let Some(action) = self.pressed.filter(|action| is_profile_slider(*action)) {
                    self.profile_slider(action,profile_ui::level(x));
                }
            }
            WM_LBUTTONUP => {
                let pressed = self.pressed.take();
                if pressed == Some(Action::Volume) { self.volume_at(x); }
                if let Some(action) = pressed.filter(|action| is_profile_slider(*action)) {
                    self.profile_slider(action,profile_ui::level(x));
                }
                unsafe { if GetCapture() == hwnd { ReleaseCapture(); } }
                let hit = self.hit(x,y);
                if let Some(action) = pressed.filter(|item| Some(*item) == hit && *item != Action::Volume && !is_profile_slider(*item)) {
                    self.invoke(action);
                }
            }
            WM_CAPTURECHANGED => { self.pressed = None; }
            WM_DPICHANGED => {
                self.scale = ((wp & 0xffff) as f32 / 96.).max(1.);
                (self.area, _) = screen(self.anchor);
                self.geometry();
                self.draw_window();
            }
            _ => return None,
        }
        Some(0)
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.audio.stop();
        self.clear_hotkey();
        unsafe {
            if self.tray_exists {
                let mut data: NOTIFYICONDATAW = zeroed();
                data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                data.hWnd = self.host;
                data.uID = 1;
                Shell_NotifyIconW(NIM_DELETE, &data);
            }
            if !self.icon.is_null() { DestroyIcon(self.icon); }
            RemovePropW(self.host, wide(TRAY_READY).as_ptr());
            DestroyWindow(self.host);
        }
    }
}

fn cursor() -> POINT {
    let mut point = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut point); }
    point
}
fn is_profile_slider(action: Action) -> bool {
    matches!(action, Action::ProfileMaster | Action::ProfileFallbackLevel | Action::AppVolume(_))
}
fn ceil_pixels(value: f32) -> i32 {
    let whole = value as i32;
    whole + i32::from(value > whole as f32)
}
fn down(key: u16) -> bool { unsafe { GetKeyState(key as i32) < 0 } }
fn logical_point(packed: LPARAM, scale: f32) -> (f32,f32) {
    ((packed as u16 as i16) as f32 / scale, ((packed >> 16) as u16 as i16) as f32 / scale)
}
fn screen(point: POINT) -> (RECT,f32) {
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        let ok = GetMonitorInfoW(monitor, &mut info) != 0;
        let mut x = 96u32;
        let mut y = 96u32;
        let scale = if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) == 0 {
            (x as f32 / 96.).clamp(0.75,5.)
        } else { 1. };
        if ok { (info.rcWork,scale) } else {
            (RECT { left: 0, top: 0, right:GetSystemMetrics(SM_CXSCREEN),bottom:GetSystemMetrics(SM_CYSCREEN) },scale)
        }
    }
}
