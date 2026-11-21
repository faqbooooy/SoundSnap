use alloc::{boxed::Box, collections::{BTreeMap,BTreeSet}, string::String, sync::Arc, vec::Vec};
use core::{ffi::c_void, ptr::{null,null_mut}, sync::atomic::{AtomicBool,Ordering}};
use windows_sys::Win32::{
    Foundation::{CloseHandle,HANDLE,HWND,WAIT_OBJECT_0,WAIT_TIMEOUT},
    System::{Com::{CoInitializeEx,CoUninitialize,COINIT_MULTITHREADED}, Threading::{CreateThread,WaitForSingleObject,INFINITE}},
    UI::WindowsAndMessaging::PostMessageW,
};
use crate::{audio::{Audio,SessionMonitor,Signal,Volume}, motion, profiles::AppLevel, sync::Lock};

pub const MESSAGE: u32 = 0x8033;
const RECONCILE_MS: u32 = 3000;
const WRITE_INTERVAL_MS: u64 = 16;

#[derive(Clone, PartialEq, Eq)]
pub struct Row { pub instance: String, pub value: AppLevel, pub active: bool }

#[derive(Clone, PartialEq, Eq)]
pub struct Snapshot { pub epoch: u64, pub rows: Vec<Row>, pub incomplete: usize, pub unavailable: bool }

pub struct Watch { pub epoch: u64, pub output: String }

pub struct Plan {
    pub output: String,
    pub apps: Vec<AppLevel>,
    pub seen: BTreeSet<String>,
}
pub struct Change {
    pub epoch: u64,
    pub instance: String,
    pub serial: u64,
    pub level: Option<i32>,
    pub mute: Option<bool>,
    pub final_value: bool,
}
pub struct Applied { pub epoch: u64, pub instance: String, pub serial: u64, pub volume: Option<Volume> }

#[derive(Default)]
struct Incoming {
    watch: Option<Option<Watch>>,
    writes: BTreeMap<String,Change>,
    reconcile: bool,
}
#[derive(Default)]
pub struct Updates {
    pub snapshot: Option<Snapshot>,
    pub applied: Vec<Applied>,
}

struct Shared {
    wake: Arc<Signal>,
    input: Lock<Incoming>,
    plan: Lock<Option<Plan>>,
    output: Lock<Updates>,
    stop: AtomicBool,
}
struct Start { shared: Arc<Shared>, host: usize }

pub struct Worker { shared: Arc<Shared>, thread: HANDLE }
impl Worker {
    pub fn start(host: HWND) -> Option<Self> {
        let shared = Arc::new(Shared {
            wake: Arc::new(Signal::new()?), input: Lock::new(Incoming::default()),
            plan: Lock::new(None), output: Lock::new(Updates::default()), stop: AtomicBool::new(false),
        });
        let state = Box::into_raw(Box::new(Start { shared: shared.clone(),host:host as usize }));
        let thread = unsafe { CreateThread(null(),0,Some(run),state.cast(),0,null_mut()) };
        if thread.is_null() {
            unsafe { drop(Box::from_raw(state)); }
            return None;
        }
        Some(Self {shared,thread})
    }
    pub fn watch(&self, watch: Option<Watch>) {
        self.shared.input.with(|i| {
            i.watch = Some(watch);
            i.writes.clear();
        });
        self.shared.wake.wake();
    }
    pub fn plan(&self, plan: Option<Plan>) {
        self.shared.plan.with(|current| *current = plan);
        self.shared.wake.wake();
    }
    pub fn pause_plan(&self) -> Option<Plan> {
        self.shared.plan.with(Option::take)
    }
    pub fn reconcile(&self) {
        self.shared.input.with(|i| i.reconcile = true);
        self.shared.wake.wake();
    }
    pub fn write(&self, change: Change) {
        self.shared.input.with(|i| merge(&mut i.writes,change));
        self.shared.wake.wake();
    }
    pub fn updates(&self) -> Updates { self.shared.output.with(core::mem::take) }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.stop.store(true,Ordering::Release);
        self.shared.wake.wake();
        unsafe {
            WaitForSingleObject(self.thread,1000);
            CloseHandle(self.thread);
        }
    }
}

fn merge(queue: &mut BTreeMap<String,Change>, mut change: Change) {
    if let Some(previous) = queue.remove(&change.instance).filter(|previous| previous.epoch == change.epoch) {
        change.level = change.level.or(previous.level);
        change.mute = change.mute.or(previous.mute);
        change.final_value |= previous.final_value;
    }
    queue.insert(change.instance.clone(),change);
}

unsafe extern "system" fn run(state: *mut c_void) -> u32 {
    let Start {shared,host} = *Box::from_raw(state.cast());
    let host = host as HWND;
    let initialized = CoInitializeEx(null(),COINIT_MULTITHREADED as u32) >= 0;
    worker_loop(host,&shared,initialized);
    if initialized { CoUninitialize(); }
    0
}

