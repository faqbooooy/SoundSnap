use alloc::{format, string::String, vec, vec::Vec};
use core::{mem::{size_of, zeroed}, ptr::{null, null_mut}};
use windows_sys::Win32::{
    Foundation::{HWND, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{FindClose, FindFirstFileW, FindNextFileW, GetBinaryTypeW, GetFileAttributesW, GetLongPathNameW,
        FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES, WIN32_FIND_DATAW},
    System::{
        Environment::GetEnvironmentVariableW,
        Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
            RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY},
        SystemInformation::GetWindowsDirectoryW,
    },
    UI::Controls::Dialogs::{CommDlgExtendedError, GetOpenFileNameW, OPENFILENAMEW, OFN_EXPLORER,
        OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR},
};
use crate::{audio::readable_process_name, locale::tr, profiles::Trigger, wide};

pub struct Popular {
    pub name: &'static str,
    file: &'static str,
    machine: &'static str,
    user: &'static [(&'static str, &'static str)],
}

pub const POPULAR: &[Popular] = &[
    Popular {name:"Steam",file:"steam.exe",machine:r"Steam\steam.exe",user:&[]},
    Popular {name:"Discord",file:"Discord.exe",machine:"",user:&[("LOCALAPPDATA",r"Discord\app-*\Discord.exe")]},
    Popular {name:"Google Chrome",file:"chrome.exe",machine:r"Google\Chrome\Application\chrome.exe",
        user:&[("LOCALAPPDATA",r"Google\Chrome\Application\chrome.exe")]},
    Popular {name:"Microsoft Edge",file:"msedge.exe",machine:r"Microsoft\Edge\Application\msedge.exe",
        user:&[("LOCALAPPDATA",r"Microsoft\Edge\Application\msedge.exe")]},
    Popular {name:"Firefox",file:"firefox.exe",machine:r"Mozilla Firefox\firefox.exe",user:&[]},
    Popular {name:"Spotify",file:"Spotify.exe",machine:r"Spotify\Spotify.exe",
        user:&[("APPDATA",r"Spotify\Spotify.exe"),("LOCALAPPDATA",r"Spotify\Spotify.exe")]},
    Popular {name:"Telegram Desktop",file:"Telegram.exe",machine:r"Telegram Desktop\Telegram.exe",
        user:&[("APPDATA",r"Telegram Desktop\Telegram.exe"),("LOCALAPPDATA",r"Programs\Telegram Desktop\Telegram.exe")]},
    Popular {name:"OBS Studio",file:"obs64.exe",machine:r"obs-studio\bin\64bit\obs64.exe",user:&[]},
    Popular {name:"VLC",file:"vlc.exe",machine:r"VideoLAN\VLC\vlc.exe",user:&[]},
    Popular {name:"Zoom",file:"Zoom.exe",machine:r"Zoom\bin\Zoom.exe",
        user:&[("APPDATA",r"Zoom\bin\Zoom.exe"),("LOCALAPPDATA",r"Zoom\bin\Zoom.exe")]},
    Popular {name:"Brave",file:"brave.exe",machine:r"BraveSoftware\Brave-Browser\Application\brave.exe",
        user:&[("LOCALAPPDATA",r"BraveSoftware\Brave-Browser\Application\brave.exe")]},
    Popular {name:"Opera",file:"opera.exe",machine:r"Opera\*\opera.exe",
        user:&[("LOCALAPPDATA",r"Programs\Opera\*\opera.exe")]},
    Popular {name:"Epic Games Launcher",file:"EpicGamesLauncher.exe",
        machine:r"Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe",user:&[]},
    Popular {name:"Visual Studio Code",file:"Code.exe",machine:r"Microsoft VS Code\Code.exe",
        user:&[("LOCALAPPDATA",r"Programs\Microsoft VS Code\Code.exe")]},
];

#[derive(Clone)]
pub enum Choice {
    Running(Trigger),
    Popular(usize),
}
impl Choice {
    pub fn name(&self) -> &str {
        match self { Self::Running(app) => &app.name, Self::Popular(i) => POPULAR[*i].name }
    }

    pub fn path(&self) -> &str {
        match self { Self::Running(app) => &app.key, Self::Popular(i) => POPULAR[*i].file }
    }
}

pub fn title(file: &str) -> String {
    POPULAR.iter().find(|app| app.file.eq_ignore_ascii_case(file))
        .map_or_else(|| readable_process_name(file), |app| app.name.into())
}

pub fn system_directory() -> String {
    let mut path = [0u16; 512];
    let len = unsafe { GetWindowsDirectoryW(path.as_mut_ptr(), path.len() as u32) } as usize;
    if len == 0 || len >= path.len() { return String::new(); }
    format!("{}\\", String::from_utf16_lossy(&path[..len]).trim_end_matches('\\').to_lowercase())
}

