use alloc::{boxed::Box, format, string::String, vec, vec::Vec};
use core::{ffi::c_void, ptr::null_mut, sync::atomic::{AtomicU32, Ordering}};
use windows_sys::{core::GUID, Win32::{Foundation::{CloseHandle, HWND}, System::{
    Com::{CoCreateInstance, CoTaskMemFree}, Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION}},
    UI::WindowsAndMessaging::PostMessageW}};
use crate::{locale::tr, profiles::AppLevel, settings, wide};

pub const WM_ENDPOINTS: u32 = 0x8031;
pub const WM_VOLUME: u32 = 0x8032;
const RENDER: i32 = 0;
const MULTIMEDIA: i32 = 1;
const CLSCTX_ALL: u32 = 23;
const NO_ENDPOINT: i32 = 0x80070490_u32 as i32;
const ENUMERATOR_CLASS: GUID = guid(0xBCDE0395_E52F_467C_8E3D_C4579291692E);
const ENUMERATOR_IID: GUID = guid(0xA95664D2_9614_4F35_A746_DE8DB63617E6);
const POLICY_CLASS: GUID = guid(0x870AF99C_171D_4F9E_AF0D_E63DF40C2BC9);
const POLICY_IID: GUID = guid(0xF8679F50_850A_41CF_9C72_430F290290C8);
const VOLUME_IID: GUID = guid(0x5CDF2C82_841E_4546_9722_0CF74078229A);
const SESSION_MANAGER_IID: GUID = guid(0x77AA99A0_1BD6_484F_8BC7_2C654C9A9B6F);
const SESSION_CONTROL_IID: GUID = guid(0xBFB7FF88_7239_4FC9_8FA2_07C950BE9C6D);
const SESSION_VOLUME_IID: GUID = guid(0x87CE5498_68D6_44E5_9215_6DA47EF883D8);
const NOTIFY_IID: GUID = guid(0x7991EEC9_7E89_4D85_8390_6C703CEC60C0);
const VOLUME_NOTIFY_IID: GUID = guid(0x657804FA_D6AD_4496_8A60_352752AF4F89);
const IUNKNOWN: GUID = guid(0x00000000_0000_0000_C000_000000000046);
const OWN_VOLUME: GUID = guid(0x21E22B88_9DF1_4287_AE8B_49101C760735);
const NAME_PROPERTY: PropertyKey = PropertyKey { guid: guid(0xA45C254E_DF1C_4EFD_8020_67D146A850E0), id: 14 };

const fn guid(value: u128) -> GUID {
    GUID { data1: (value >> 96) as u32, data2: (value >> 80) as u16, data3: (value >> 64) as u16,
        data4: (value as u64).to_be_bytes() }
}
fn same(left: &GUID, right: &GUID) -> bool {
    left.data1 == right.data1 && left.data2 == right.data2 && left.data3 == right.data3 && left.data4 == right.data4
}
#[repr(C)]
struct PropertyKey { guid: GUID, id: u32 }
#[repr(C, align(8))]
struct Variant([u64; 3]);
#[link(name = "ole32")]
extern "system" { fn PropVariantClear(value: *mut Variant) -> i32; }

type Raw = *mut c_void;
type Result<T> = core::result::Result<T, i32>;
fn checked(hr: i32) -> Result<()> { if hr < 0 { Err(hr) } else { Ok(()) } }

// These slots correspond to the COM ABI including the three IUnknown methods.
macro_rules! call {
    ($obj:expr, $slot:expr, [$($arg_ty:ty),*] [$($arg:expr),*], $ret:ty) => {{
        let object = $obj;
        let vtable = *(object as *const *const *const ());
        let func: unsafe extern "system" fn(Raw, $($arg_ty),*) -> $ret =
            core::mem::transmute(*vtable.add($slot));
        func(object, $($arg),*)
    }};
}

struct Com(Raw);
impl Com {
    fn create(class: &GUID, iid: &GUID) -> Result<Self> {
        let mut raw = null_mut();
        checked(unsafe { CoCreateInstance(class, null_mut(), CLSCTX_ALL, iid, &mut raw) })?;
        Self::from_raw(raw)
    }
    fn from_raw(raw: Raw) -> Result<Self> {
        if raw.is_null() { Err(0x80004003_u32 as i32) } else { Ok(Self(raw)) }
    }
    fn query(&self, iid: &GUID) -> Result<Self> {
        let mut raw = null_mut();
        checked(unsafe { call!(self.0, 0, [*const GUID, *mut Raw] [iid, &mut raw], i32) })?;
        Self::from_raw(raw)
    }
}
impl Drop for Com {
    fn drop(&mut self) { unsafe { call!(self.0, 2, [] [], u32); } }
}

