use alloc::{format, string::{String, ToString}, vec::Vec};
use core::ptr::null_mut;
use windows_sys::Win32::System::Registry::*;
use crate::{runtime, wide};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode { System, Light, Dark }

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Shortcut { pub key: u32, pub mods: u32 }

#[derive(Clone, Copy)]
pub struct Settings { pub shortcut: Shortcut, pub theme: Mode }

const CONTROL: i32 = 0x20000;
const ALT: i32 = 0x40000;
const SHIFT: i32 = 0x10000;
const PERSONALIZE: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const ENTRY: &str = "SoftAudioSwitch";
const DEVICES: &str = "Software\\SoftAudioSwitch\\Devices";

fn read_device_value(kind: &str, id: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut bytes = 4u32;
    let name = wide(&format!("{kind}:{id}"));
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, wide(DEVICES).as_ptr(), name.as_ptr(),
        RRF_RT_REG_DWORD, null_mut(), (&mut value as *mut u32).cast(), &mut bytes) };
    (status == 0).then_some(value)
}

fn write_device_value(kind: &str, id: &str, value: u32) -> bool {
    let name = wide(&format!("{kind}:{id}"));
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, wide(DEVICES).as_ptr(), name.as_ptr(),
        REG_DWORD, (&value as *const u32).cast(), 4) == 0 }
}

pub fn device_hidden(id: &str) -> bool { read_device_value("Hidden", id) == Some(1) }

pub fn set_device_hidden(id: &str, hidden: bool) -> bool {
    write_device_value("Hidden", id, u32::from(hidden))
}

pub fn device_volume(id: &str) -> Option<f32> {
    read_device_value("Volume", id).map(f32::from_bits).filter(|value| (0.0..=1.0).contains(value))
}

pub fn remember_volume(id: &str, scalar: f32) {
    if (0.0..=1.0).contains(&scalar) && device_volume(id) != Some(scalar) {
        write_device_value("Volume", id, scalar.to_bits());
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut settings = Self { shortcut: Shortcut { key: 0x78, mods: 3 }, theme: Mode::System };
        let Some(text) = runtime::read_settings() else { return settings };
        let read = |name: &str| -> Option<i32> {
            let field = format!("\"{name}\"");
            let (_, suffix) = text.split_once(&field)?;
            let value = suffix.trim_start().strip_prefix(':')?.trim_start();
            let end = value.find(|c: char| c == ',' || c == '}').unwrap_or(value.len());
            value[..end].trim().parse().ok()
        };
        if let (Some(key @ 1..=254), Some(mods)) = (read("HotkeyKey"), read("HotkeyModifiers")) {
            if mods & !(CONTROL | ALT | SHIFT) == 0 {
                settings.shortcut = Shortcut { key: key as u32, mods:
                    u32::from(mods & ALT != 0) | (u32::from(mods & CONTROL != 0) << 1) | (u32::from(mods & SHIFT != 0) << 2) };
            }
        }
        settings.theme = match read("ThemeMode") { Some(1) => Mode::Light, Some(2) => Mode::Dark, _ => Mode::System };
        settings
    }

    pub fn save(self) -> bool {
        let mods = (if self.shortcut.mods & 1 != 0 { ALT } else { 0 })
            | (if self.shortcut.mods & 2 != 0 { CONTROL } else { 0 })
            | (if self.shortcut.mods & 4 != 0 { SHIFT } else { 0 });
        runtime::write_settings(&format!(
            "{{\n  \"HotkeyKey\": {},\n  \"HotkeyModifiers\": {},\n  \"ThemeMode\": {}\n}}",
            self.shortcut.key, mods, self.theme as u8))
    }
}

impl Shortcut {
    pub fn labels(self) -> Vec<String> {
        let mut labels = Vec::with_capacity(4);
        if self.mods & 2 != 0 { labels.push("Ctrl".into()); }
        if self.mods & 1 != 0 { labels.push("Alt".into()); }
        if self.mods & 4 != 0 { labels.push("Shift".into()); }
        let key = match self.key {
            0x70..=0x87 => format!("F{}", self.key - 0x6F),
            0x30..=0x39 | 0x41..=0x5A => char::from_u32(self.key).unwrap().to_string(),
            vk => {
                use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyNameTextW, MapVirtualKeyW};
                let mut name = [0u16; 64];
                let scan = unsafe { MapVirtualKeyW(vk, 4) };
                let code = ((scan & 0xff) << 16) | (u32::from(scan & 0xff00 != 0) << 24);
                let count = unsafe { GetKeyNameTextW(code as i32, name.as_mut_ptr(), name.len() as i32) };
                if count > 0 { String::from_utf16_lossy(&name[..count as usize]) } else { format!("VK {vk}") }
            }
        };
        labels.push(key);
        labels
    }
}

fn read_preference(value: &str) -> Option<bool> {
    let mut data = 0u32;
    let mut bytes = 4u32;
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, wide(PERSONALIZE).as_ptr(), wide(value).as_ptr(),
        RRF_RT_REG_DWORD, null_mut(), (&mut data as *mut u32).cast(), &mut bytes) };
    (status == 0).then_some(data != 0)
}
pub fn dark(mode: Mode) -> bool {
    match mode { Mode::System => read_preference("AppsUseLightTheme") == Some(false),
        Mode::Light => false, Mode::Dark => true }
}
pub fn taskbar_dark() -> bool { read_preference("SystemUsesLightTheme") != Some(true) }

pub fn startup_value() -> Option<String> {
    let mut buffer = [0u16; 8192];
    let mut bytes = (buffer.len() * 2) as u32;
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), wide(ENTRY).as_ptr(),
        RRF_RT_REG_SZ, null_mut(), buffer.as_mut_ptr().cast(), &mut bytes) };
    if status != 0 { return None; }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

pub fn launched_on_startup() -> bool {
    runtime::executable_path().zip(startup_value()).is_some_and(|(exe, value)|
        value.trim().trim_matches('"').eq_ignore_ascii_case(&exe))
}

pub fn set_startup_value(value: Option<&str>) -> bool {
    let branch = wide(RUN);
    let entry = wide(ENTRY);
    unsafe {
        match value {
            Some(command) => {
                let chars = wide(command);
                RegSetKeyValueW(HKEY_CURRENT_USER, branch.as_ptr(), entry.as_ptr(), REG_SZ, chars.as_ptr().cast(),
                    (chars.len() * 2) as u32) == 0
            }
            None => matches!(RegDeleteKeyValueW(HKEY_CURRENT_USER, branch.as_ptr(), entry.as_ptr()), 0 | 2),
        }
    }
}

pub fn set_startup(enabled: bool) -> bool {
    if !enabled { return set_startup_value(None); }
    runtime::executable_path().is_some_and(|exe| set_startup_value(Some(&format!("\"{exe}\""))))
}
