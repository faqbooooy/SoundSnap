use super::*;
use alloc::{collections::BTreeMap, sync::Arc};
use core::ptr::null;
use windows_sys::Win32::{Foundation::HANDLE, System::Threading::{CreateEventW, SetEvent}};
use crate::sync::Lock;

const CREATED_IID: GUID = guid(0xDCA5616C_FF2B_4EF8_BA1E_208AEB22A1B0);
const EVENTS_IID: GUID = guid(0x24918ACC_64B3_37C1_8CA9_74A66E9957A8);

pub struct Signal(HANDLE);
// The event stays valid while any callback or worker holds an Arc.
unsafe impl Send for Signal {}
unsafe impl Sync for Signal {}

impl Signal {
    pub fn new() -> Option<Self> {
        let event = unsafe { CreateEventW(null(),0,0,null()) };
        (!event.is_null()).then_some(Self(event))
    }
    pub fn handle(&self) -> HANDLE { self.0 }
    pub fn wake(&self) { unsafe { SetEvent(self.0); } }
}
impl Drop for Signal {
    fn drop(&mut self) { unsafe { CloseHandle(self.0); } }
}

struct Sink {
    signal: Arc<Signal>,
    created: Lock<Vec<usize>>,
    changed: AtomicU32,
}
impl Sink {
    fn take_created(&self) -> Vec<Com> {
        self.created.with(core::mem::take).into_iter().map(|p| Com(p as Raw)).collect()
    }
}
impl Drop for Sink {
    fn drop(&mut self) { drop(self.take_created()); }
}

struct Tracked { session: Session, subscribed: bool, readable: bool }

pub struct SessionMonitor {
    manager: Com,
    sink: Arc<Sink>,
    created: Com,
    events: Com,
    watching: bool,
    tracked: BTreeMap<String, Tracked>,
    incomplete: usize,
}

impl SessionMonitor {
    pub fn open(audio: &mut Audio, output: &str, signal: Arc<Signal>) -> Result<Self> {
        let device = audio.device_by_id(output)?;
        let manager = activate(&device,&SESSION_MANAGER_IID)?;
        let sink = Arc::new(Sink { signal, created: Lock::new(Vec::new()), changed: AtomicU32::new(0) });
        let created = callback(&CREATED_TABLE,sink.clone(),CREATED_IID);
        let events = callback(&EVENTS_TABLE,sink.clone(),EVENTS_IID);
        let watching = unsafe { call!(manager.0,6,[Raw] [created.0],i32) } >= 0;
        let mut monitor = Self { manager,sink,created,events,watching,tracked:BTreeMap::new(),incomplete:0 };
        // Calling GetCount after registration enables creation notifications.
        monitor.refresh(true)?;
        Ok(monitor)
    }

    pub fn refresh(&mut self, reconcile: bool) -> Result<()> {
        let changed = self.sink.changed.swap(0,Ordering::AcqRel);
        let mut controls = self.sink.take_created();
        if reconcile {
            self.incomplete = 0;
            let mut raw = null_mut();
            checked(unsafe { call!(self.manager.0,5,[*mut Raw] [&mut raw],i32) })?;
            let enumerator = Com::from_raw(raw)?;
            let mut count = 0i32;
            checked(unsafe { call!(enumerator.0,3,[*mut i32] [&mut count],i32) })?;
            for index in 0..count {
                let mut raw = null_mut();
                if unsafe { call!(enumerator.0,4,[i32,*mut Raw] [index,&mut raw],i32) } >= 0 && !raw.is_null() {
                    controls.push(Com(raw));
                } else { self.incomplete += 1; }
            }
        }
        if reconcile || changed != 0 {
            self.tracked.retain(|_,tracked| {
                if changed & 2 != 0 && tracked.session.value.key != "system" {
                    if let Some(name) = session_text(&tracked.session.control,4)
                        .filter(|name| !name.trim().is_empty() && !name.starts_with('@')) {
                        tracked.session.value.name = name;
                    }
                }
                match tracked.session.sample() {
                    Ok(true) => { tracked.readable = true; true }
                    Ok(false) => { unsubscribe(tracked,&self.events); false }
                    Err(_) => { tracked.readable = false; true }
                }
            });
        }
        for control in controls {
            let state = match session_state(&control) {
                Ok(2) => continue,
                Ok(state) => state,
                Err(_) => { self.incomplete += 1; continue; }
            };
            let Ok(control2) = control.query(&SESSION_CONTROL_IID) else { self.incomplete += 1; continue; };
            let Some(instance) = session_text(&control2,13) else { self.incomplete += 1; continue; };
            if self.tracked.contains_key(&instance) { continue; }
            let system = unsafe { call!(control2.0,15,[] [],i32) } == 0;
            let Some(session) = read_session_identity(control,control2,state,system,instance.clone()) else {
                self.incomplete += 1;
                continue;
            };
            let subscribed = unsafe { call!(session.control.0,10,[Raw] [self.events.0],i32) } >= 0;
            self.tracked.insert(instance,Tracked { session,subscribed,readable:true });
        }
        Ok(())
    }