#[derive(Clone, Debug)]
pub struct Device { pub id: String, pub name: String, pub selected: bool }
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Volume { pub level: i32, pub muted: bool }

pub struct Session {
    pub value: AppLevel,
    pub instance: String,
    pub active: bool,
    interface: Com,
}
impl Session {
    pub fn set(&self, level: Option<i32>, muted: Option<bool>) -> bool {
        let interface = self.interface.0;
        let mute = |value| checked(unsafe { call!(interface, 5, [i32, *const GUID] [i32::from(value), &OWN_VOLUME], i32) }).is_ok();
        if muted == Some(true) && !mute(true) { return false; }
        if let Some(level) = level {
            let scalar = level.clamp(0, 100) as f32 / 100.;
            if unsafe { call!(interface, 3, [f32, *const GUID] [scalar, &OWN_VOLUME], i32) } < 0 { return false; }
        }
        muted != Some(false) || mute(false)
    }
}

struct EndpointVolume { id: String, interface: Com, notify: Option<*mut Callback> }
impl Drop for EndpointVolume {
    fn drop(&mut self) {
        if let Some(notification) = self.notify.take() {
            unsafe {
                call!(self.interface.0, 4, [*mut Callback] [notification], i32);
                release(notification);
            }
        }
    }
}

#[derive(Default)]
pub struct Audio {
    enumerator: Option<Com>,
    volume: Option<EndpointVolume>,
    notification: Option<*mut Callback>,
    watching: Option<HWND>,
}

impl Audio {
    fn enumerator(&mut self) -> Result<Raw> {
        if self.enumerator.is_none() { self.enumerator = Some(Com::create(&ENUMERATOR_CLASS, &ENUMERATOR_IID)?); }
        Ok(self.enumerator.as_ref().unwrap().0)
    }

