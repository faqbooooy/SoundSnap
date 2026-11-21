use alloc::{collections::{BTreeMap,BTreeSet}, string::String, vec::Vec};
use core::mem::{size_of,zeroed};
use windows_sys::Win32::{
    Foundation::{CloseHandle,INVALID_HANDLE_VALUE},
    System::{Diagnostics::ToolHelp::*, RemoteDesktop::ProcessIdToSessionId, Threading::GetCurrentProcessId},
};
use crate::{app_picker, audio::{process_path,Volume},profiles::{Output,Profile,Trigger}};

pub const TIMER: usize = 9;
pub const INTERVAL: u32 = 1000;

pub type RunningApps = BTreeMap<String,BTreeSet<u32>>;

pub fn discard_suppressed(running: &mut RunningApps, suppressed: &mut RunningApps) {
    suppressed.retain(|key, instances| {
        if let Some(current) = running.get(key) {
            instances.retain(|pid| current.contains(pid));
            !instances.is_empty()
        } else { false }
    });
    running.retain(|key,_| !suppressed.contains_key(key));
}

pub struct SavedSession { pub instance: String, pub key: String, pub volume: Volume }
pub struct Baseline {
    pub output: Output,
    pub volume: Volume,
    pub sessions: Vec<SavedSession>,
    pub restored: BTreeSet<String>,
}

#[derive(Default)]
pub struct State {
    pub baseline: Option<Baseline>,
    pub active: Option<usize>,
    pub suppressed: RunningApps,
    pub watching: bool,
    pub last_failed: Option<usize>,
    pub retry_after: u8,
    pub restore_pending: Option<(Baseline,u8)>,
}

pub fn best(profiles: &[Profile], running: &RunningApps, active: Option<usize>, connected: impl Fn(&str)->bool) -> Option<usize> {
    let mut best = None::<(usize,u8)>;
    for (i,p) in profiles.iter().enumerate() {
        let Some(trigger) = &p.automation.trigger else {continue;};
        if running.contains_key(&trigger.key) && (!p.automation.require_output || active == Some(i) || connected(&p.output.id))
            && best.is_none_or(|(_,priority)| p.automation.priority > priority) {
            best = Some((i,p.automation.priority));
        }
    }
    best.map(|(i,_)| i)
}

pub fn route(profile: &Profile, allow_primary: bool, connected: impl Fn(&str) -> bool) -> (Option<&Output>, Volume, bool) {
    if allow_primary && connected(&profile.output.id) {
        let level = profile.volume.level.min(profile.primary_cap.into());
        return (Some(&profile.output),Volume {level,..profile.volume},false);
    }
    let backup = profile.fallback.as_ref().filter(|device| connected(&device.id));
    let volume = Volume {level:profile.fallback_level.min(profile.fallback_cap).into(),muted:profile.volume.muted};
    (backup,volume,true)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EndpointAction { Stay, Apply {allow_primary: bool}, ManualOverride }

pub fn endpoint_action(profile: &Profile, degraded: bool, routed: bool, unchanged: bool, route_connected: bool,
    connected: impl Fn(&str) -> bool) -> EndpointAction {
    let primary = connected(&profile.output.id);
    let recovered = degraded && primary && profile.reconnect;
    let backup_arrived = degraded && !routed && unchanged
        && profile.fallback.as_ref().is_some_and(|output| connected(&output.id));
    if (!primary && !degraded) || recovered || backup_arrived || (degraded && (routed || !unchanged) && !route_connected) {
        EndpointAction::Apply {allow_primary:!degraded || recovered}
    } else if !unchanged { EndpointAction::ManualOverride } else { EndpointAction::Stay }
}

pub fn running(profiles: &[Profile]) -> Option<RunningApps> {
    let keys: BTreeSet<_> = profiles.iter().filter_map(|p| p.automation.trigger.as_ref()).map(|t| t.key.as_str()).collect();
    if keys.is_empty() { return Some(RunningApps::new()); }
    let files: BTreeSet<_> = keys.iter().map(|key| key.rsplit('\\').next().unwrap_or(key)).collect();
    let mut found = RunningApps::new();
    let complete = processes(
        |file| files.contains(file.to_lowercase().as_str()),
        |pid, _| {
            if let Some(path) = process_path(pid).map(|path| app_picker::path_key(&path)) {
                for key in keys.iter().filter(|key| app_picker::same_application(key,&path)) {
                    found.entry((*key).into()).or_default().insert(pid);
                }
            }
        },
    );
    complete.then_some(found)
}

pub fn applications() -> Vec<Trigger> {
    let mut found = BTreeMap::new();
    let system = app_picker::system_directory();
    processes(|file| app_picker::is_user_process("", file, ""), |pid,file| {
        if let Some(path) = process_path(pid) {
            let key = app_picker::path_key(&path);
            if app_picker::is_user_process(&key, file, &system) {
                found.entry(key.clone()).or_insert_with(|| Trigger {key,name:app_picker::title(file)});
            }
        }
    });
    let mut list: Vec<_> = found.into_values().collect();
    list.sort_by_cached_key(|entry| entry.name.to_lowercase());
    list
}

fn processes(matches_file: impl Fn(&str) -> bool, mut visit: impl FnMut(u32,&str)) -> bool {
    unsafe {
        let own = GetCurrentProcessId();
        let mut session = 0;
        if ProcessIdToSessionId(own,&mut session) == 0 {return false;}
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0);
        if snapshot == INVALID_HANDLE_VALUE {return false;}
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize=size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot,&mut entry) != 0;
        let complete = ok;
        while ok {
            let pid=entry.th32ProcessID;
            if pid != 0 && pid != own {
                let len = entry.szExeFile.iter().position(|&unit| unit == 0).unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                let mut target_session = 0;
                if matches_file(&name) && ProcessIdToSessionId(pid,&mut target_session) != 0 && target_session == session {
                    visit(pid,&name);
                }
            }
            ok=Process32NextW(snapshot,&mut entry)!=0;
        }
        CloseHandle(snapshot);
        complete
    }
}