    pub fn rows(&self) -> impl Iterator<Item = &Session> {
        self.tracked.values().filter(|row| row.readable).map(|row| &row.session)
    }
    pub fn incomplete(&self) -> usize {
        self.incomplete + self.tracked.values().filter(|row| !row.readable).count()
    }
    pub fn set(&mut self, instance: &str, level: Option<i32>, mute: Option<bool>) -> Option<Volume> {
        let row = self.tracked.get_mut(instance)?;
        if !row.readable || !row.session.set(level,mute) { return None; }
        if row.session.sample().ok()? {
            Some(row.session.value.volume)
        } else {
            row.readable = false;
            None
        }
    }
}
impl Drop for SessionMonitor {
    fn drop(&mut self) {
        if self.watching { unsafe { call!(self.manager.0,7,[Raw] [self.created.0],i32); } }
        for tracked in self.tracked.values_mut() { unsubscribe(tracked,&self.events); }
        drop(self.sink.take_created());
    }
}
fn unsubscribe(tracked: &mut Tracked, events: &Com) {
    if tracked.subscribed {
        unsafe { call!(tracked.session.control.0,11,[Raw] [events.0],i32); }
        tracked.subscribed = false;
    }
}

#[repr(C)]
struct CallbackBase {
    table: *const c_void,
    refs: AtomicU32,
    iid: GUID,
    sink: Arc<Sink>,
}
fn callback<T>(table: &'static T, sink: Arc<Sink>, iid: GUID) -> Com {
    Com(Box::into_raw(Box::new(CallbackBase {
        table: (table as *const T).cast(), refs: AtomicU32::new(1), iid, sink,
    })).cast())
}
#[repr(C)]
struct Unknown {
    query: unsafe extern "system" fn(*mut CallbackBase,*const GUID,*mut Raw)->i32,
    add: unsafe extern "system" fn(*mut CallbackBase)->u32,
    release: unsafe extern "system" fn(*mut CallbackBase)->u32,
}
const UNKNOWN: Unknown = Unknown { query,add,release };
unsafe extern "system" fn query(this: *mut CallbackBase,iid: *const GUID,out: *mut Raw) -> i32 {
    if out.is_null() || iid.is_null() { return 0x80004003u32 as i32; }
    *out = null_mut();
    if !same(&*iid,&IUNKNOWN) && !same(&*iid,&(*this).iid) { return 0x80004002u32 as i32; }
    *out = this.cast();
    add(this);
    0
}
unsafe extern "system" fn add(this: *mut CallbackBase) -> u32 {
    (*this).refs.fetch_add(1,Ordering::Relaxed)+1
}
unsafe extern "system" fn release(this: *mut CallbackBase) -> u32 {
    let count = (*this).refs.fetch_sub(1,Ordering::AcqRel)-1;
    if count == 0 { drop(Box::from_raw(this)); }
    count
}
#[repr(C)]
struct Created {
    unknown: Unknown,
    created: unsafe extern "system" fn(*mut CallbackBase,Raw)->i32,
}
static CREATED_TABLE: Created = Created {unknown:UNKNOWN,created};
unsafe extern "system" fn created(this: *mut CallbackBase, control: Raw) -> i32 {
    if !control.is_null() {
        call!(control,1,[] [],u32);
        let sink = &(*this).sink;
        sink.created.with(|queue| queue.push(control as usize));
        sink.signal.wake();
    }
    0
}

#[repr(C)]
struct Events {
    unknown: Unknown,
    name: unsafe extern "system" fn(*mut CallbackBase,*const u16,*const GUID)->i32,
    icon: unsafe extern "system" fn(*mut CallbackBase,*const u16,*const GUID)->i32,
    volume: unsafe extern "system" fn(*mut CallbackBase,f32,i32,*const GUID)->i32,
    channels: unsafe extern "system" fn(*mut CallbackBase,u32,*const f32,u32,*const GUID)->i32,
    group: unsafe extern "system" fn(*mut CallbackBase,*const GUID,*const GUID)->i32,
    state: unsafe extern "system" fn(*mut CallbackBase,i32)->i32,
    disconnect: unsafe extern "system" fn(*mut CallbackBase,i32)->i32,
}
static EVENTS_TABLE: Events = Events {unknown:UNKNOWN,name,icon,volume,channels,group,state,disconnect:state};
unsafe fn signal(this: *mut CallbackBase, flags: u32) -> i32 {
    let sink = &(*this).sink;
    sink.changed.fetch_or(flags,Ordering::Release);
    sink.signal.wake();
    0
}
unsafe extern "system" fn name(this: *mut CallbackBase,_: *const u16,_: *const GUID) -> i32 { signal(this,2) }
unsafe extern "system" fn icon(_: *mut CallbackBase,_: *const u16,_: *const GUID) -> i32 { 0 }
unsafe extern "system" fn volume(this: *mut CallbackBase,_: f32,_: i32,context: *const GUID) -> i32 {
    if !context.is_null() && same(&*context,&OWN_VOLUME) { 0 } else { signal(this,1) }
}
unsafe extern "system" fn channels(_: *mut CallbackBase,_: u32,_: *const f32,_: u32,_: *const GUID) -> i32 { 0 }
unsafe extern "system" fn group(_: *mut CallbackBase,_: *const GUID,_: *const GUID) -> i32 { 0 }
unsafe extern "system" fn state(this: *mut CallbackBase,_: i32) -> i32 { signal(this,1) }