    fn default_device(&mut self, role: i32) -> Result<Com> {
        let enumerator = self.enumerator()?;
        let mut raw: Raw = null_mut();
        checked(unsafe { call!(enumerator, 4, [i32, i32, *mut Raw] [RENDER, role, &mut raw], i32) })?;
        Com::from_raw(raw)
    }
    pub fn default_id(&mut self, role: i32) -> Result<Option<String>> {
        match self.default_device(role) {
            Ok(device) => id(&device).map(Some),
            Err(NO_ENDPOINT) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub fn devices(&mut self) -> Result<Vec<Device>> {
        let enumerator = self.enumerator()?;
        let mut raw: Raw = null_mut();
        checked(unsafe { call!(enumerator, 3, [i32, u32, *mut Raw] [RENDER, 1, &mut raw], i32) })?;
        let collection = Com::from_raw(raw)?;
        let mut count = 0u32;
        checked(unsafe { call!(collection.0, 3, [*mut u32] [&mut count], i32) })?;
        let selected = self.default_id(MULTIMEDIA)?;
        let mut devices = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut raw: Raw = null_mut();
            if unsafe { call!(collection.0, 4, [u32, *mut Raw] [index, &mut raw], i32) } < 0 { continue }
            let Ok(device) = Com::from_raw(raw) else { continue };
            let Ok(id) = id(&device) else { continue };
            let name = friendly_name(&device).unwrap_or_else(|| tr("Устройство вывода").into());
            let active = selected.as_ref().is_some_and(|current| current.eq_ignore_ascii_case(&id));
            devices.push(Device { id, name, selected: active });
        }
        devices.sort_by_cached_key(|device| (device.name.to_lowercase(), device.id.to_ascii_lowercase()));
        Ok(devices)
    }
    pub fn select(&mut self, target: &str) -> Result<bool> {
        self.select_with_volume(target, None)
    }
    pub fn select_with_volume(&mut self, target: &str, volume: Option<Volume>) -> Result<bool> {
        let roles = [0, 2, 1];
        let previous = [self.default_id(0)?, self.default_id(2)?, self.default_id(1)?];
        let policy = Com::create(&POLICY_CLASS, &POLICY_IID)?;
        let encoded = wide(target);
        if let (Some(current), Ok(endpoint)) = (&previous[2], self.endpoint_volume()) {
            if self.volume.as_ref().is_some_and(|volume| volume.id.eq_ignore_ascii_case(current)) {
                if let Ok(scalar) = volume_scalar(endpoint) { settings::remember_volume(current, scalar); }
            }
        }
        let saved = volume.map(|volume| volume.level.clamp(0,100) as f32 / 100.)
            .or_else(|| settings::device_volume(target));
        if let Some(saved) = saved {
            let device = self.device_by_id(target)?;
            let endpoint = activate_volume(&device)?;
            if volume.is_some_and(|volume| volume.muted) { set_mute(endpoint.0, true)?; }
            // Prepare the target before changing any default role.
            if volume_scalar(endpoint.0)? != saved {
                set_scalar(endpoint.0, saved)?;
            }
            if volume.is_some_and(|volume| !volume.muted) { set_mute(endpoint.0, false)?; }
        }
        for (index, role) in roles.into_iter().enumerate() {
            if previous[index].as_ref().is_some_and(|old| old.eq_ignore_ascii_case(target)) { continue }
            let result = unsafe { call!(policy.0, 13, [*const u16, i32] [encoded.as_ptr(), role], i32) };
            if result < 0 {
                for (role, old) in roles[..index].iter().copied().zip(&previous).rev() {
                    if let Some(old) = old.as_ref().filter(|old| !old.eq_ignore_ascii_case(target)) {
                        unsafe { call!(policy.0, 13, [*const u16, i32] [wide(old).as_ptr(), role], i32); }
                    }
                }
                return Err(result);
            }
        }
        self.default_id(MULTIMEDIA).map(|current| current.is_some_and(|id| id.eq_ignore_ascii_case(target)))
    }
    fn device_by_id(&mut self, id: &str) -> Result<Com> {
        let enumerator = self.enumerator()?;
        let mut raw = null_mut();
        checked(unsafe { call!(enumerator, 5, [*const u16, *mut Raw] [wide(id).as_ptr(), &mut raw], i32) })?;
        Com::from_raw(raw)
    }

    pub fn sessions(&mut self, output: Option<&str>) -> Result<Vec<Session>> {
        let device = match output {
            Some(id) => self.device_by_id(id)?,
            None => self.default_device(MULTIMEDIA)?,
        };
        let manager = activate(&device, &SESSION_MANAGER_IID)?;
        let mut raw = null_mut();
        checked(unsafe { call!(manager.0, 5, [*mut Raw] [&mut raw], i32) })?;
        let sessions = Com::from_raw(raw)?;
        let mut count = 0i32;
        checked(unsafe { call!(sessions.0, 3, [*mut i32] [&mut count], i32) })?;
        let mut result = Vec::new();
        for index in 0..count {
            let mut raw = null_mut();
            if unsafe { call!(sessions.0, 4, [i32, *mut Raw] [index, &mut raw], i32) } < 0 { continue; }
            if let Ok(control) = Com::from_raw(raw) {
                if let Some(session) = read_session(control) { result.push(session); }
            }
        }
        Ok(result)
    }
    pub fn watch(&mut self, hwnd: HWND) {
        self.stop();
        self.watching = Some(hwnd);
        if let Ok(enumerator) = self.enumerator() {
            let notify = Callback::new(hwnd, false);
            if unsafe { call!(enumerator, 6, [*mut Callback] [notify], i32) } >= 0 {
                self.notification = Some(notify);
            } else { unsafe { release(notify); } }
        }
    }
    pub fn stop(&mut self) {
        self.watching = None;
        self.volume = None;
        if let Some(notify) = self.notification.take() {
            unsafe {
                if let Some(owner) = &self.enumerator { call!(owner.0, 7, [*mut Callback] [notify], i32); }
                release(notify);
            }
        }
    }
    fn endpoint_volume(&mut self) -> Result<Raw> {
        let device = self.default_device(MULTIMEDIA)?;
        let id = id(&device)?;
        if let Some(volume) = &self.volume {
            if volume.id.eq_ignore_ascii_case(&id) { return Ok(volume.interface.0) }
        }
        self.volume = None;
        let interface = activate_volume(&device)?;
        let raw = interface.0;
        let notify = self.watching.and_then(|hwnd| {
            let callback = Callback::new(hwnd, true);
            if unsafe { call!(interface.0, 3, [*mut Callback] [callback], i32) } >= 0 { Some(callback) }
            else { unsafe { release(callback); } None }
        });
        self.volume = Some(EndpointVolume { id, interface, notify });
        Ok(raw)
    }
    pub fn volume(&mut self) -> Result<Volume> {
        let endpoint = self.endpoint_volume()?;
        let result = (|| {
            let scalar = volume_scalar(endpoint)?;
            let mut muted = 0i32;
            checked(unsafe { call!(endpoint, 15, [*mut i32] [&mut muted], i32) })?;
            Ok(Volume { level: (scalar * 100. + 0.5).clamp(0., 100.) as i32, muted: muted != 0 })
        })();
        if result.is_err() { self.volume = None; }
        result
    }
    pub fn set_volume(&mut self, level: i32) -> Result<()> {
        let endpoint = self.endpoint_volume()?;
        let scalar = level.clamp(0,100) as f32 / 100.;
        let result = set_scalar(endpoint, scalar);
        if result.is_ok() {
            if let Some(volume) = &self.volume {
                settings::remember_volume(&volume.id, volume_scalar(endpoint).unwrap_or(scalar));
            }
        } else { self.volume = None; }
        result
    }
    pub fn toggle_mute(&mut self) -> Result<()> {
        let endpoint = self.endpoint_volume()?;
        let mut muted = 0;
        checked(unsafe { call!(endpoint, 15, [*mut i32] [&mut muted], i32) })?;
        set_mute(endpoint, muted == 0)
    }
    pub fn mute_output(&mut self) -> Result<()> {
        set_mute(self.endpoint_volume()?, true)
    }
}
impl Drop for Audio { fn drop(&mut self) { self.stop(); } }

fn activate_volume(device: &Com) -> Result<Com> {
    activate(device, &VOLUME_IID)
}
fn activate(device: &Com, iid: &GUID) -> Result<Com> {
    let mut raw = null_mut();
    checked(unsafe { call!(device.0, 3, [*const GUID, u32, Raw, *mut Raw]
        [iid, CLSCTX_ALL, null_mut(), &mut raw], i32) })?;
    Com::from_raw(raw)
}

fn volume_scalar(endpoint: Raw) -> Result<f32> {
    let mut scalar = 0.0;
    checked(unsafe { call!(endpoint, 9, [*mut f32] [&mut scalar], i32) })?;
    Ok(scalar)
}

fn set_scalar(endpoint: Raw, scalar: f32) -> Result<()> {
    checked(unsafe { call!(endpoint, 7, [f32, *const GUID] [scalar, &OWN_VOLUME], i32) })
}
fn set_mute(endpoint: Raw, muted: bool) -> Result<()> {
    checked(unsafe { call!(endpoint, 14, [i32, *const GUID] [i32::from(muted), &OWN_VOLUME], i32) })
}

fn read_session(control: Com) -> Option<Session> {
    let mut state = 0i32;
    checked(unsafe { call!(control.0, 3, [*mut i32] [&mut state], i32) }).ok()?;
    if state == 2 { return None; }
    let control2 = control.query(&SESSION_CONTROL_IID).ok()?;
    let system = unsafe { call!(control2.0, 15, [] [], i32) } == 0;
    let mut pid = 0u32;
    checked(unsafe { call!(control2.0, 14, [*mut u32] [&mut pid], i32) }).ok()?;
    let mut pointer = null_mut();
    checked(unsafe { call!(control2.0, 13, [*mut *mut u16] [&mut pointer], i32) }).ok()?;
    let instance = unsafe {
        let value = decode(pointer);
        CoTaskMemFree(pointer.cast());
        value?
    };
    let (key, name) = if system {
        ("system".into(), tr("Системные звуки").into())
    } else if let Some(path) = process_path(pid) {
        let file = path.rsplit('\\').next().unwrap_or(&path);
        let name = file.strip_suffix(".exe").unwrap_or(file).into();
        (path.to_lowercase(), name)
    } else {
        // Unknown processes stay usable in the live mixer, but PID identities are not saved.
        (format!("pid:{pid}"), format!("{} · {pid}",tr("Приложение")))
    };
    let interface = control.query(&SESSION_VOLUME_IID).ok()?;
    let mut level = 0f32;
    let mut muted = 0i32;
    checked(unsafe { call!(interface.0, 4, [*mut f32] [&mut level], i32) }).ok()?;
    checked(unsafe { call!(interface.0, 6, [*mut i32] [&mut muted], i32) }).ok()?;
    let volume = Volume { level: (level * 100. + 0.5).clamp(0.,100.) as i32, muted: muted != 0 };
    Some(Session { value: AppLevel { key, name, volume }, instance, active: state == 1, interface })
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() { return None; }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let result = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        CloseHandle(process);
        (result != 0).then(|| String::from_utf16_lossy(&path[..length as usize]))
    }
}

