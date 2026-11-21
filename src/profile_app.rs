use alloc::{collections::{BTreeMap, BTreeSet}, format, string::String, vec::Vec};
use crate::{audio::{Device, Session, Volume}, locale::tr, profile_ui::{Page, View},
    profiles::{self, AppLevel, Output, Profile}, settings::Shortcut, ui::Action, App, WindowKind};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

pub const TIMER: usize = 6;
const HOTKEY_BASE: i32 = 1000;
const HOTKEY_PROBE: i32 = 2000;

pub struct AppChoice {
    pub app: AppLevel,
    pub sessions: usize,
}

struct Running {
    index: usize,
    output: Option<String>,
    degraded: bool,
    apps_allowed: bool,
    seen: BTreeSet<String>,
}

pub struct ProfileState {
    list: Vec<Profile>,
    draft: Option<Profile>,
    draft_index: Option<usize>,
    running: Option<Running>,
    pub page: Page,
    pub entry: Page,
    pub typing: bool,
    pub capturing: bool,
    start: usize,
    outputs: Vec<Device>,
    apps: Vec<AppLevel>,
    playing: Vec<bool>,
    choices: Vec<AppChoice>,
    live_sessions: Vec<Session>,
    confirm_delete: bool,
    error: &'static str,
    timer_interval: u32,
    pending_surrogate: Option<u16>,
    registered_hotkeys: Vec<i32>,
}

impl ProfileState {
    pub fn new() -> Self {
        Self { list: profiles::load(), draft: None, draft_index: None, running: None,
            page: Page::List, entry: Page::List, typing: false, capturing: false, start: 0, outputs: Vec::new(), apps: Vec::new(),
            playing: Vec::new(), choices: Vec::new(), live_sessions: Vec::new(), confirm_delete: false, error: "", timer_interval: 0,
            pending_surrogate: None, registered_hotkeys: Vec::new() }
    }
    pub fn name(&self) -> Option<&str> {
        self.running.as_ref().and_then(|running| self.list.get(running.index)).map(|p| p.name.as_str())
    }
    pub fn view(&self, available_height: f32) -> View<'_> {
        View {
            page: self.page, list: &self.list, selected: self.running.as_ref().map(|running| running.index),
            draft: self.draft.as_ref(), editing_existing: self.draft_index.is_some(), start: self.start,
            outputs: &self.outputs, apps: &self.apps, playing: &self.playing, choices: &self.choices, available_height,
            typing: self.typing, capturing: self.capturing, error: self.error, confirm_delete: self.confirm_delete,
        }
    }
    pub fn close(&mut self) {
        self.draft = None;
        self.draft_index = None;
        self.typing = false;
        self.capturing = false;
        self.pending_surrogate = None;
        self.confirm_delete = false;
        self.apps.clear();
        self.playing.clear();
        self.choices.clear();
        self.live_sessions.clear();
        self.outputs.clear();
    }
}

impl App {
    pub(crate) fn profile_height_budget(&self) -> f32 {
        (self.area.bottom - self.area.top) as f32 / self.scale
    }

