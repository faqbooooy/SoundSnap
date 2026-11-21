use alloc::{collections::BTreeSet, format, vec, vec::Vec};
use core::{mem::{size_of,zeroed}, ptr::{null,null_mut}};
use windows_sys::Win32::{
    Foundation::{CloseHandle,HANDLE,WAIT_TIMEOUT},
    Media::Audio::*,
    System::Threading::{CreateProcessW, Sleep, TerminateProcess,
        WaitForSingleObject, PROCESS_INFORMATION, STARTUPINFOW},
};
use crate::{app_picker, audio::{Audio,Volume}, automation::{self,Baseline,EndpointAction,SavedSession}, mixer,
    profile_app::{restore_apps,restore_baseline_sessions}, profiles::{self,AppLevel,Automation,Output,Profile,Trigger}, runtime,wide};

pub fn fixture() -> i32 {
    unsafe {
        let mut output = null_mut();
        let format = WAVEFORMATEX {wFormatTag:1,nChannels:1,nSamplesPerSec:48000,
            nAvgBytesPerSec:96000,nBlockAlign:2,wBitsPerSample:16,cbSize:0};
        if waveOutOpen(&mut output,WAVE_MAPPER,&format,0,0,0) != 0 { return 201; }
        let mut silence = vec![0u8; 96000*45];
        let mut header = WAVEHDR {lpData:silence.as_mut_ptr(),dwBufferLength:silence.len() as u32,..zeroed()};
        let size = size_of::<WAVEHDR>() as u32;
        if waveOutPrepareHeader(output,&mut header,size) != 0 { waveOutClose(output); return 202; }
        let playing = waveOutWrite(output,&mut header,size) == 0;
        if playing { Sleep(45000); }
        waveOutReset(output);
        waveOutUnprepareHeader(output,&mut header,size);
        waveOutClose(output);
        if playing {0} else {203}
    }
}

struct Child(HANDLE);
impl Child {
    fn start(path: &str, argument: &str) -> Option<Self> {
        let application = wide(path);
        let mut command = wide(&format!("\"{path}\" {argument}"));
        unsafe {
            let startup = STARTUPINFOW {cb:size_of::<STARTUPINFOW>() as u32,..zeroed()};
            let mut process: PROCESS_INFORMATION = zeroed();
            if CreateProcessW(application.as_ptr(),command.as_mut_ptr(),null(),null(),0,0,null(),null(),&startup,&mut process) == 0 {
                return None;
            }
            CloseHandle(process.hThread);
            Some(Self(process.hProcess))
        }
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        unsafe {
            if WaitForSingleObject(self.0,0) == WAIT_TIMEOUT {
                TerminateProcess(self.0,0);
                WaitForSingleObject(self.0,1500);
            }
            CloseHandle(self.0);
        }
    }
}

fn wait_for<T>(mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    for _ in 0..80 {
        if let Some(value) = probe() { return Some(value); }
        unsafe { Sleep(75); }
    }
    None
}

fn profile(app: &Trigger, output: &Output, priority: u8) -> Profile {
    Profile {
        name:app.name.clone(),output:output.clone(),volume:Volume{level:81,muted:false},
        apps:vec![AppLevel{key:app.key.clone(),name:app.name.clone(),volume:Volume{level:27,muted:true}}],
        fallback:Some(Output{id:"backup:fixture".into(),name:"Backup".into()}),fallback_level:82,
        reconnect:true,shortcut:None,primary_cap:55,fallback_cap:28,
        automation:Automation{trigger:Some(app.clone()),priority,..Automation::default()},
    }
}

fn levels(audio: &mut Audio, output: &str, key: &str) -> Vec<Volume> {
    audio.sessions(Some(output)).unwrap_or_default().into_iter()
        .filter(|s| s.value.key == key).map(|s| s.value.volume).collect()
}