unsafe fn decode(pointer: *const u16) -> Option<String> {
    if pointer.is_null() { return None }
    let mut length = 0;
    while *pointer.add(length) != 0 { length += 1; }
    Some(String::from_utf16_lossy(core::slice::from_raw_parts(pointer, length)))
}
fn id(device: &Com) -> Result<String> {
    let mut pointer: *mut u16 = null_mut();
    checked(unsafe { call!(device.0, 5, [*mut *mut u16] [&mut pointer], i32) })?;
    unsafe {
        let decoded = decode(pointer);
        CoTaskMemFree(pointer.cast());
        decoded.ok_or(0x80004003_u32 as i32)
    }
}
fn friendly_name(device: &Com) -> Option<String> {
    let mut raw: Raw = null_mut();
    checked(unsafe { call!(device.0, 4, [u32, *mut Raw] [0, &mut raw], i32) }).ok()?;
    let properties = Com::from_raw(raw).ok()?;
    let mut variant = Variant([0; 3]);
    let result = unsafe { call!(properties.0, 5, [*const PropertyKey, *mut Variant] [&NAME_PROPERTY, &mut variant], i32) };
    unsafe {
        let text = if result >= 0 && matches!(variant.0[0] as u16, 31 | 8) { decode(variant.0[1] as *const u16) } else { None };
        PropVariantClear(&mut variant);
        text
    }
}