    pub(crate) fn profile_model(&self) -> View<'_> {
        self.profiles.view(self.profile_height_budget())
    }

    pub(crate) fn reset_profile_view(&mut self) {
        self.profiles.close();
        self.profiles.page = self.profiles.entry;
        self.profiles.start = 0;
        self.profiles.error = "";
        if self.profiles.page == Page::Mixer { self.refresh_mixer(); }
        self.sync_profile_timer();
    }

    fn profile_page(&mut self, page: Page) {
        self.profiles.page = page;
        self.profiles.start = 0;
        self.profiles.typing = false;
        self.profiles.capturing = false;
        self.profiles.pending_surrogate = None;
        self.profiles.confirm_delete = false;
        self.profiles.error = "";
        self.focus = None;
        self.hover = None;
        if page != Page::Mixer { self.profiles.live_sessions.clear(); }
        match page {
            Page::Mixer => self.refresh_mixer(),
            Page::Output | Page::Fallback => self.refresh_profile_outputs(),
            Page::PickApps => self.refresh_app_choices(),
            _ => (),
        }
        self.sync_profile_timer();
        self.geometry();
        self.begin_page_motion();
        self.draw_window();
    }

    pub(crate) fn profile_back(&mut self) {
        match self.profiles.page {
            Page::List => self.show(WindowKind::Popup,self.anchor),
            Page::Mixer => self.show(WindowKind::Popup,self.anchor),
            Page::Edit => {
                self.profiles.draft = None;
                self.profiles.draft_index = None;
                self.profile_page(Page::List);
            }
            Page::Apps | Page::Output | Page::Fallback => self.profile_page(Page::Edit),
            Page::PickApps => self.profile_page(Page::Apps),
        }
    }

    fn refresh_profile_outputs(&mut self) {
        let mut outputs = self.audio.devices().unwrap_or_default();
        if self.profiles.page == Page::Fallback {
            if let Some(profile) = &self.profiles.draft {
                outputs.retain(|output| !same(&output.id,&profile.output.id));
            }
        }
        self.profiles.outputs = outputs;
        self.clamp_profile_scroll();
    }

    pub(crate) fn profile_language_changed(&mut self) {
        match self.profiles.page {
            Page::Mixer if self.pressed.is_none() => self.refresh_mixer(),
            Page::PickApps => self.refresh_app_choices(),
            Page::Output | Page::Fallback => self.refresh_profile_outputs(),
            _ => (),
        }
    }

    fn refresh_mixer(&mut self) {
        match self.audio.sessions(None) {
            Ok(mut sessions) => {
                let mut labels = BTreeMap::new();
                for session in &sessions { *labels.entry(session.value.name.to_lowercase()).or_insert(0usize) += 1; }
                sessions.sort_by_cached_key(|session| (session.value.name.to_lowercase(),session.instance.clone()));
                let mut current = BTreeMap::new();
                let (apps, playing) = sessions.iter().map(|session| {
                    let mut value = session.value.clone();
                    let label = value.name.to_lowercase();
                    if labels[&label] > 1 {
                        let index = current.entry(label).or_insert(0);
                        *index += 1;
                        value.name = format!("{} · {}",value.name,index);
                    }
                    (value,session.active)
                }).unzip();
                self.profiles.apps = apps;
                self.profiles.playing = playing;
                self.profiles.live_sessions = sessions;
                self.profiles.error = "";
                self.clamp_profile_scroll();
            }
            Err(_) => {
                self.profiles.apps.clear();
                self.profiles.playing.clear();
                self.profiles.live_sessions.clear();
                self.profiles.start = 0;
                self.profiles.error = tr("Не удалось прочитать аудиосессии.");
            }
        }
    }

    fn clamp_profile_scroll(&mut self) {
        let view = self.profile_model();
        self.profiles.start = self.profiles.start.min(view.count().saturating_sub(view.rows()));
    }

    pub(crate) fn profile_scroll(&mut self, delta: i32) {
        let view = self.profile_model();
        let max = view.count().saturating_sub(view.rows());
        let next = if delta < 0 { (self.profiles.start+1).min(max) }
            else { self.profiles.start.saturating_sub(1) };
        if next != self.profiles.start {
            self.profiles.start = next;
            self.focus = None;
            self.hover = None;
            self.draw_window();
        }
    }

    pub(crate) fn profile_action(&mut self, action: Action) {
        match action {
            Action::Profiles | Action::Mixer if self.kind != WindowKind::Profiles => {
                self.profiles.entry = if action == Action::Mixer {Page::Mixer} else {Page::List};
                self.show(WindowKind::Profiles, self.anchor);
            }
            Action::Profiles => self.profile_page(Page::List),
            Action::Mixer => self.profile_page(Page::Mixer),
            Action::ProfileBack => self.profile_back(),
            Action::ProfileCapture => self.capture_profile(),
            Action::ProfileApply(i) => self.activate_profile(i),
            Action::ProfileDeactivate => {
                self.deactivate_profile();
                self.focus = None;
                self.hover = None;
                self.draw_window();
            }
            Action::ProfileEdit(i) => {
                self.profiles.draft = self.profiles.list.get(i).cloned();
                self.profiles.draft_index = Some(i);
                self.profile_page(Page::Edit);
            }
            Action::ProfileName => {
                self.profiles.typing = true;
                self.profiles.capturing = false;
                self.profiles.error = tr("Введите имя. Enter завершает ввод.");
                self.draw_window();
            }
            Action::ProfileSave => self.save_profile(),
            Action::ProfileDelete => self.delete_profile(),
            Action::ProfileOutput => self.profile_page(Page::Output),
            Action::ProfileFallback => self.profile_page(Page::Fallback),
            Action::ProfileApps => self.profile_page(Page::Apps),
            Action::ProfileReconnect => {
                if let Some(draft) = &mut self.profiles.draft { draft.reconnect = !draft.reconnect; }
                self.draw_window();
            }
            Action::ProfileShortcut => {
                self.profiles.typing = false;
                self.profiles.capturing = true;
                self.profile_error(tr("Нажмите сочетание. Backspace — сбросить."));
            }
            Action::ProfileMasterMute => {
                if let Some(draft) = &mut self.profiles.draft { draft.volume.muted = !draft.volume.muted; }
                self.draw_window();
            }
            Action::ProfileOutputPick(i) => self.pick_profile_output(i),
            Action::ProfileMuteFallback => {
                if let Some(draft) = &mut self.profiles.draft { draft.fallback = None; }
                self.profile_page(Page::Edit);
            }
            Action::ProfileImportApps => self.profile_page(Page::PickApps),
            Action::ProfileAppPick(i) => self.toggle_profile_app(i),
            Action::ProfileRefreshApps => {
                self.refresh_app_choices();
                self.hover = None;
                self.focus = None;
                self.geometry();
                self.draw_window();
            }
            Action::AppMute(i) => self.change_app(i, None, true),
            Action::AppForget(i) => {
                if let Some(draft) = &mut self.profiles.draft {
                    if i < draft.apps.len() { draft.apps.remove(i); }
                }
                self.focus = None;
                self.hover = None;
                self.clamp_profile_scroll();
                self.geometry();
                self.draw_window();
            }
            _ => (),
        }
    }

    pub(crate) fn profile_slider(&mut self, action: Action, level: i32) {
        let level = level.clamp(0, 100);
        if self.profile_slider_level(action) == Some(level) { return; }
        match action {
            Action::ProfileMaster => {
                if let Some(draft) = &mut self.profiles.draft { draft.volume.level = level; }
                self.draw_window();
            }
            Action::ProfileFallbackLevel => {
                if let Some(draft) = &mut self.profiles.draft { draft.fallback_level = level as u8; }
                self.draw_window();
            }
            Action::AppVolume(i) => self.change_app(i, Some(level), false),
            _ => (),
        }
    }

    pub(crate) fn profile_slider_level(&self, action: Action) -> Option<i32> {
        let draft = self.profiles.draft.as_ref();
        match action {
            Action::ProfileMaster => draft.map(|p| p.volume.level),
            Action::ProfileFallbackLevel => draft.map(|p| p.fallback_level.into()),
            Action::AppVolume(i) => {
                let apps = if self.profiles.page == Page::Apps {&draft?.apps} else {&self.profiles.apps};
                apps.get(i).map(|app| app.volume.level)
            }
            _ => None,
        }
    }

    fn change_app(&mut self, index: usize, level: Option<i32>, toggle: bool) {
        if self.profiles.page == Page::Apps {
            if let Some(app) = self.profiles.draft.as_mut().and_then(|profile| profile.apps.get_mut(index)) {
                if let Some(level) = level { app.volume.level = level; }
                if toggle { app.volume.muted = !app.volume.muted; }
            }
        } else if let Some(app) = self.profiles.apps.get_mut(index) {
            let muted = toggle.then_some(!app.volume.muted);
            if level == Some(app.volume.level) && !toggle { return; }
            let changed = self.profiles.live_sessions.get(index).is_some_and(|session| session.set(level,muted));
            if changed {
                if let Some(level) = level { app.volume.level = level; }
                if let Some(muted) = muted { app.volume.muted = muted; }
                self.profiles.error = "";
            } else { self.profiles.error = tr("Аудиосессия недоступна."); }
        }
        self.draw_window();
    }

    fn capture_profile(&mut self) {
        if self.profiles.list.len() >= profiles::LIMIT { return; }
        let Ok(outputs) = self.audio.devices() else {
            self.profile_error(tr("Не удалось получить текущий выход."));
            return;
        };
        let (Some(output), Ok(volume)) = (outputs.iter().find(|output| output.selected), self.audio.volume()) else {
            self.profile_error(tr("Сначала выберите доступное устройство вывода."));
            return;
        };
        let label = (1..).map(|n| format!("{} {n}",tr("Профиль"))).find(|name| {
            !self.profiles.list.iter().any(|profile| &profile.name == name)
        }).unwrap();
        let apps = self.audio.sessions(None).map(|sessions| saved_apps(&sessions)).unwrap_or_default();
        self.profiles.draft_index = None;
        self.profiles.draft = Some(Profile {
            name: label, output: output_data(output), volume, apps,
            fallback: None, fallback_level: 20, reconnect: true, shortcut: None,
        });
        self.profile_page(Page::Edit);
    }

    fn pick_profile_output(&mut self, index: usize) {
        let (Some(device), Some(profile)) = (self.profiles.outputs.get(index), &mut self.profiles.draft) else { return };
        let output = output_data(device);
        if self.profiles.page == Page::Fallback {
            profile.fallback = Some(output);
        } else {
            if profile.fallback.as_ref().is_some_and(|fallback| same(&fallback.id,&output.id)) {
                profile.fallback = None;
            }
            profile.output = output;
        }
        self.profile_page(Page::Edit);
    }

    fn refresh_app_choices(&mut self) {
        let sessions = match self.audio.sessions(None) {
            Ok(sessions) => {
                self.profiles.error = "";
                sessions
            }
            Err(_) => {
                self.profiles.error = tr("Микшер недоступен. Попробуй обновить список.");
                Vec::new()
            }
        };
        let mut choices = grouped(&sessions);
        if let Some(profile) = &self.profiles.draft {
            for app in &profile.apps {
                if let Some(choice) = choices.iter_mut().find(|choice| choice.app.key == app.key) {
                    choice.app = app.clone();
                } else {
                    choices.push(AppChoice {app: app.clone(), sessions: 0});
                }
            }
        }
        choices.sort_by_cached_key(|choice| choice.app.name.to_lowercase());
        self.profiles.choices = choices;
        self.clamp_profile_scroll();
    }

    fn toggle_profile_app(&mut self, index: usize) {
        let (Some(choice),Some(profile)) = (self.profiles.choices.get(index), &mut self.profiles.draft) else { return; };
        if let Some(index) = profile.apps.iter().position(|app| app.key == choice.app.key) {
            profile.apps.remove(index);
        } else if profile.apps.len() < profiles::APP_LIMIT {
            profile.apps.push(choice.app.clone());
            profile.apps.sort_by_cached_key(|app| app.name.to_lowercase());
        } else {
            self.profile_error(tr("Достигнут лимит приложений профиля."));
            return;
        }
        self.profiles.error = "";
        self.draw_window();
    }

    pub(crate) fn profile_char(&mut self, code: u16) {
        if !self.profiles.typing { return; }
        let Some(draft) = &mut self.profiles.draft else { return; };
        match code {
            8 => { self.profiles.pending_surrogate = None; draft.name.pop(); }
            13 => {
                self.profiles.typing = false;
                self.profiles.pending_surrogate = None;
                self.profiles.error = "";
            }
            0xD800..=0xDBFF => self.profiles.pending_surrogate = Some(code),
            _ if draft.name.chars().count() < 40 => {
                let mut pair = [code,0];
                let units = if let Some(high) = self.profiles.pending_surrogate.take() {
                    pair = [high,code];
                    &pair[..]
                } else { &pair[..1] };
                for value in char::decode_utf16(units.iter().copied()).flatten() {
                    if !value.is_control() { draft.name.push(value); }
                }
            }
            _ => (),
        }
        self.draw_window();
    }

    pub(crate) fn stop_shortcut_capture(&mut self) {
        self.profiles.capturing = false;
        self.profile_error("");
    }

    pub(crate) fn profile_shortcut(&mut self, value: Option<Shortcut>) {
        if let Some(draft) = &mut self.profiles.draft { draft.shortcut = value; }
        self.stop_shortcut_capture();
    }

    pub(crate) fn profile_hotkey(&mut self, id: i32) {
        if self.profiles.typing || self.profiles.capturing
            || self.form.as_ref().is_some_and(|form| form.capture) { return; }
        if self.profiles.registered_hotkeys.contains(&id) { self.activate_profile((id-HOTKEY_BASE) as usize); }
    }

    pub(crate) fn sync_profile_hotkeys(&mut self) {
        for id in self.profiles.registered_hotkeys.drain(..) {
            unsafe { UnregisterHotKey(self.host,id); }
        }
        for (index, profile) in self.profiles.list.iter().enumerate() {
            if let Some(shortcut) = profile.shortcut {
                let id = HOTKEY_BASE + index as i32;
                if unsafe { RegisterHotKey(self.host,id,shortcut.mods|MOD_NOREPEAT,shortcut.key) } != 0 {
                    self.profiles.registered_hotkeys.push(id);
                }
            }
        }
    }

    fn shortcut_available(&self, index: Option<usize>, shortcut: Shortcut) -> bool {
        if self.profiles.list.iter().enumerate().any(|(i,p)| Some(i) != index && p.shortcut == Some(shortcut)) {
            return false;
        }
        if index.is_some_and(|i| self.profiles.list[i].shortcut == Some(shortcut)
            && self.profiles.registered_hotkeys.contains(&(HOTKEY_BASE+i as i32))) {
            return true;
        }
        unsafe {
            if RegisterHotKey(self.host,HOTKEY_PROBE,shortcut.mods|MOD_NOREPEAT,shortcut.key) == 0 {
                return false;
            }
            UnregisterHotKey(self.host,HOTKEY_PROBE);
        }
        true
    }

    fn save_profile(&mut self) {
        let Some(mut draft) = self.profiles.draft.clone() else { return; };
        draft.name = draft.name.trim().into();
        if draft.name.is_empty() {
            self.profile_error(tr("У профиля должно быть имя."));
            return;
        }
        let index = self.profiles.draft_index;
        let name = draft.name.to_lowercase();
        if self.profiles.list.iter().enumerate().any(|(i,p)| Some(i) != index && p.name.to_lowercase() == name) {
            self.profile_error(tr("Такое имя уже занято."));
            return;
        }
        if draft.shortcut.is_some_and(|shortcut| !self.shortcut_available(index,shortcut)) {
            self.profile_error(tr("Горячая клавиша уже занята."));
            return;
        }
        let mut next = self.profiles.list.clone();
        if let Some(i) = index { next[i] = draft; } else { next.push(draft); }
        if !profiles::save(&next) {
            self.profile_error(tr("Не удалось сохранить профиль."));
            return;
        }
        self.profiles.list = next;
        self.sync_profile_hotkeys();
        if index.is_some() && index == self.profiles.running.as_ref().map(|running| running.index) {
            self.deactivate_profile();
        }
        self.profiles.draft = None;
        self.profiles.draft_index = None;
        self.profile_page(Page::List);
    }

    fn delete_profile(&mut self) {
        let Some(index) = self.profiles.draft_index else { return; };
        if !self.profiles.confirm_delete {
            self.profiles.confirm_delete = true;
            self.draw_window();
            return;
        }
        let mut next = self.profiles.list.clone();
        next.remove(index);
        if !profiles::save(&next) {
            self.profile_error(tr("Не удалось удалить профиль."));
            return;
        }
        self.profiles.list = next;
        self.sync_profile_hotkeys();
        if let Some(running) = &mut self.profiles.running {
            if running.index == index { self.deactivate_profile(); }
            else if running.index > index { running.index -= 1; }
        }
        self.profile_back();
    }

    fn profile_error(&mut self, text: &'static str) {
        self.profiles.error = text;
        self.draw_window();
    }

    fn activate_profile(&mut self, index: usize) {
        let Some(profile) = self.profiles.list.get(index).cloned() else { return; };
        if self.route_profile(index, &profile, true) {
            let status = if self.profiles.running.as_ref().is_some_and(|running| running.degraded) {
                tr("Профиль применён с запасным режимом")
            } else { tr("Профиль применён") };
            self.notify(status, &profile.name);
            self.profiles.error = "";
            self.draw_window();
        } else {
            self.profile_error(tr("Не удалось применить профиль."));
        }
    }

    fn route_profile(&mut self, index: usize, profile: &Profile, allow_primary: bool) -> bool {
        let Ok(devices) = self.audio.devices() else { return false; };
        let connected = |id: &str| devices.iter().any(|device| same(&device.id,id));
        let (target, level, degraded) = if allow_primary && connected(&profile.output.id) {
            (Some(&profile.output), profile.volume, false)
        } else {
            (profile.fallback.as_ref().filter(|device| connected(&device.id)),
                Volume { level: profile.fallback_level.into(), muted: profile.volume.muted }, true)
        };
        let mut seen = BTreeSet::new();
        let output = if let Some(target) = target {
            restore_apps(&mut self.audio, &target.id, &profile.apps, &mut seen);
            if !matches!(self.audio.select_with_volume(&target.id, Some(level)), Ok(true)) {
                return false;
            }
            restore_apps(&mut self.audio, &target.id, &profile.apps, &mut seen);
            Some(target.id.clone())
        } else {
            let current = self.audio.default_id(1).ok().flatten();
            if current.is_some() && self.audio.mute_output().is_err() { return false; }
            current
        };
        self.profiles.running = Some(Running { index, output, degraded, apps_allowed: target.is_some(), seen });
        self.refresh_volume();
        self.sync_profile_timer();
        true
    }

    pub(crate) fn deactivate_profile(&mut self) {
        self.profiles.running = None;
        self.sync_profile_timer();
    }

    pub(crate) fn profile_endpoint_changed(&mut self) {
        if let Some(running) = &self.profiles.running {
            let index = running.index;
            let profile = self.profiles.list[index].clone();
            if let (Ok(current), Ok(devices)) = (self.audio.default_id(1),self.audio.devices()) {
                let connected = |id: &str| devices.iter().any(|device| same(&device.id,id));
                let unchanged = same_output(current.as_deref(),running.output.as_deref());
                let primary = connected(&profile.output.id);
                let recovered = running.degraded && primary && profile.reconnect;
                let needed = (!primary && !running.degraded) || recovered
                    || (!unchanged && running.degraded && !running.output.as_ref().is_some_and(|id| connected(id)));
                if needed {
                    if !self.route_profile(index,&profile,!running.degraded || recovered) {
                        let _ = self.audio.mute_output();
                        self.deactivate_profile();
                        self.notify(tr("Не удалось восстановить выход"), &profile.name);
                    }
                } else if !unchanged {
                    self.deactivate_profile();
                }
            }
        }
        if self.kind == WindowKind::Profiles && matches!(self.profiles.page, Page::Output | Page::Fallback) {
            self.refresh_profile_outputs();
            self.geometry();
        }
        if self.kind == WindowKind::Profiles && self.profiles.page == Page::Mixer {
            if self.pressed.is_some() { self.profiles.live_sessions.clear(); }
            else {
                self.refresh_mixer();
                self.geometry();
            }
        }
    }

    pub(crate) fn sync_profile_timer(&mut self) {
        let interval = if self.kind == WindowKind::Profiles && self.profiles.page == Page::Mixer { 650 }
            else if self.profiles.running.as_ref().is_some_and(|running| running.apps_allowed
                && !self.profiles.list[running.index].apps.is_empty()) { 1500 }
            else { 0 };
        if self.profiles.timer_interval != interval {
            unsafe {
                if interval == 0 { KillTimer(self.host, TIMER); }
                else { SetTimer(self.host, TIMER, interval, None); }
            }
            self.profiles.timer_interval = interval;
        }
    }

    pub(crate) fn profile_tick(&mut self) {
        if self.pressed.is_some() { return; }
        if let Some(running) = &mut self.profiles.running {
            let profile = &self.profiles.list[running.index];
            if let Some(output) = running.output.as_ref() {
                if running.apps_allowed && !profile.apps.is_empty()
                    && self.audio.default_id(1).ok().flatten().is_some_and(|id| same(&id,output)) {
                    restore_apps(&mut self.audio,output,&profile.apps,&mut running.seen);
                }
            }
        }
        if self.kind == WindowKind::Profiles && self.profiles.page == Page::Mixer {
            self.refresh_mixer();
            if self.height != self.profile_model().height() { self.geometry(); }
            self.draw_window();
        }
    }
}