pub fn is_user_process(key: &str, file: &str, system_directory: &str) -> bool {
    if !system_directory.is_empty() && key.starts_with(system_directory) { return false; }
    let file = file.to_ascii_lowercase();
    if !file.ends_with(".exe") || key.contains(r"\program files\windowsapps\microsoftwindows.client.") {
        return false;
    }
    !matches!(file.as_str(), "applicationframehost.exe" | "audiodg.exe" | "conhost.exe" | "dllhost.exe"
        | "msedgewebview2.exe" | "runtimebroker.exe" | "searchhost.exe" | "securityhealthsystray.exe"
        | "steamservice.exe" | "steamwebhelper.exe" | "crashpad_handler.exe" | "crashreporter.exe"
        | "chromium_crashpad_handler.exe" | "epicwebhelper.exe" | "obs-browser-page.exe" | "soundsnap.exe")
}

pub fn popular_index(path: &str) -> Option<usize> {
    if !path.contains('\\') { return None; }
    POPULAR.iter().position(|app| file_name(path).eq_ignore_ascii_case(app.file))
}

pub fn locate(index: usize, preferred: Option<&str>, running: &[Trigger]) -> Option<Trigger> {
    let app = POPULAR.get(index)?;
    let known = |path: &str| file_name(path).eq_ignore_ascii_case(app.file);
    let key = preferred.filter(|path| known(path)).and_then(verified_path)
        .or_else(|| running.iter().find(|candidate| known(&candidate.key) && existing_exe(&candidate.key))
            .map(|candidate| candidate.key.clone()))
        .or_else(|| {
            let app_paths = format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{}", app.file);
            registry_paths(&app_paths, None).into_iter()
                .flat_map(|path| [path.clone(), format!("{path}.exe")])
                .find_map(|path| known(&path).then(|| verified_path(&path)).flatten())
        })
        .or_else(|| {
            if app.file != "steam.exe" { return None; }
            registry_paths(r"Software\Valve\Steam", Some("SteamPath")).into_iter()
                .find_map(|folder| candidate_path(&folder, "steam.exe"))
        })
        .or_else(|| app.user.iter().find_map(|(root, relative)| candidate_path(&environment(root)?, relative)))
        .or_else(|| {
            if app.machine.is_empty() { return None; }
            ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"].into_iter()
                .find_map(|root| candidate_path(&environment(root)?, app.machine))
        })?;
    Some(Trigger { key, name: app.name.into() })
}

pub enum ExeSelection { Cancelled, Invalid, Failed, Selected(Trigger) }

pub fn select_exe(owner: HWND) -> ExeSelection {
    let mut path = vec![0u16; 32768];
    let filter = wide(&format!("{} (*.exe)\0*.exe\0",tr("Приложения")));
    let title = wide(tr("Выбрать EXE"));
    let extension = wide("exe");
    let accepted = unsafe {
        let mut options: OPENFILENAMEW = zeroed();
        options.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        options.hwndOwner = owner;
        options.lpstrFilter = filter.as_ptr();
        options.nFilterIndex = 1;
        options.lpstrFile = path.as_mut_ptr();
        options.nMaxFile = path.len() as u32;
        options.lpstrTitle = title.as_ptr();
        options.lpstrDefExt = extension.as_ptr();
        options.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
        if GetOpenFileNameW(&mut options) == 0 {
            return if CommDlgExtendedError() == 0 { ExeSelection::Cancelled } else { ExeSelection::Failed };
        }
        text(&path)
    };
    app_from_path(&accepted).map_or(ExeSelection::Invalid,ExeSelection::Selected)
}

pub fn app_from_path(path: &str) -> Option<Trigger> {
    Some(Trigger { name: title(file_name(path)), key: verified_path(path)? })
}

pub fn path_key(path: &str) -> String {
    let path = path.replace('/', "\\");
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!("\\\\{unc}").to_lowercase()
    } else { path.strip_prefix(r"\\?\").unwrap_or(&path).to_lowercase() }
}

pub fn same_application(saved: &str, current: &str) -> bool {
    if saved == current { return true; }
    if file_name(saved).eq_ignore_ascii_case("discord.exe") && file_name(current).eq_ignore_ascii_case("discord.exe") {
        return discord_installation(saved).zip(discord_installation(current)).is_some_and(|(a,b)| a == b);
    }
    false
}

// Discord uses a new app-* directory for each desktop update.
fn discord_installation(path: &str) -> Option<&str> {
    let (version_path,_) = path.rsplit_once('\\')?;
    let (root,version) = version_path.rsplit_once('\\')?;
    let version = version.strip_prefix("app-")?;
    (file_name(root) == "discord" && !version.is_empty() && version.split('.')
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))).then_some(root)
}