fn worker_loop(host: HWND, shared: &Shared, initialized: bool) {
    let mut audio = Audio::default();
    let mut monitors = BTreeMap::<String,SessionMonitor>::new();
    let mut watch = None::<Watch>;
    let mut pending = BTreeMap::<String,Change>::new();
    let mut last_snapshot = None;
    let mut last_write = None::<u64>;
    let mut last_scan = motion::now();
    loop {
        let now = motion::now();
        let until_scan = if watch.is_none() && shared.plan.with(|plan| plan.is_none()) { INFINITE }
            else { RECONCILE_MS.saturating_sub(now.saturating_sub(last_scan) as u32) };
        let until_write = if pending.is_empty() { INFINITE } else {
            last_write.map_or(0,|time| WRITE_INTERVAL_MS.saturating_sub(now.saturating_sub(time)) as u32)
        };
        let wake = unsafe { WaitForSingleObject(shared.wake.handle(),until_scan.min(until_write)) };
        if shared.stop.load(Ordering::Acquire) || (wake != WAIT_OBJECT_0 && wake != WAIT_TIMEOUT) { break; }
        let now = motion::now();
        let incoming = shared.input.with(core::mem::take);
        let reconcile = incoming.reconcile || now.saturating_sub(last_scan) >= RECONCILE_MS as u64;
        if reconcile { last_scan = now; }
        if let Some(next) = incoming.watch {
            watch = next;
            pending.clear();
            last_snapshot = None;
            last_write = None;
        }
        for change in incoming.writes.into_values() {
            if watch.as_ref().is_some_and(|target| target.epoch == change.epoch) { merge(&mut pending,change); }
        }
        let mixer_output = watch.as_ref().map(|watch| watch.output.as_str());
        let profile_output = shared.plan.with(|plan| plan.as_ref().map(|plan| plan.output.clone()));
        let profile_output = profile_output.as_deref();
        monitors.retain(|id,_| mixer_output == Some(id.as_str()) || profile_output == Some(id.as_str()));
        if initialized {
            for id in [mixer_output,profile_output].into_iter().flatten() {
                if !monitors.contains_key(id) {
                    if let Ok(monitor) = SessionMonitor::open(&mut audio,id,shared.wake.clone()) {
                        monitors.insert(id.into(),monitor);
                    }
                }
            }
            monitors.retain(|_,monitor| monitor.refresh(reconcile).is_ok());
        }
        shared.plan.with(|plan| {
            if let Some(profile) = plan {
                if let Some(monitor) = monitors.get_mut(&profile.output) {
                    apply_plan(&mut audio,monitor,profile);
                }
            }
        });
        let mut applied = Vec::new();
        let ready = last_write.is_none_or(|time| now.saturating_sub(time) >= WRITE_INTERVAL_MS);
        if !pending.is_empty() && (ready || pending.values().any(|change| change.final_value)) {
            for change in core::mem::take(&mut pending).into_values() {
                let volume = mixer_output.and_then(|id| monitors.get_mut(id))
                    .and_then(|monitor| monitor.set(&change.instance,change.level,change.mute));
                applied.push(Applied {epoch:change.epoch,instance:change.instance,serial:change.serial,volume});
            }
            last_write = Some(motion::now());
        }
        let snapshot = watch.as_ref().map(|watch| {
            let source = monitors.get(&watch.output);
            Snapshot { epoch:watch.epoch, unavailable:source.is_none(),
                incomplete:source.map_or(0,SessionMonitor::incomplete),
                rows:source.map_or_else(Vec::new,|source| source.rows().map(|s| Row {
                    instance:s.instance.clone(),value:s.value.clone(),active:s.active,
                }).collect()),
            }
        });
        let changed = snapshot != last_snapshot;
        if changed { last_snapshot = snapshot.clone(); }
        if changed || !applied.is_empty() {
            let post = shared.output.with(|output| {
                let need_message = output.snapshot.is_none() && output.applied.is_empty();
                if changed { output.snapshot = snapshot; }
                output.applied.extend(applied);
                need_message && (output.snapshot.is_some() || !output.applied.is_empty())
            });
            if post && !shared.stop.load(Ordering::Acquire) {
                unsafe { PostMessageW(host,MESSAGE,0,0); }
            }
        }
    }
}

fn apply_plan(audio: &mut Audio, monitor: &mut SessionMonitor, plan: &mut Plan) {
    let mut present = BTreeSet::new();
    let mut writes = Vec::new();
    for session in monitor.rows() {
        let Some(saved) = plan.apps.iter().find(|app| app.key == session.value.key).or_else(|| {
            plan.apps.iter().find(|app| crate::app_picker::controls_session(&app.key,&session.value.key))
        }) else { continue; };
        if plan.seen.contains(&session.instance) || session.value.volume == saved.volume {
            present.insert(session.instance.clone());
        } else { writes.push((session.instance.clone(),saved.volume)); }
    }
    if !writes.is_empty() && !audio.default_id(1).ok().flatten().is_some_and(|id| id.eq_ignore_ascii_case(&plan.output)) {
        return;
    }
    for (instance,volume) in writes {
        if monitor.set(&instance,Some(volume.level),Some(volume.muted)).is_some() { present.insert(instance); }
    }
    plan.seen = present;
}