#[repr(C)]
struct Callback { vtable: *const (), refs: AtomicU32, hwnd: HWND, volume: bool }
impl Callback {
    fn new(hwnd: HWND, volume: bool) -> *mut Self {
        let vtable = if volume { (&VOLUME_CALLBACK as *const VolumeVtable).cast() }
            else { (&DEVICE_CALLBACK as *const DeviceVtable).cast() };
        Box::into_raw(Box::new(Self { vtable, refs: AtomicU32::new(1), hwnd, volume }))
    }
}
#[repr(C)]
struct UnknownVtable {
    query: unsafe extern "system" fn(*mut Callback, *const GUID, *mut Raw) -> i32,
    add: unsafe extern "system" fn(*mut Callback) -> u32,
    release: unsafe extern "system" fn(*mut Callback) -> u32,
}
#[repr(C)]
struct DeviceVtable {
    unknown: UnknownVtable,
    state: unsafe extern "system" fn(*mut Callback, *const u16, u32) -> i32,
    added: unsafe extern "system" fn(*mut Callback, *const u16) -> i32,
    removed: unsafe extern "system" fn(*mut Callback, *const u16) -> i32,
    default: unsafe extern "system" fn(*mut Callback, i32, i32, *const u16) -> i32,
    property: unsafe extern "system" fn(*mut Callback, *const u16, PropertyKey) -> i32,
}
#[repr(C)]
struct VolumeVtable { unknown: UnknownVtable, notify: unsafe extern "system" fn(*mut Callback, *const GUID) -> i32 }
const UNKNOWN: UnknownVtable = UnknownVtable { query, add, release };
static DEVICE_CALLBACK: DeviceVtable = DeviceVtable { unknown: UNKNOWN, state, added: changed, removed: changed,
    default: default_changed, property };
static VOLUME_CALLBACK: VolumeVtable = VolumeVtable { unknown: UNKNOWN, notify: volume_changed };
unsafe extern "system" fn query(this: *mut Callback, iid: *const GUID, out: *mut Raw) -> i32 {
    if out.is_null() || iid.is_null() { return 0x80004003_u32 as i32; }
    let supported = if (*this).volume { &VOLUME_NOTIFY_IID } else { &NOTIFY_IID };
    if same(&*iid, &IUNKNOWN) || same(&*iid, supported) {
        *out = this.cast(); add(this); 0
    } else { *out = null_mut(); 0x80004002_u32 as i32 }
}
unsafe extern "system" fn add(this: *mut Callback) -> u32 { (*this).refs.fetch_add(1, Ordering::Relaxed) + 1 }
unsafe extern "system" fn release(this: *mut Callback) -> u32 {
    let count = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if count == 0 { drop(Box::from_raw(this)); }
    count
}
unsafe extern "system" fn changed(this: *mut Callback, _: *const u16) -> i32 { PostMessageW((*this).hwnd, WM_ENDPOINTS, 0, 0); 0 }
unsafe extern "system" fn state(this: *mut Callback, id: *const u16, _: u32) -> i32 { changed(this, id) }
unsafe extern "system" fn default_changed(this: *mut Callback, flow: i32, role: i32, id: *const u16) -> i32 {
    if flow == RENDER && role == MULTIMEDIA { changed(this, id); } 0
}
unsafe extern "system" fn property(this: *mut Callback, id: *const u16, property: PropertyKey) -> i32 {
    if property.id == NAME_PROPERTY.id && same(&property.guid, &NAME_PROPERTY.guid) { changed(this, id); } 0
}
unsafe extern "system" fn volume_changed(this: *mut Callback, context: *const GUID) -> i32 {
    if !context.is_null() && !same(&*context, &OWN_VOLUME) { PostMessageW((*this).hwnd, WM_VOLUME, 0, 0); } 0
}