fn same(a: &str, b: &str) -> bool { a.eq_ignore_ascii_case(b) }
fn same_output(a: Option<&str>, b: Option<&str>) -> bool {
    match (a,b) { (Some(a),Some(b)) => same(a,b), _ => a == b }
}
fn output_data(device: &Device) -> Output { Output { id: device.id.clone(), name: device.name.clone() } }

fn grouped(sessions: &[Session]) -> Vec<AppChoice> {
    let mut values = BTreeMap::new();
    for session in sessions {
        if session.value.key.starts_with("pid:") { continue; }
        let (status,count) = values.entry(session.value.key.as_str()).or_insert((session,0));
        if session.active && !status.active { *status = session; }
        *count += 1;
    }
    let mut rows: Vec<_> = values.into_values().collect();
    rows.sort_by_cached_key(|(row,_)| row.value.name.to_lowercase());
    rows.into_iter().map(|(row,sessions)| AppChoice {app: row.value.clone(),sessions}).collect()
}

fn saved_apps(sessions: &[Session]) -> Vec<AppLevel> {
    grouped(sessions).into_iter().take(profiles::APP_LIMIT).map(|choice| choice.app).collect()
}

fn restore_apps(audio: &mut crate::audio::Audio, output: &str, apps: &[AppLevel], seen: &mut BTreeSet<String>) {
    if apps.is_empty() { return; }
    let Ok(sessions) = audio.sessions(Some(output)) else { return };
    let mut present = BTreeSet::new();
    for session in sessions {
        let Some(saved) = apps.iter().find(|app| app.key == session.value.key) else { continue; };
        if seen.contains(&session.instance) || session.value.volume == saved.volume
            || session.set(Some(saved.volume.level),Some(saved.volume.muted)) {
            present.insert(session.instance);
        }
    }
    *seen = present;
}