pub fn controls_session(saved: &str, current: &str) -> bool {
    if same_application(saved,current) { return true; }
    if let Some(root) = saved.strip_suffix("\\steam.exe") {
        return current.strip_prefix(root).is_some_and(|relative| {
            relative.starts_with("\\bin\\cef\\") && relative.ends_with("\\steamwebhelper.exe")
        });
    }
    false
}

fn file_name(path: &str) -> &str { path.rsplit(['\\','/']).next().unwrap_or(path) }

fn text(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

fn existing_exe(path: &str) -> bool {
    if !file_name(path).to_ascii_lowercase().ends_with(".exe") { return false; }
    let attrs = unsafe { GetFileAttributesW(wide(path).as_ptr()) };
    attrs != INVALID_FILE_ATTRIBUTES && attrs & FILE_ATTRIBUTE_DIRECTORY == 0
}

fn verified_path(path: &str) -> Option<String> {
    let path = path.trim().trim_matches('"').replace('/', "\\");
    if !(path.starts_with(r"\\") || path.as_bytes().get(1..3) == Some(b":\\")) || !existing_exe(&path) { return None; }
    let encoded = wide(&path);
    const WINDOWS_32BIT: u32 = 0;
    const WINDOWS_64BIT: u32 = 6;
    let mut binary_type = 0;
    if unsafe { GetBinaryTypeW(encoded.as_ptr(), &mut binary_type) } == 0
        || !matches!(binary_type, WINDOWS_32BIT | WINDOWS_64BIT) { return None; }
    let length = unsafe { GetLongPathNameW(encoded.as_ptr(), null_mut(), 0) };
    let long = if length == 0 || length > 32768 { None } else {
        let mut buffer = vec![0u16; length as usize];
        let result = unsafe { GetLongPathNameW(encoded.as_ptr(), buffer.as_mut_ptr(), length) };
        (result != 0 && result < length).then(|| text(&buffer))
    };
    Some(path_key(&long.unwrap_or(path)))
}

fn environment(name: &str) -> Option<String> {
    let name = wide(name);
    let capacity = unsafe { GetEnvironmentVariableW(name.as_ptr(), null_mut(), 0) };
    if capacity == 0 || capacity > 32768 { return None; }
    let mut buffer = vec![0u16; capacity as usize];
    let len = unsafe { GetEnvironmentVariableW(name.as_ptr(), buffer.as_mut_ptr(), capacity) };
    (len > 0 && len < capacity).then(|| text(&buffer))
}

fn registry_paths(subkey: &str, value: Option<&str>) -> Vec<String> {
    let subkey = wide(subkey);
    let value = value.map(wide);
    [(HKEY_CURRENT_USER,0), (HKEY_LOCAL_MACHINE,RRF_SUBKEY_WOW6464KEY), (HKEY_LOCAL_MACHINE,RRF_SUBKEY_WOW6432KEY)]
        .into_iter().filter_map(|(root,view)| {
            let ptr = value.as_ref().map_or(null(), |name| name.as_ptr());
            registry_value(root, &subkey, ptr, view)
        }).collect()
}

fn registry_value(root: HKEY, subkey: &[u16], value: *const u16, view: u32) -> Option<String> {
    let mut bytes = 0u32;
    let flags = view | RRF_RT_REG_SZ;
    unsafe {
        if RegGetValueW(root,subkey.as_ptr(),value,flags,null_mut(),null_mut(),&mut bytes) != 0
            || bytes < 2 || bytes > 65536 { return None; }
        let mut buffer = vec![0u16; bytes.div_ceil(2) as usize];
        if RegGetValueW(root,subkey.as_ptr(),value,flags,null_mut(),buffer.as_mut_ptr().cast(),&mut bytes) != 0 { return None; }
        Some(text(&buffer).trim().trim_matches('"').into())
    }
}

pub(crate) fn candidate_path(root: &str, relative: &str) -> Option<String> {
    let base = format!("{}\\{relative}",root.trim_end_matches(['\\','/']));
    let Some((prefix, suffix)) = base.split_once('*') else { return verified_path(&base); };
    let (parent, stem) = prefix.rsplit_once('\\')?;
    let mut entries = Vec::new();
    unsafe {
        let mut data: WIN32_FIND_DATAW = zeroed();
        let search = FindFirstFileW(wide(&format!("{prefix}*")).as_ptr(), &mut data);
        if search == INVALID_HANDLE_VALUE { return None; }
        loop {
            if data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                let name = text(&data.cFileName);
                if name != "." && name != ".." {
                    let version = name.strip_prefix(stem).unwrap_or(&name).split('.')
                        .map(|part| part.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
                    entries.push((version,name));
                }
            }
            if FindNextFileW(search, &mut data) == 0 { break; }
        }
        FindClose(search);
    }
    entries.sort_unstable_by(|a,b| b.cmp(a));
    entries.into_iter().find_map(|(_,name)| verified_path(&format!("{parent}\\{name}{suffix}")))
}
