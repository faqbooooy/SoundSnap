use alloc::{collections::{BTreeMap, BTreeSet}, format, string::String, vec::Vec};
use crate::{app_picker::{self, Choice, ExeSelection}, audio::{Device, Session, Volume}, automation, locale::tr, mixer, profile_ui::{Page, View},
    profiles::{self, AppLevel, Output, Profile, Trigger}, settings::Shortcut, ui::Action, App, WindowKind};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer,SetTimer};
const HOTKEY_BASE: i32 = 1000;
const HOTKEY_PROBE: i32 = 2000;

#[derive(Clone)]
struct Running {
    index: Option<usize>,
    profile: Profile,
    output: Option<String>,
    degraded: bool,
    apps_allowed: bool,
}

struct Pending { serial: u64, volume: Volume }

pub struct ProfileState {
    list: Vec<Profile>,
    draft: Option<Profile>,
    draft_index: Option<usize>,
    running: Option<Running>,
    before_automation: Option<Running>,
    pub page: Page,
    pub entry: Page,
    pub typing: bool,
    pub capturing: bool,
    automation: automation::State,
    query: String,
    select_text: bool,
    running_apps: Vec<Trigger>,
    resolved_popular: BTreeMap<usize, String>,
    show_popular: bool,
    start: usize,
    outputs: Vec<Device>,
    apps: Vec<AppLevel>,
    playing: Vec<bool>,
    choices: Vec<Choice>,
    mixer: Option<mixer::Worker>,
    mixer_rows: Vec<mixer::Row>,
    mixer_output: Option<Output>,
    mixer_override: Option<String>,
    mixer_epoch: u64,
    mixer_serial: u64,
    mixer_pending: BTreeMap<String,Pending>,
    mixer_deferred: Option<mixer::Snapshot>,
    mixer_numbers: BTreeMap<String,usize>,
    mixer_next_numbers: BTreeMap<String,usize>,
    mixer_loading: bool,
    mixer_unavailable: bool,
    mixer_incomplete: usize,
    mixer_write_failed: bool,
    confirm_delete: bool,
    error: &'static str,
    pending_surrogate: Option<u16>,
    registered_hotkeys: Vec<i32>,
}

