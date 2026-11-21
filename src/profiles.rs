use alloc::{string::String, vec::Vec};
use crate::{audio::Volume, locale::tr, runtime, settings::Shortcut};

pub const LIMIT: usize = 32;
pub const APP_LIMIT: usize = 128;
const FILE: &str = "\\profiles.dat";

#[derive(Clone)]
pub struct Output {
    pub id: String,
    pub name: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AppLevel {
    pub key: String,
    pub name: String,
    pub volume: Volume,
}

impl AppLevel {
    pub fn display_name(&self) -> &str {
        if self.key == "system" && matches!(self.name.as_str(),"Системные звуки" | "System sounds") {
            tr("Системные звуки")
        } else { &self.name }
    }
}

#[derive(Clone)]
pub struct Profile {
    pub name: String,
    pub output: Output,
    pub volume: Volume,
    pub apps: Vec<AppLevel>,
    pub fallback: Option<Output>,
    pub fallback_level: u8,
    pub reconnect: bool,
    pub shortcut: Option<Shortcut>,
}

pub fn load() -> Vec<Profile> {
    runtime::read_data(FILE, 16 * 1024 * 1024)
        .and_then(|data| decode(&data)).unwrap_or_default()
}

pub fn save(profiles: &[Profile]) -> bool {
    let mut data = b"SSP1".to_vec();
    data.push(profiles.len() as u8);
    for profile in profiles {
        put_string(&mut data, &profile.name);
        put_output(&mut data, &profile.output);
        put_volume(&mut data, profile.volume);
        data.push(profile.fallback_level);
        data.push(u8::from(profile.reconnect));
        data.push(profile.shortcut.map_or(0, |shortcut| shortcut.key as u8));
        data.push(profile.shortcut.map_or(0, |shortcut| shortcut.mods as u8));
        data.push(u8::from(profile.fallback.is_some()));
        if let Some(output) = &profile.fallback { put_output(&mut data, output); }
        data.push(profile.apps.len() as u8);
        for app in &profile.apps {
            put_string(&mut data, &app.key);
            put_string(&mut data, &app.name);
            put_volume(&mut data, app.volume);
        }
    }
    runtime::write_data(FILE, "\\profiles.tmp", &data)
}

fn put_string(data: &mut Vec<u8>, text: &str) {
    data.extend_from_slice(&(text.len() as u32).to_le_bytes());
    data.extend_from_slice(text.as_bytes());
}

fn put_output(data: &mut Vec<u8>, output: &Output) {
    put_string(data, &output.id);
    put_string(data, &output.name);
}

fn put_volume(data: &mut Vec<u8>, volume: Volume) {
    data.extend([volume.level.clamp(0, 100) as u8, u8::from(volume.muted)]);
}

fn decode(data: &[u8]) -> Option<Vec<Profile>> {
    let mut read = Reader(data.strip_prefix(b"SSP1")?);
    let count = read.byte()? as usize;
    if count > LIMIT { return None; }
    let mut profiles = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read.string()?;
        let output = read.output()?;
        let volume = read.volume()?;
        let fallback_level = read.level()?;
        let reconnect = read.flag()?;
        let key = read.byte()?;
        let mods = read.byte()?;
        if mods & !7 != 0 || key == 255 { return None; }
        let shortcut = (key != 0).then_some(Shortcut { key: key.into(), mods: mods.into() });
        let fallback = if read.flag()? { Some(read.output()?) } else { None };
        let count = read.byte()? as usize;
        if count > APP_LIMIT { return None; }
        let mut apps = Vec::with_capacity(count);
        for _ in 0..count {
            apps.push(AppLevel { key: read.string()?, name: read.string()?, volume: read.volume()? });
        }
        profiles.push(Profile { name, output, volume, apps, fallback, fallback_level, reconnect, shortcut });
    }
    read.0.is_empty().then_some(profiles)
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(length)?;
        self.0 = rest;
        Some(head)
    }
    fn byte(&mut self) -> Option<u8> { Some(self.take(1)?[0]) }
    fn flag(&mut self) -> Option<bool> {
        match self.byte()? { 0 => Some(false), 1 => Some(true), _ => None }
    }
    fn level(&mut self) -> Option<u8> {
        let level = self.byte()?;
        (level <= 100).then_some(level)
    }
    fn volume(&mut self) -> Option<Volume> {
        Some(Volume { level: self.level()?.into(), muted: self.flag()? })
    }
    fn string(&mut self) -> Option<String> {
        let length = u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize;
        if length > 131072 { return None; }
        Some(core::str::from_utf8(self.take(length)?).ok()?.into())
    }
    fn output(&mut self) -> Option<Output> {
        Some(Output { id: self.string()?, name: self.string()? })
    }
}