pub fn run() -> i32 {
    let Some(executable) = runtime::executable_path() else { return 10 };
    let Some((root,_)) = executable.rsplit_once('\\') else { return 10 };
    let base = root.rsplit('\\').next().unwrap_or_default();
    if !base.starts_with("profile-audit-") { return 10; }
    let a_path = format!("{root}\\Тестовое аудио A.exe");
    let old_discord = format!("{root}\\Discord\\app-1.0.1\\Discord.exe");
    let new_discord = format!("{root}\\Discord\\app-1.0.2\\Discord.exe");
    let bad = format!("{root}\\missing.exe");
    let fake = format!("{root}\\Not a binary.exe");
    let Some(a) = app_picker::app_from_path(&a_path) else { return 11; };
    let Some(old) = app_picker::app_from_path(&old_discord) else { return 11; };
    let Some(updated) = app_picker::app_from_path(&new_discord) else { return 11; };
    if app_picker::candidate_path(root,r"Discord\app-*\Discord.exe").as_deref() != Some(updated.key.as_str()) {
        return 30;
    }
    if app_picker::app_from_path(&fake).is_some() { return 31; }
    if app_picker::app_from_path(&bad).is_some()
        || app_picker::path_key(r"\\?\C:\Test\APP.EXE") != r"c:\test\app.exe"
        || app_picker::path_key(r"\\?\UNC\server\Share\APP.EXE") != r"\\server\share\app.exe"
        || !app_picker::same_application(&old.key,&updated.key)
        || app_picker::same_application(&old.key,r"c:\other\discord\app-1.0.2\discord.exe")
        || !app_picker::controls_session(r"c:\steam\steam.exe",r"c:\steam\bin\cef\cef.win64\steamwebhelper.exe")
        || app_picker::controls_session(r"c:\steam\steam.exe",r"c:\steam-two\bin\cef\steamwebhelper.exe")
        || app_picker::is_user_process(r"c:\windows\system32\explorer.exe","explorer.exe","c:\\windows\\") {
        return 12;
    }

    let mut audio = Audio::default();
    let Some(device) = audio.devices().ok().and_then(|devices| devices.into_iter().find(|d| d.selected))
        else {return 13;};
    let output = Output {id:device.id,name:device.name};
    let low = profile(&a,&output,2);
    let Some(discord_index) = app_picker::popular_index(&old.key) else { return 24; };
    let Some(popular_closed) = app_picker::locate(discord_index,Some(&old.key),&[]) else { return 24; };
    if popular_closed.key != old.key || !automation::running(&[profile(&popular_closed,&output,9)])
        .is_some_and(|running| !running.contains_key(&old.key)) { return 24; }
    let mut high = profile(&popular_closed,&output,9);
    high.apps[0].volume = Volume {level:34,muted:false};
    let backup = high.fallback.as_ref().unwrap().id.as_str();
    let route = automation::route(&high,true,|id| id == output.id);
    let fallback = automation::route(&high,true,|id| id == backup);
    if route.0.map(|o| o.id.as_str()) != Some(output.id.as_str()) || route.1.level != 55 || route.2
        || fallback.0.map(|o| o.id.as_str()) != Some(backup) || fallback.1.level != 28 || !fallback.2
        || automation::route(&high,true,|_| false).0.is_some()
        || automation::endpoint_action(&high,false,true,false,false,|id| id == backup) != (EndpointAction::Apply{allow_primary:true})
        || automation::endpoint_action(&high,true,true,true,true,|_| true) != (EndpointAction::Apply{allow_primary:true})
        || automation::endpoint_action(&high,true,false,true,true,|id| id == backup) != (EndpointAction::Apply{allow_primary:false})
        || automation::endpoint_action(&high,true,true,true,false,|_| false) != (EndpointAction::Apply{allow_primary:false})
        || automation::endpoint_action(&high,false,true,false,true,|_| true) != EndpointAction::ManualOverride {
        return 14;
    }
    high.reconnect = false;
    if automation::endpoint_action(&high,true,true,true,true,|_| true) != EndpointAction::Stay { return 14; }
    let profiles = [low,high];
    let Some(loaded) = profiles::decode(&profiles::encode(&profiles)) else { return 15; };
    let mut legacy_profile = profiles[0].clone();
    legacy_profile.automation.trigger = None;
    let mut legacy = profiles::encode(&[legacy_profile]);
    legacy[..4].copy_from_slice(b"SSP1");
    legacy.truncate(legacy.len() - 6);
    if !profiles::decode(&legacy).is_some_and(|profiles| {
        profiles.len() == 1 && profiles[0].primary_cap == 100 && profiles[0].automation.trigger.is_none()
    }) { return 15; }
    if loaded.len() != 2 || loaded[1].apps != profiles[1].apps || loaded[1].automation.priority != 9
        || loaded[1].automation.trigger.as_ref().map(|app| app.key.as_str()) != Some(old.key.as_str())
        || loaded[1].primary_cap != 55 || loaded[1].fallback_cap != 28 || loaded[1].reconnect {
        return 15;
    }

    let Some(idle) = Child::start(&a_path,"--profile-idle-fixture") else { return 16; };
    let from_running = wait_for(|| automation::applications().into_iter().find(|app| app.key == a.key));
    if from_running.as_ref().map(|app| app.key.as_str()) != Some(a.key.as_str())
        || !levels(&mut audio,&output.id,&a.key).is_empty() {
        return 16;
    }
    let mut suppressed = automation::running(&loaded).unwrap_or_default();
    let mut still_running = suppressed.clone();
    automation::discard_suppressed(&mut still_running,&mut suppressed);
    if still_running.contains_key(&a.key) { return 29; }
    drop(idle);
    let Some(first) = Child::start(&a_path,"--profile-audio-fixture") else { return 17; };
    if wait_for(|| levels(&mut audio,&output.id,&a.key).first().copied()).is_none() { return 17; }
    let mut running = automation::running(&loaded).unwrap_or_default();
    automation::discard_suppressed(&mut running,&mut suppressed);
    if automation::best(&loaded,&running,None,|_| true) != Some(0)
        || automation::best(&loaded,&running,None,|_| false).is_some()
        || automation::best(&loaded,&running,Some(0),|_| false) != Some(0) {
        return 18;
    }
    let Ok(sessions) = audio.sessions(Some(&output.id)) else { return 19; };
    let mut baseline = Baseline {output:output.clone(),volume:Volume{level:0,muted:false},restored:BTreeSet::new(),
        sessions:sessions.into_iter().filter(|s| s.value.key == a.key).map(|s| SavedSession {
            instance:s.instance,key:s.value.key,volume:s.value.volume,
        }).collect()};
    if baseline.sessions.len() != 1 { return 19; }
    let mut seen = BTreeSet::new();
    restore_apps(&mut audio,&output.id,&loaded[0].apps,&mut seen,None);
    if levels(&mut audio,&output.id,&a.key) != [loaded[0].apps[0].volume] || seen.len() != 1 { return 20; }
    restore_baseline_sessions(&mut audio,&mut baseline);
    if levels(&mut audio,&output.id,&a.key) != [baseline.sessions[0].volume] { return 21; }

    let Some(worker) = mixer::Worker::start(null_mut()) else { return 22; };
    worker.plan(Some(mixer::Plan {output:output.id.clone(),apps:loaded[1].apps.clone(),seen:BTreeSet::new()}));
    let Some(next) = Child::start(&new_discord,"--profile-audio-fixture") else { return 23; };
    let wanted = loaded[1].apps[0].volume;
    if wait_for(|| (levels(&mut audio,&output.id,&updated.key) == [wanted]).then_some(())).is_none() { return 23; }
    let popular = app_picker::popular_index(&updated.key).and_then(|i| app_picker::locate(i,None,&[updated.clone()]));
    if popular.as_ref().map(|app| app.key.as_str()) != Some(updated.key.as_str()) { return 24; }
    let running = automation::running(&loaded).unwrap_or_default();
    if automation::best(&loaded,&running,None,|_| true) != Some(1)
        || automation::best(&loaded,&running,None,|_| false).is_some() {
        return 25;
    }
    if worker.pause_plan().is_none() { return 26; }
    let Ok(sessions) = audio.sessions(Some(&output.id)) else { return 26; };
    let Some(session) = sessions.into_iter().find(|s| s.value.key == updated.key) else {return 26;};
    if !session.set(Some(73),Some(false)) {return 26;}
    worker.reconcile();
    unsafe { Sleep(250); }
    if levels(&mut audio,&output.id,&updated.key) != [Volume {level:73,muted:false}] { return 26; }
    drop(next);
    if wait_for(|| {
        let running = automation::running(&loaded)?;
        (automation::best(&loaded,&running,Some(1),|_| true) == Some(0)).then_some(())
    }).is_none() { return 27; }
    drop(first);
    if wait_for(|| {
        let running = automation::running(&loaded)?;
        automation::best(&loaded,&running,Some(0),|_| true).is_none().then_some(())
    }).is_none() { return 28; }
    0
}