impl ProfileState {
    pub fn new() -> Self {
        Self { list: profiles::load(), draft: None, draft_index: None, running: None, before_automation: None,
            page: Page::List, entry: Page::List, typing: false, capturing: false,
            automation: automation::State::default(), query: String::new(), select_text:false,
            running_apps:Vec::new(), resolved_popular:BTreeMap::new(), show_popular:false, start: 0, outputs: Vec::new(), apps: Vec::new(),
            playing: Vec::new(), choices: Vec::new(), mixer: None, mixer_rows: Vec::new(),
            mixer_output: None, mixer_override: None, mixer_epoch: 0, mixer_serial: 0,
            mixer_pending: BTreeMap::new(), mixer_deferred: None, mixer_numbers: BTreeMap::new(),
            mixer_next_numbers: BTreeMap::new(), mixer_loading: false, mixer_unavailable: false,
            mixer_incomplete: 0, mixer_write_failed: false, confirm_delete: false, error: "",
            pending_surrogate: None, registered_hotkeys: Vec::new() }
    }
    pub fn name(&self) -> Option<&str> {
        self.running.as_ref().map(|running| running.profile.name.as_str())
    }
    pub fn view(&self, available_height: f32) -> View<'_> {
        View {
            page: self.page, list: &self.list, selected: self.running.as_ref().and_then(|running| running.index),
            draft: self.draft.as_ref(), editing_existing: self.draft_index.is_some(), start: self.start,
            outputs: &self.outputs, apps: &self.apps, playing: &self.playing, choices: &self.choices, available_height,
            typing: self.typing, capturing: self.capturing, error: self.error, confirm_delete: self.confirm_delete,
            query: &self.query, show_popular: self.show_popular, resolved_popular: &self.resolved_popular,
            active_trigger: self.automation.active.and_then(|i| self.list.get(i)).and_then(|profile| profile.automation.trigger.as_ref()),
            profile_running: self.running.is_some(),
            mixer_output: self.mixer_output.as_ref().map(|output| output.name.as_str()),
            mixer_selected: self.mixer_override.as_deref(), mixer_loading: self.mixer_loading,
            mixer_unavailable: self.mixer_unavailable, mixer_incomplete: self.mixer_incomplete,
            mixer_write_failed: self.mixer_write_failed,
        }
    }
    pub fn close(&mut self) {
        self.draft = None;
        self.draft_index = None;
        self.typing = false;
        self.capturing = false;
        self.pending_surrogate = None;
        self.confirm_delete = false;
        self.select_text = false;
        self.query.clear();
        self.running_apps.clear();
        self.resolved_popular.clear();
        self.choices.clear();
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
    }

    fn profile_page(&mut self, page: Page) {
        self.hide_hint();
        self.profiles.page = page;
        self.profiles.start = 0;
        self.profiles.typing = false;
        self.profiles.capturing = false;
        self.profiles.pending_surrogate = None;
        self.profiles.select_text = false;
        self.profiles.query.clear();
        self.profiles.confirm_delete = false;
        self.profiles.error = "";
        self.focus = None;
        self.hover = None;
        match page {
            Page::MixerOutputs => self.refresh_profile_outputs(),
            Page::Output | Page::Fallback => self.refresh_profile_outputs(),
            Page::PickApps | Page::TriggerPicker => {
                self.profiles.show_popular = false;
                self.profiles.resolved_popular.clear();
                if let Some(draft) = &self.profiles.draft {
                    let keys = if page == Page::TriggerPicker {
                        draft.automation.trigger.iter().map(|trigger| trigger.key.as_str()).collect::<Vec<_>>()
                    } else { draft.apps.iter().map(|app| app.key.as_str()).collect() };
                    self.profiles.resolved_popular.extend(keys.into_iter()
                        .filter_map(|key| app_picker::popular_index(key).map(|index| (index,key.into()))));
                }
                self.scan_running_apps();
            }
            _ => (),
        }
        self.sync_mixer_watch();
        self.geometry();
        self.begin_page_motion();
        self.draw_window();
    }

    pub(crate) fn profile_back(&mut self) {
        match self.profiles.page {
            Page::List => self.show(WindowKind::Popup,self.anchor),
            Page::Mixer => self.show(WindowKind::Popup,self.anchor),
            Page::MixerOutputs => self.profile_page(Page::Mixer),
            Page::Edit => {
                self.profiles.draft = None;
                self.profiles.draft_index = None;
                self.profile_page(Page::List);
            }
            Page::Primary | Page::Safety | Page::Launch | Page::Apps => self.profile_page(Page::Edit),
            Page::Output => self.profile_page(Page::Primary),
            Page::Fallback => self.profile_page(Page::Safety),
            Page::TriggerPicker => self.profile_page(Page::Launch),
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
            Page::Mixer if self.pressed.is_none() => {
                self.apply_mixer_snapshot(mixer::Snapshot {
                    epoch:self.profiles.mixer_epoch, rows:self.profiles.mixer_rows.clone(),
                    incomplete:self.profiles.mixer_incomplete, unavailable:self.profiles.mixer_unavailable,
                });
            }
            Page::PickApps | Page::TriggerPicker => self.filter_catalog(),
            Page::Output | Page::Fallback => self.refresh_profile_outputs(),
            _ => (),
        }
    }

    pub(crate) fn clamp_profile_scroll(&mut self) {
        let view = self.profile_model();
        self.profiles.start = self.profiles.start.min(view.count().saturating_sub(view.rows()));
    }

    pub(crate) fn profile_scroll(&mut self, delta: i32) {
        let view = self.profile_model();
        let max = view.count().saturating_sub(view.rows());
        let next = if delta < 0 { (self.profiles.start+1).min(max) }
            else { self.profiles.start.saturating_sub(1) };
        if next != self.profiles.start {
            self.hide_hint();
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
            Action::MixerOutputs => self.profile_page(Page::MixerOutputs),
            Action::MixerOutputAutomatic => {
                self.profiles.mixer_override = None;
                self.profile_page(Page::Mixer);
            }
            Action::MixerOutputPick(i) => {
                if let Some(output) = self.profiles.outputs.get(i) {
                    self.profiles.mixer_override = Some(output.id.clone());
                    self.profile_page(Page::Mixer);
                }
            }
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
                self.profiles.select_text = true;
                self.profiles.capturing = false;
                self.profiles.error = tr("Введите имя. Enter завершает ввод.");
                self.draw_window();
            }
            Action::ProfileSave => self.save_profile(),
            Action::ProfileDelete => self.delete_profile(),
            Action::ProfilePreview => self.preview_profile(),
            Action::ProfilePrimary => self.profile_page(Page::Primary),
            Action::ProfileSafety => self.profile_page(Page::Safety),
            Action::ProfileLaunch => self.profile_page(Page::Launch),
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
                self.profile_page(Page::Safety);
            }
            Action::ProfileTrigger => self.profile_page(Page::TriggerPicker),
            Action::ProfileTriggerClear => {
                if let Some(draft) = &mut self.profiles.draft { draft.automation.trigger = None; }
                self.profile_page(Page::Launch);
            }
            Action::ProfileRequireOutput | Action::ProfileAutoReturn | Action::ProfilePriority(_) => {
                if let Some(draft) = &mut self.profiles.draft {
                    let options = &mut draft.automation;
                    match action {
                        Action::ProfileRequireOutput => options.require_output = !options.require_output,
                        Action::ProfileAutoReturn => options.restore = !options.restore,
                        Action::ProfilePriority(step) => options.priority = (i16::from(options.priority)+i16::from(step)).clamp(1,9) as u8,
                        _ => (),
                    }
                }
                self.draw_window();
            }
            Action::ProfileQuery => {
                self.profiles.typing = true;
                self.profiles.select_text = false;
                self.draw_window();
            }
            Action::ProfileCopyMixer => self.copy_mixer_levels(),
            Action::ProfileImportApps => self.profile_page(Page::PickApps),
            Action::ProfileAppPick(i) => self.pick_application(i),
            Action::ProfileBrowseExe => self.browse_application(),
            Action::ProfileAppSource(popular) => {
                self.profiles.show_popular = popular;
                self.profiles.start = 0;
                self.profiles.typing = false;
                self.profiles.error = "";
                if popular {
                    self.filter_catalog();
                    self.geometry();
                    self.draw_window();
                } else { self.refresh_running_picker(); }
            }
            Action::ProfileRefreshApps => self.refresh_running_picker(),
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
            Action::ProfilePrimaryCap | Action::ProfileFallbackCap => {
                if let Some(draft) = &mut self.profiles.draft {
                    if action == Action::ProfilePrimaryCap { draft.primary_cap = level as u8; }
                    else { draft.fallback_cap = level as u8; }
                }
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
            Action::ProfilePrimaryCap => draft.map(|p| p.primary_cap.into()),
            Action::ProfileFallbackCap => draft.map(|p| p.fallback_cap.into()),
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
        } else {
            self.mixer_change(index,level,toggle,true);
            return;
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
        self.profiles.draft_index = None;
        self.profiles.draft = Some(Profile {
            name: label, output: output_data(output), volume, apps:Vec::new(),
            fallback: None, fallback_level: 20, reconnect: true, shortcut: None,
            primary_cap:100, fallback_cap:100, automation:profiles::Automation::default(),
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
        self.profile_page(if self.profiles.page == Page::Fallback {Page::Safety} else {Page::Primary});
    }

    fn scan_running_apps(&mut self) {
        self.profiles.running_apps = automation::applications();
        self.filter_catalog();
    }

    pub(crate) fn refresh_running_picker(&mut self) {
        if self.profiles.show_popular { return; }
        self.scan_running_apps();
        self.hover = None;
        self.focus = None;
        self.geometry();
        self.draw_window();
    }

    fn filter_catalog(&mut self) {
        let query = self.profiles.query.trim().to_lowercase();
        self.profiles.choices = if self.profiles.show_popular {
            app_picker::POPULAR.iter().enumerate().map(|(i,_)| Choice::Popular(i))
                .filter(|row| matches_query(row.name(),row.path(),&query)).collect()
        } else {
            self.profiles.running_apps.iter().filter(|row| matches_query(&row.name,&row.key,&query))
                .cloned().map(Choice::Running).collect()
        };
        self.clamp_profile_scroll();
    }

    fn pick_application(&mut self, index: usize) {
        match self.profiles.choices.get(index).cloned() {
            Some(Choice::Running(app)) => self.add_application(app,true),
            Some(Choice::Popular(i)) => {
                let preferred = self.profiles.resolved_popular.get(&i).map(String::as_str);
                if let Some(app) = app_picker::locate(i,preferred,&self.profiles.running_apps) {
                    self.profiles.resolved_popular.insert(i,app.key.clone());
                    self.add_application(app,true);
                } else {
                    self.profiles.resolved_popular.remove(&i);
                    self.profile_error(tr("Приложение не найдено. Выбери EXE вручную."));
                }
            }
            None => (),
        }
    }

    fn browse_application(&mut self) {
        self.profiles.typing = false;
        self.hide_hint();
        match app_picker::select_exe(self.window.hwnd) {
            ExeSelection::Cancelled => { self.draw_window(); }
            ExeSelection::Invalid => self.profile_error(tr("Выбери существующий файл .exe.")),
            ExeSelection::Failed => self.profile_error(tr("Не удалось открыть выбор файла.")),
            ExeSelection::Selected(app) => self.add_application(app,false),
        }
    }

    fn copy_mixer_levels(&mut self) {
        let Ok(sessions) = self.audio.sessions(None) else {
            self.profile_error(tr("Микшер недоступен. Попробуй обновить список."));
            return;
        };
        let apps = saved_apps(&sessions);
        if apps.is_empty() {
            self.profile_error(tr("Запустите звук в нужном приложении."));
            return;
        }
        if let Some(draft) = &mut self.profiles.draft {
            for app in apps {
                if let Some(existing) = draft.apps.iter_mut().find(|r| app_picker::same_application(&r.key,&app.key)) { *existing = app; }
                else if draft.apps.len() < profiles::APP_LIMIT { draft.apps.push(app); }
            }
            draft.apps.sort_by_cached_key(|row| row.name.to_lowercase());
        }
        self.clamp_profile_scroll();
        self.geometry();
        self.profile_error(tr("Текущие уровни добавлены в профиль."));
    }

    fn add_application(&mut self, app: Trigger, toggle: bool) {
        let Some(profile) = &mut self.profiles.draft else { return; };
        if self.profiles.page == Page::TriggerPicker {
            profile.automation.trigger = Some(app);
            self.profile_page(Page::Launch);
            return;
        }
        let message = if let Some(index) = profile.apps.iter().position(|row| app_picker::same_application(&row.key,&app.key)) {
            if toggle {
                profile.apps.remove(index);
                ""
            } else { tr("Приложение уже добавлено.") }
        } else if profile.apps.len() < profiles::APP_LIMIT {
            profile.apps.push(AppLevel {key:app.key,name:app.name,volume:Volume {level:100,muted:false}});
            profile.apps.sort_by_cached_key(|row| row.name.to_lowercase());
            tr("Приложение добавлено.")
        } else { tr("Достигнут лимит приложений профиля.") };
        self.profile_error(message);
    }

    pub(crate) fn profile_char(&mut self, code: u16) {
        if !self.profiles.typing { return; }
        let search = self.profiles.page.is_app_picker();
        if code == 13 {
            self.profiles.typing = false;
            self.profiles.select_text = false;
            self.profiles.pending_surrogate = None;
            self.profiles.error = "";
            self.draw_window();
            return;
        }
        let text = if search { &mut self.profiles.query }
            else if let Some(draft) = &mut self.profiles.draft { &mut draft.name }
            else { return; };
        match code {
            8 => {
                self.profiles.pending_surrogate = None;
                if core::mem::take(&mut self.profiles.select_text) { text.clear(); }
                else { text.pop(); }
            }
            1 => self.profiles.select_text = true,
            0xD800..=0xDBFF => self.profiles.pending_surrogate = Some(code),
            _ => {
                let mut pair = [code,0];
                let units = if let Some(high) = self.profiles.pending_surrogate.take() {
                    pair = [high,code];
                    &pair[..]
                } else { &pair[..1] };
                for value in char::decode_utf16(units.iter().copied()).flatten() {
                    if value.is_control() { continue; }
                    if core::mem::take(&mut self.profiles.select_text) { text.clear(); }
                    if text.chars().count() < 60 { text.push(value); }
                }
            }
        }
        if search {
            self.profiles.start = 0;
            self.filter_catalog();
            self.geometry();
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
        if index.is_some() && index == self.profiles.running.as_ref().and_then(|running| running.index) {
            self.deactivate_profile();
        }
        self.sync_automation_timer();
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
        if self.profiles.running.as_ref().and_then(|running| running.index) == Some(index) {
            self.deactivate_profile();
        } else {
            if let Some(running) = &mut self.profiles.running {
                if let Some(i) = &mut running.index { if *i > index { *i -= 1; } }
            }
            if let Some(i) = &mut self.profiles.automation.active { if *i > index { *i -= 1; } }
        }
        if let Some(previous) = &mut self.profiles.before_automation {
            if previous.index == Some(index) {
                self.profiles.before_automation = None;
            } else if let Some(i) = &mut previous.index { if *i > index { *i -= 1; } }
        }
        self.profiles.automation.last_failed = None;
        self.sync_automation_timer();
        self.profile_back();
    }

    fn profile_error(&mut self, text: &'static str) {
        self.profiles.error = text;
        self.draw_window();
    }

    fn activate_profile(&mut self, index: usize) {
        let Some(profile) = self.profiles.list.get(index).cloned() else { return; };
        if self.route_profile(Some(index), &profile, true) {
            self.cancel_automation();
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

    fn preview_profile(&mut self) {
        let Some(profile) = self.profiles.draft.clone() else {return;};
        if self.route_profile(None,&profile,true) {
            self.cancel_automation();
            self.profile_error(tr("Профиль применён для проверки."));
        } else { self.profile_error(tr("Не удалось применить профиль.")); }
    }

    fn route_profile(&mut self, index: Option<usize>, profile: &Profile, allow_primary: bool) -> bool {
        let Ok(devices) = self.audio.devices() else { return false; };
        let connected = |id: &str| devices.iter().any(|device| same(&device.id,id));
        let (target,level,degraded) = automation::route(profile,allow_primary,connected);
        let previous_plan = self.profiles.mixer.as_ref().and_then(mixer::Worker::pause_plan);
        let mut seen = BTreeSet::new();
        let routed = if let Some(target) = target {
            let mut rollback = Vec::new();
            restore_apps(&mut self.audio, &target.id, &profile.apps, &mut seen, Some(&mut rollback));
            if matches!(self.audio.select_with_volume(&target.id, Some(level)), Ok(true)) {
                restore_apps(&mut self.audio, &target.id, &profile.apps, &mut seen, None);
                Ok(Some(target.id.clone()))
            } else {
                for (session, previous) in rollback {
                    session.set(Some(previous.level), Some(previous.muted));
                }
                Err(())
            }
        } else {
            self.audio.default_id(1).map_err(|_| ()).and_then(|current| {
                let current = current.filter(|id| connected(id));
                if current.is_some() { self.audio.mute_output().map_err(|_| ())?; }
                Ok(current)
            })
        };
        let Ok(output) = routed else {
            if let Some(worker) = &self.profiles.mixer { worker.plan(previous_plan); }
            return false;
        };
        self.profiles.running = Some(Running { index, profile:profile.clone(), output, degraded, apps_allowed: target.is_some() });
        self.refresh_volume();
        self.sync_mixer_plan(seen);
        true
    }

    pub(crate) fn deactivate_profile(&mut self) {
        self.cancel_automation();
        self.clear_running_profile();
    }

    fn clear_running_profile(&mut self) {
        self.profiles.running = None;
        self.sync_mixer_plan(BTreeSet::new());
    }

    pub(crate) fn profile_endpoint_changed(&mut self) {
        if let Some(running) = &self.profiles.running {
            let index = running.index;
            let profile = running.profile.clone();
            if let (Ok(current), Ok(devices)) = (self.audio.default_id(1),self.audio.devices()) {
                let connected = |id: &str| devices.iter().any(|device| same(&device.id,id));
                let unchanged = same_output(current.as_deref(),running.output.as_deref());
                let route_connected = running.output.as_ref().is_some_and(|id| connected(id));
                let action = automation::endpoint_action(&profile,running.degraded,running.apps_allowed,
                    unchanged,route_connected,connected);
                match action {
                    automation::EndpointAction::Apply {allow_primary} => {
                        if !self.route_profile(index,&profile,allow_primary) {
                            let _ = self.audio.mute_output();
                            self.deactivate_profile();
                            self.notify(tr("Не удалось восстановить выход"), &profile.name);
                        }
                    }
                    automation::EndpointAction::ManualOverride => self.deactivate_profile(),
                    automation::EndpointAction::Stay => (),
                }
            }
        }
        if self.kind == WindowKind::Profiles && matches!(self.profiles.page, Page::Output | Page::Fallback | Page::MixerOutputs) {
            self.refresh_profile_outputs();
            self.geometry();
        }
        if self.mixer_visible() {
            self.sync_mixer_watch();
            if let Some(worker) = &self.profiles.mixer { worker.reconcile(); }
        }
    }

    pub(crate) fn profile_cap(&self) -> i32 {
        self.profiles.running.as_ref().filter(|running| running.apps_allowed).map_or(100,|running| {
            if running.degraded { running.profile.fallback_cap.into() } else { running.profile.primary_cap.into() }
        })
    }

    pub(crate) fn cap_applies_to_current_output(&mut self) -> bool {
        let Some(id) = self.profiles.running.as_ref().and_then(|r| r.output.as_deref()) else {return false;};
        self.audio.default_id(1).ok().flatten().is_some_and(|current| same(id,&current))
    }

    pub(crate) fn sync_automation_timer(&mut self) {
        let watch = self.profiles.list.iter().any(|p| p.automation.trigger.is_some());
        if self.profiles.automation.watching != watch {
            unsafe {
                if watch {
                    self.profiles.automation.watching = SetTimer(self.host,automation::TIMER,automation::INTERVAL,None) != 0;
                } else {
                    KillTimer(self.host,automation::TIMER);
                    self.profiles.automation.watching = false;
                }
            }
        }
    }

    fn cancel_automation(&mut self) {
        self.profiles.before_automation = None;
        let state = &mut self.profiles.automation;
        state.active = None;
        state.baseline = None;
        state.restore_pending = None;
        state.last_failed = None;
        if state.watching {
            if let Some(running) = automation::running(&self.profiles.list) {
                state.suppressed.extend(running);
            }
        }
    }

    pub(crate) fn automation_tick(&mut self) {
        if self.pressed.is_some() { return; }
        let Some(mut running) = automation::running(&self.profiles.list) else {return;};
        automation::discard_suppressed(&mut running,&mut self.profiles.automation.suppressed);
        let old = self.profiles.automation.active;
        let needs_outputs = self.profiles.list.iter().enumerate().any(|(i,profile)| {
            Some(i) != old && profile.automation.require_output
                && profile.automation.trigger.as_ref().is_some_and(|trigger| running.contains_key(&trigger.key))
        });
        let connected = if needs_outputs {
            let Ok(devices) = self.audio.devices() else { return; };
            devices
        } else { Vec::new() };
        let winner = automation::best(&self.profiles.list,&running,old,
            |id| connected.iter().any(|output| same(id,&output.id)));
        if old == winner {
            if winner.is_none() {
                self.profiles.automation.last_failed = None;
                self.retry_baseline_sessions();
            }
            return;
        }
        if let Some(index) = winner {
            let previously_failed = self.profiles.automation.last_failed == Some(index);
            if previously_failed && self.profiles.automation.retry_after > 0 {
                self.profiles.automation.retry_after -= 1;
                return;
            }
            self.profiles.automation.restore_pending = None;
            let profile = self.profiles.list[index].clone();
            let baseline = if old.is_none() { self.capture_baseline() } else {None};
            if profile.automation.restore && old.is_none() && baseline.is_none() {
                self.profiles.automation.last_failed = Some(index);
                self.profiles.automation.retry_after = 4;
                if !previously_failed { self.notify(tr("Не удалось запомнить прежний звук"),&profile.name); }
                return;
            }
            let before = old.is_none().then(|| self.profiles.running.clone()).flatten();
            if self.route_profile(Some(index),&profile,true) {
                self.profiles.automation.active = Some(index);
                self.profiles.automation.last_failed = None;
                if old.is_none() {
                    self.profiles.automation.baseline = baseline;
                    self.profiles.before_automation = before;
                }
                let trigger = profile.automation.trigger.as_ref().map_or("",|trigger| trigger.name.as_str());
                self.notify(tr("Профиль включился автоматически"),&alloc::format!("{} · {trigger}",profile.name));
                self.draw_window();
            } else {
                self.profiles.automation.last_failed = Some(index);
                self.profiles.automation.retry_after = 4;
                if !previously_failed { self.notify(tr("Не удалось применить профиль."),&profile.name); }
            }
        } else if let Some(index) = old {
            self.profiles.automation.active = None;
            self.profiles.automation.last_failed = None;
            let restore = self.profiles.list.get(index).is_some_and(|profile| profile.automation.restore);
            let baseline = self.profiles.automation.baseline.take();
            let previous = self.profiles.before_automation.take();
            self.clear_running_profile();
            if let Some(mut baseline) = baseline.filter(|_| restore) {
                if matches!(self.audio.select_with_volume(&baseline.output.id,Some(baseline.volume)),Ok(true)) {
                    restore_baseline_sessions(&mut self.audio,&mut baseline);
                    self.profiles.running = previous.filter(|previous| previous.output.as_deref()
                        .is_some_and(|id| same(id,&baseline.output.id)));
                    let name = self.profiles.running.as_ref()
                        .map_or_else(|| baseline.output.name.clone(),|running| running.profile.name.clone());
                    if self.profiles.running.is_some() {
                        self.sync_mixer_plan(baseline.restored);
                    } else {
                        self.profiles.automation.restore_pending = Some((baseline,2));
                    }
                    self.notify(tr("Прежний звук восстановлен"),&name);
                } else { self.notify(tr("Прежний выход недоступен"),&baseline.output.name); }
            }
            if self.kind == WindowKind::Popup { self.refresh_devices(); } else {self.refresh_volume();}
            self.draw_window();
        }
    }

    fn capture_baseline(&mut self) -> Option<automation::Baseline> {
        let devices = self.audio.devices().ok()?;
        let output = output_data(devices.iter().find(|d| d.selected)?);
        let volume = self.audio.volume_at(&output.id).ok()?;
        let sessions = self.audio.sessions(Some(&output.id)).ok()?.into_iter().map(|session| automation::SavedSession {
            instance:session.instance,key:session.value.key,volume:session.value.volume,
        }).collect();
        self.audio.default_id(1).ok().flatten().filter(|current| same(current,&output.id))?;
        Some(automation::Baseline {output,volume,sessions,restored:BTreeSet::new()})
    }

    fn retry_baseline_sessions(&mut self) {
        let Some((mut baseline,attempts)) = self.profiles.automation.restore_pending.take() else {return;};
        if self.audio.default_id(1).ok().flatten().is_some_and(|id| same(&id,&baseline.output.id)) {
            restore_baseline_sessions(&mut self.audio,&mut baseline);
            if attempts > 1 { self.profiles.automation.restore_pending = Some((baseline,attempts-1)); }
        }
    }
}
impl App {
    fn mixer_visible(&self) -> bool {
        self.kind == WindowKind::Profiles && matches!(self.profiles.page,Page::Mixer | Page::MixerOutputs)
    }

    fn ensure_mixer_worker(&mut self) -> bool {
        if self.profiles.mixer.is_none() { self.profiles.mixer = mixer::Worker::start(self.host); }
        self.profiles.mixer.is_some()
    }

    pub(crate) fn stop_mixer_worker(&mut self) { self.profiles.mixer = None; }

    pub(crate) fn sync_mixer_watch(&mut self) {
        if !self.mixer_visible() {
            // Keep the old output alive until the final value of a just-ended drag is applied.
            if self.profiles.mixer_pending.is_empty() && self.profiles.mixer_output.is_some() {
                self.replace_mixer_output(None);
            }
            return;
        }
        let outputs = match self.audio.devices() {
            Ok(outputs) => outputs,
            Err(_) => {
                self.replace_mixer_output(None);
                self.profiles.mixer_unavailable = true;
                return;
            }
        };
        let explicit = self.profiles.mixer_override.as_ref()
            .and_then(|id| outputs.iter().find(|output| same(id,&output.id)));
        if explicit.is_none() { self.profiles.mixer_override = None; }
        let output = explicit.or_else(|| outputs.iter().find(|output| output.selected)).map(output_data);
        if !same_output(self.profiles.mixer_output.as_ref().map(|o| o.id.as_str()),output.as_ref().map(|o| o.id.as_str())) {
            self.replace_mixer_output(output);
        } else {
            if output.is_none() { self.profiles.mixer_unavailable = false; }
            self.profiles.mixer_output = output;
        }
    }

    fn replace_mixer_output(&mut self, output: Option<Output>) {
        let p = &mut self.profiles;
        p.mixer_epoch = p.mixer_epoch.wrapping_add(1);
        p.mixer_output = output;
        p.mixer_rows.clear();
        p.mixer_pending.clear();
        p.mixer_deferred = None;
        p.mixer_numbers.clear();
        p.mixer_next_numbers.clear();
        p.mixer_loading = p.mixer_output.is_some();
        p.mixer_unavailable = false;
        p.mixer_incomplete = 0;
        p.mixer_write_failed = false;
        p.apps.clear();
        p.playing.clear();
        p.start = 0;
        if self.kind == WindowKind::Profiles && self.profiles.page == Page::Mixer {
            self.focus = None;
            self.hover = None;
            if matches!(self.pressed,Some(Action::AppVolume(_) | Action::AppMute(_))) {
                self.pressed = None;
                unsafe {
                    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetCapture,ReleaseCapture};
                    if GetCapture() == self.window.hwnd { ReleaseCapture(); }
                }
            }
        }
        let watch = self.profiles.mixer_output.as_ref().map(|output| mixer::Watch {
            epoch:self.profiles.mixer_epoch, output:output.id.clone(),
        });
        if watch.is_some() && !self.ensure_mixer_worker() {
            self.profiles.mixer_loading = false;
            self.profiles.mixer_unavailable = true;
        }
        if let Some(worker) = &self.profiles.mixer { worker.watch(watch); }
    }

    fn sync_mixer_plan(&mut self, seen: BTreeSet<String>) {
        let plan = self.profiles.running.as_ref().and_then(|running| {
            let profile = &running.profile;
            if !running.apps_allowed || profile.apps.is_empty() { return None; }
            Some(mixer::Plan {output:running.output.clone()?,apps:profile.apps.clone(),seen})
        });
        if plan.is_some() { self.ensure_mixer_worker(); }
        if let Some(worker) = &self.profiles.mixer { worker.plan(plan); }
    }

    pub(crate) fn profile_drag(&mut self, action: Action, level: i32, final_value: bool) {
        match action {
            Action::AppVolume(i) if self.profiles.page == Page::Mixer => self.mixer_change(i,Some(level),false,final_value),
            _ => self.profile_slider(action,level),
        }
    }

    fn mixer_change(&mut self, index: usize, level: Option<i32>, toggle: bool, final_value: bool) {
        let p = &mut self.profiles;
        let (Some(worker),Some(row),Some(app)) = (p.mixer.as_ref(),p.mixer_rows.get(index),p.apps.get_mut(index)) else { return; };
        let mute = toggle.then_some(!app.volume.muted);
        let preview = Volume {level:level.unwrap_or(app.volume.level).clamp(0,100),muted:mute.unwrap_or(app.volume.muted)};
        let changed = preview != app.volume;
        if !changed && !(final_value && p.mixer_pending.contains_key(&row.instance)) { return; }
        p.mixer_serial = p.mixer_serial.wrapping_add(1);
        let serial = p.mixer_serial;
        p.mixer_pending.insert(row.instance.clone(),Pending {serial,volume:preview});
        app.volume = preview;
        worker.write(mixer::Change {
            epoch:p.mixer_epoch, instance:row.instance.clone(), serial,
            level:level.map(|_| preview.level), mute, final_value,
        });
        if changed { self.draw_window(); }
    }

    pub(crate) fn mixer_updates(&mut self) {
        let Some(worker) = &self.profiles.mixer else { return; };
        let updates = worker.updates();
        let mut redraw = false;
        let mut snapshot_applied = false;
        if let Some(snapshot) = updates.snapshot.filter(|s| s.epoch == self.profiles.mixer_epoch) {
            if self.pressed.is_some() && self.kind == WindowKind::Profiles && self.profiles.page == Page::Mixer {
                self.profiles.mixer_deferred = Some(snapshot);
            } else {
                redraw |= self.apply_mixer_snapshot(snapshot);
                snapshot_applied = true;
            }
        }
        for ack in updates.applied.into_iter().filter(|ack| ack.epoch == self.profiles.mixer_epoch) {
            if !self.profiles.mixer_pending.get(&ack.instance).is_some_and(|pending| pending.serial <= ack.serial) { continue; }
            self.profiles.mixer_pending.remove(&ack.instance);
            let error = ack.volume.is_none();
            redraw |= error != self.profiles.mixer_write_failed;
            self.profiles.mixer_write_failed = error;
            if let Some(index) = self.profiles.mixer_rows.iter().position(|row| row.instance == ack.instance) {
                let row = &mut self.profiles.mixer_rows[index];
                if !snapshot_applied {
                    if let Some(volume) = ack.volume { row.value.volume = volume; }
                }
                let preview = &mut self.profiles.apps[index].volume;
                redraw |= *preview != row.value.volume;
                *preview = row.value.volume;
            }
        }
        if !self.mixer_visible() { self.sync_mixer_watch(); }
        if redraw && self.kind == WindowKind::Profiles && self.profiles.page == Page::Mixer {
            if self.height != self.profile_model().height() { self.geometry(); }
            self.draw_window();
        }
    }

    pub(crate) fn finish_mixer_gesture(&mut self, pressed: Option<Action>) {
        if self.kind != WindowKind::Profiles || self.profiles.page != Page::Mixer { return; }
        if let Some(Action::AppVolume(i)) = pressed {
            if let Some(level) = self.profile_slider_level(Action::AppVolume(i)) {
                self.mixer_change(i,Some(level),false,true);
            }
        }
        if let Some(snapshot) = self.profiles.mixer_deferred.take() {
            if self.apply_mixer_snapshot(snapshot) {
                if self.height != self.profile_model().height() { self.geometry(); }
                self.draw_window();
            }
        }
    }

    fn apply_mixer_snapshot(&mut self, snapshot: mixer::Snapshot) -> bool {
        let previous_apps = core::mem::take(&mut self.profiles.apps);
        let previous_playing = core::mem::take(&mut self.profiles.playing);
        let status_changed = self.profiles.mixer_loading || self.profiles.mixer_unavailable != snapshot.unavailable
            || self.profiles.mixer_incomplete != snapshot.incomplete;
        let previous_focus = self.focus;
        let previous_start = self.profiles.start;
        let previous_ids: Vec<_> = self.profiles.mixer_rows.iter().map(|row| row.instance.clone()).collect();
        let focused = match self.focus {
            Some(Action::AppVolume(i)) => self.profiles.mixer_rows.get(i).map(|r| (r.instance.clone(),false)),
            Some(Action::AppMute(i)) => self.profiles.mixer_rows.get(i).map(|r| (r.instance.clone(),true)),
            _ => None,
        };
        if matches!(self.focus,Some(Action::AppVolume(_) | Action::AppMute(_))) { self.focus = None; }
        self.profiles.mixer_loading = false;
        self.profiles.mixer_unavailable = snapshot.unavailable;
        self.profiles.mixer_incomplete = snapshot.incomplete;
        self.profiles.mixer_rows = snapshot.rows;
        self.rebuild_mixer_rows();
        if let Some((instance,mute)) = focused {
            if let Some(i) = self.profiles.mixer_rows.iter().position(|row| row.instance == instance) {
                self.focus = Some(if mute {Action::AppMute(i)} else {Action::AppVolume(i)});
                let rows = self.profile_model().rows();
                if i < self.profiles.start { self.profiles.start = i; }
                else if i >= self.profiles.start+rows { self.profiles.start = i+1-rows; }
                self.clamp_profile_scroll();
            }
        }
        let rows_changed = !previous_ids.iter().map(String::as_str)
            .eq(self.profiles.mixer_rows.iter().map(|row| row.instance.as_str()));
        if rows_changed && matches!(self.hover,Some(Action::AppVolume(_) | Action::AppMute(_))) {
            self.hover = None;
        }
        status_changed || rows_changed || previous_apps != self.profiles.apps || previous_playing != self.profiles.playing
            || previous_focus != self.focus || previous_start != self.profiles.start
    }

    fn rebuild_mixer_rows(&mut self) {
        let p = &mut self.profiles;
        p.mixer_rows.sort_by_cached_key(|row| (row.value.display_name().to_lowercase(),row.instance.clone()));
        p.mixer_numbers.retain(|instance,_| p.mixer_rows.iter().any(|row| row.instance == *instance));
        let mut names = BTreeMap::new();
        for row in &p.mixer_rows { *names.entry(row.value.display_name().to_lowercase()).or_insert(0usize) += 1; }
        p.mixer_next_numbers.retain(|name,_| names.contains_key(name));
        let (apps,playing) = p.mixer_rows.iter().map(|row| {
            let mut app = row.value.clone();
            if let Some(pending) = p.mixer_pending.get(&row.instance) { app.volume = pending.volume; }
            let name = app.display_name().to_lowercase();
            let number = if let Some(number) = p.mixer_numbers.get(&row.instance) { Some(*number) }
            else if names[&name] > 1 {
                let next = p.mixer_next_numbers.entry(name).or_insert(0);
                *next += 1;
                p.mixer_numbers.insert(row.instance.clone(),*next);
                Some(*next)
            } else { None };
            if let Some(number) = number { app.name = format!("{} · {number}",app.display_name()); }
            (app,row.active)
        }).unzip();
        p.apps = apps;
        p.playing = playing;
        self.clamp_profile_scroll();
    }
}

fn same(a: &str, b: &str) -> bool { a.eq_ignore_ascii_case(b) }
pub(crate) fn restore_baseline_sessions(audio: &mut crate::audio::Audio, baseline: &mut automation::Baseline) {
    if let Ok(sessions) = audio.sessions(Some(&baseline.output.id)) {
        for session in sessions {
            if baseline.restored.contains(&session.instance) { continue; }
            let saved = baseline.sessions.iter().find(|r| r.instance == session.instance).or_else(|| {
                if session.value.key.starts_with("pid:") {return None;}
                let mut matches = baseline.sessions.iter()
                    .filter(|row| app_picker::same_application(&row.key,&session.value.key));
                matches.next().filter(|_| matches.next().is_none())
            });
            if saved.is_some_and(|saved| saved.volume == session.value.volume
                || session.set(Some(saved.volume.level),Some(saved.volume.muted))) {
                baseline.restored.insert(session.instance);
            }
        }
    }
}
fn matches_query(name: &str, key: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query) || key.to_lowercase().contains(query)
}
fn same_output(a: Option<&str>, b: Option<&str>) -> bool {
    match (a,b) { (Some(a),Some(b)) => same(a,b), _ => a == b }
}
fn output_data(device: &Device) -> Output { Output { id: device.id.clone(), name: device.name.clone() } }

fn saved_apps(sessions: &[Session]) -> Vec<AppLevel> {
    let mut values = BTreeMap::<_, &Session>::new();
    for session in sessions.iter().filter(|row| !row.value.key.starts_with("pid:")) {
        let best = values.entry(session.value.key.as_str()).or_insert(session);
        if session.active && !best.active { *best = session; }
    }
    let mut rows: Vec<_> = values.into_values().collect();
    rows.sort_by_cached_key(|row| row.value.name.to_lowercase());
    rows.into_iter().take(profiles::APP_LIMIT).map(|row| row.value.clone()).collect()
}

pub(crate) fn restore_apps(audio: &mut crate::audio::Audio, output: &str, apps: &[AppLevel], seen: &mut BTreeSet<String>,
    mut rollback: Option<&mut Vec<(Session,Volume)>>) {
    if apps.is_empty() { return; }
    let Ok(sessions) = audio.sessions(Some(output)) else { return };
    let mut present = BTreeSet::new();
    for session in sessions {
        let Some(saved) = apps.iter().find(|app| app.key == session.value.key)
            .or_else(|| apps.iter().find(|app| app_picker::controls_session(&app.key,&session.value.key))) else { continue; };
        let previous = session.value.volume;
        if seen.contains(&session.instance) || previous == saved.volume {
            present.insert(session.instance);
            continue;
        }
        if session.set(Some(saved.volume.level),Some(saved.volume.muted)) {
            present.insert(session.instance.clone());
        }
        if let Some(rollback) = rollback.as_deref_mut() {
            rollback.push((session,previous));
        }
    }
    *seen = present;
}
