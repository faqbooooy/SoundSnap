use alloc::{collections::BTreeMap, format, string::String, vec::Vec};
use windows_sys::Win32::Graphics::GdiPlus::RectF;
use crate::{app_picker::Choice, audio::Device, gfx::{point, rect, Painter, WHITE}, locale::tr, palette::Palette,
    profiles::{AppLevel, Profile, Trigger, LIMIT}, ui::{button, close, Action}};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page { List, Mixer, MixerOutputs, Edit, Primary, Apps, PickApps, Output, Safety, Fallback, Launch, TriggerPicker }

impl Page {
    pub fn is_app_picker(self) -> bool { matches!(self, Self::PickApps | Self::TriggerPicker) }
}

pub struct View<'a> {
    pub page: Page,
    pub list: &'a [Profile],
    pub selected: Option<usize>,
    pub draft: Option<&'a Profile>,
    pub editing_existing: bool,
    pub confirm_delete: bool,
    pub apps: &'a [AppLevel],
    pub playing: &'a [bool],
    pub choices: &'a [Choice],
    pub query: &'a str,
    pub show_popular: bool,
    pub resolved_popular: &'a BTreeMap<usize,String>,
    pub active_trigger: Option<&'a Trigger>,
    pub profile_running: bool,
    pub available_height: f32,
    pub outputs: &'a [Device],
    pub start: usize,
    pub typing: bool,
    pub capturing: bool,
    pub error: &'a str,
    pub mixer_output: Option<&'a str>,
    pub mixer_selected: Option<&'a str>,
    pub mixer_loading: bool,
    pub mixer_unavailable: bool,
    pub mixer_incomplete: usize,
    pub mixer_write_failed: bool,
}

const MAX_ROWS: usize = 8;
const LEFT: f32 = 16.;
const WIDTH: f32 = 294.;
const SLIDER_LEFT: f32 = 23.;
const SLIDER_WIDTH: f32 = 204.;
const CLOSE_X: f32 = 286.;
const HEADER_Y: f32 = 16.;

impl View<'_> {
    pub fn count(&self) -> usize {
        match self.page {
            Page::List => self.list.len() + usize::from(self.list.len() < LIMIT),
            Page::Mixer => self.apps.len(),
            Page::Apps => self.draft.map_or(0, |profile| profile.apps.len()),
            Page::PickApps | Page::TriggerPicker => self.choices.len(),
            Page::Output | Page::Fallback | Page::MixerOutputs => self.outputs.len(),
            _ => 0,
        }
    }
    pub fn height(&self) -> i32 {
        match self.page {
            Page::Edit => if self.editing_existing {376} else {342},
            Page::Primary => 301,
            Page::Safety => if self.draft.is_some_and(|p| p.fallback.is_some()) {332} else {219},
            Page::Launch => 341,
            Page::List => (105. + self.count().clamp(1,self.rows()).div_ceil(2) as f32 * self.row_step()) as i32,
            _ => (144. + self.mixer_extra() + self.search_extra()
                + self.count().clamp(1,self.rows()) as f32 * self.row_step()) as i32,
        }
    }
    pub fn rows(&self) -> usize {
        if self.page == Page::List {
            return (((self.available_height - 110.) / self.row_step()) as usize).clamp(1,4) * 2;
        }
        (((self.available_height - 164. - self.mixer_extra() - self.search_extra()) / self.row_step()) as usize).clamp(1,MAX_ROWS)
    }
    fn row_step(&self) -> f32 {
        match self.page { Page::List => 84., Page::Mixer | Page::Apps => 48., _ => 39. }
    }
    fn mixer_extra(&self) -> f32 { if self.page == Page::Mixer { 28. } else { 0. } }
    fn search_extra(&self) -> f32 {
        if self.page.is_app_picker() { 73. } else { 0. }
    }
    fn row_y(&self, row: usize) -> f32 { 87. + self.mixer_extra() + self.search_extra() + row as f32 * self.row_step() }
    fn footer(&self) -> f32 { self.height() as f32 - 46. }
}

pub fn level(x: f32) -> i32 {
    ((x-SLIDER_LEFT)*100./SLIDER_WIDTH+0.5).clamp(0.,100.) as i32
}
fn bar(y: f32) -> RectF { rect(SLIDER_LEFT-6.,y-9.,SLIDER_WIDTH+12.,18.) }
fn header(x: f32) -> RectF { rect(x,HEADER_Y,26.,26.) }
fn field(y: f32) -> RectF { rect(LEFT,y,WIDTH,31.) }
fn command(y: f32) -> RectF { rect(LEFT,y,WIDTH,32.) }

pub fn controls(v: &View) -> Vec<(Action, RectF)> {
    let mut controls = alloc::vec![(Action::ProfileBack,header(16.)), (Action::Close,header(CLOSE_X))];
    match v.page {
        Page::List | Page::Mixer => {
            let tab = if v.page == Page::List {Action::Mixer} else {Action::Profiles};
            controls.push((tab, rect(208.,HEADER_Y,73.,26.)));
            if v.page == Page::List && v.profile_running {
                controls.push((Action::ProfileDeactivate,rect(216.,58.,94.,22.)));
            }
            if v.page == Page::Mixer { controls.push((Action::MixerOutputs,rect(LEFT,58.,WIDTH,29.))); }
        }
        Page::Edit => {
            setting(&mut controls, Action::ProfileName, Help::Name, field(78.));
            for (i, (action, help)) in [
                (Action::ProfilePrimary, Help::Output), (Action::ProfileApps, Help::Apps),
                (Action::ProfileSafety, Help::Fallback), (Action::ProfileLaunch, Help::Launch),
            ].into_iter().enumerate() {
                setting(&mut controls, action, help, tile(i,120.,82.));
            }
            controls.extend([
                (Action::ProfilePreview,rect(LEFT,298.,142.,31.)),
                (Action::ProfileSave,rect(168.,298.,142.,31.)),
            ]);
            if v.editing_existing { controls.push((Action::ProfileDelete,rect(LEFT,338.,142.,24.))); }
        }
        Page::Primary => {
            setting(&mut controls, Action::ProfileOutput, Help::Output, field(80.));
            controls.push((Action::ProfileMaster,bar(144.)));
            setting(&mut controls, Action::ProfileMasterMute, Help::Mute, field(166.));
            controls.push((Action::ProfilePrimaryCap,bar(229.)));
            controls.extend([tip(Help::Master, 119.), tip(Help::Cap, 204.), (Action::ProfileBack,command(v.footer()))]);
        }
        Page::Safety => {
            setting(&mut controls,Action::ProfileFallback,Help::Fallback,field(80.));
            if v.draft.is_some_and(|profile| profile.fallback.is_some()) {
                controls.extend([(Action::ProfileFallbackLevel,bar(144.)), tip(Help::FallbackLevel,119.),
                    (Action::ProfileFallbackCap,bar(198.)), tip(Help::FallbackCap,173.)]);
                setting(&mut controls,Action::ProfileReconnect,Help::Reconnect,field(232.));
            } else {
                setting(&mut controls,Action::ProfileReconnect,Help::Reconnect,field(124.));
            }
            controls.push((Action::ProfileBack,command(v.footer())));
        }
        Page::Launch => {
            setting(&mut controls,Action::ProfileShortcut,Help::Shortcut,field(79.));
            setting(&mut controls,Action::ProfileTrigger,Help::Trigger,field(120.));
            setting(&mut controls,Action::ProfileRequireOutput,Help::RequireOutput,field(161.));
            controls.extend([(Action::ProfilePriority(-1),rect(215.,202.,23.,31.)),
                (Action::ProfilePriority(1),rect(263.,202.,23.,31.)),tip(Help::Priority,209.)]);
            setting(&mut controls,Action::ProfileAutoReturn,Help::AutoReturn,field(243.));
            controls.push((Action::ProfileBack,command(v.footer())));
        }
        Page::Apps => {
            controls.push((Action::ProfileImportApps,rect(LEFT,v.footer(),139.,32.)));
            setting(&mut controls,Action::ProfileCopyMixer,Help::Copy,rect(163.,v.footer(),147.,32.));
        }
        Page::PickApps | Page::TriggerPicker => {
            controls.extend([
                (Action::ProfileAppSource(false),rect(LEFT,83.,127.,28.)),
                (Action::ProfileAppSource(true),rect(149.,83.,127.,28.)),
                (Action::ProfileBrowseExe,rect(LEFT,v.footer(),142.,32.)),
            ]);
            if !v.show_popular { controls.push((Action::ProfileRefreshApps,rect(282.,83.,28.,28.))); }
            setting(&mut controls,Action::ProfileQuery,Help::Search,field(119.));
            let right = if v.page == Page::PickApps {Action::ProfileBack} else {Action::ProfileTriggerClear};
            controls.push((right,rect(168.,v.footer(),142.,32.)));
        }
        Page::Fallback => controls.push((Action::ProfileMuteFallback,command(v.footer()))),
        Page::MixerOutputs => controls.push((Action::MixerOutputAutomatic,command(v.footer()))),
        Page::Output => (),
    }
    let can_create = v.page == Page::List && v.list.len() < LIMIT;
    if can_create { controls.push((Action::ProfileCapture,tile(0,v.row_y(0),76.))); }
    let reserved = usize::from(can_create);
    for (row, index) in (v.start..(v.start + v.rows() - reserved).min(v.count() - reserved)).enumerate() {
        let y = v.row_y(row);
        match v.page {
            Page::List => {
                let r = tile(row+reserved,v.row_y(0),76.);
                controls.push((Action::ProfileEdit(index),rect(r.X+r.Width-30.,r.Y+6.,24.,23.)));
                controls.push((Action::ProfileApply(index),r));
            }
            Page::Mixer | Page::Apps => {
                controls.push((Action::AppVolume(index),bar(y+32.)));
                controls.push((Action::AppMute(index),rect(268.,y+11.,v_mute_width(v.page),27.)));
                if v.page == Page::Apps {
                    controls.push((Action::AppForget(index),rect(293.,y+11.,17.,27.)));
                    controls.push((Action::Help(Help::AppRow(index)),rect(244.,y-3.,21.,21.)));
                }
            }
            Page::Output | Page::Fallback => controls.push((Action::ProfileOutputPick(index),field(y))),
            Page::MixerOutputs => controls.push((Action::MixerOutputPick(index),field(y))),
            Page::PickApps | Page::TriggerPicker => controls.push((Action::ProfileAppPick(index),rect(LEFT,y,WIDTH,34.))),
            _ => (),
        }
    }
    controls
}

fn tile(index: usize, y: f32, height: f32) -> RectF {
    rect(LEFT + (index%2) as f32*151.,y + (index/2) as f32*(height+8.),143.,height)
}

fn setting(controls: &mut Vec<(Action,RectF)>, action: Action, help: Help, r: RectF) {
    controls.extend([
        (Action::Help(help),rect(r.X+r.Width-25.,r.Y+3.,22.,22.)),
        (action,r),
    ]);
}

fn tip(help: Help, y: f32) -> (Action,RectF) {
    (Action::Help(help),rect(287.,y-2.,22.,22.))
}

fn v_mute_width(page: Page) -> f32 { if page == Page::Apps {23.} else {42.} }

pub fn draw(g: &Painter, p: &Palette, v: &View, hover: Option<Action>, focus: Option<Action>) {
    g.use_panel_typography();
    g.gradient(rect(0.5,0.5,325.,v.height() as f32-1.),18.,p.glass_start,p.glass_end,p.glass_edge);
    let title = match v.page {
        Page::List => tr("Профили звука"), Page::Mixer => tr("Микшер"),
        Page::MixerOutputs => tr("Выход микшера"),
        Page::Edit if !v.editing_existing => tr("Новый профиль"),
        Page::Edit => tr("Настройка профиля"), Page::Apps => tr("Громкость приложений"),
        Page::PickApps => tr("Выбор приложений"),
        Page::Output | Page::Primary => tr("Основной выход"), Page::Fallback => tr("Запасной выход"),
        Page::Safety => tr("Если выход отключится"), Page::Launch => tr("Когда включать профиль"),
        Page::TriggerPicker => tr("Автозапуск профиля"),
    };
    g.text(title,rect(52.,HEADER_Y,if matches!(v.page,Page::List | Page::Mixer) {152.} else {224.},26.),16.,true,0,p.ink);
    let icon_x = 29.;
    let icon_y = 29.;
    button(g,p,header(16.),7.,if hover == Some(Action::ProfileBack) {p.hover} else {p.elevated},
        focus == Some(Action::ProfileBack));
    g.stroke(&[point(icon_x+4.,icon_y),point(icon_x-4.,icon_y)],1.5,p.ink);
    g.stroke(&[point(icon_x,icon_y-4.),point(icon_x-4.,icon_y),point(icon_x,icon_y+4.)],1.5,p.ink);
    close(g,p,CLOSE_X,HEADER_Y,hover == Some(Action::Close),focus == Some(Action::Close));
    g.fill(rect(LEFT,54.,WIDTH,1.),p.line);

    let auto_status = v.active_trigger.map(|trigger| format!("{} · {}",tr("Авто"),trigger.name));
    let hint = if !v.error.is_empty() { v.error } else { match v.page {
        Page::List => if v.list.is_empty() { tr("Сохрани настройки для игры, работы или музыки.") }
            else if v.profile_running { auto_status.as_deref().unwrap_or(tr("Профиль включён")) }
            else { tr("Выбери профиль, чтобы включить его.") },
        Page::Mixer => "",
        Page::MixerOutputs => tr("Выбери устройство, сессии которого хочешь видеть."),
        Page::Apps => tr("Уровни применяются при включении профиля."),
        Page::PickApps | Page::TriggerPicker if v.show_popular => tr("Выбери приложение, чтобы найти его на ПК"),
        Page::PickApps => tr("Приложения, запущенные сейчас"),
        Page::TriggerPicker => tr("Выбери приложение для автозапуска профиля"),
        Page::Output => tr("Куда выводить звук при включении профиля"),
        Page::Fallback => tr("Куда вывести звук, если основной выход пропадёт"),
        _ => "",
    }};
    if v.page == Page::Mixer {
        let count = format!("{}: {}",tr("Сессий"),v.apps.len());
        g.text(&count,rect(LEFT,91.,76.,17.),10.5,true,0,p.secondary_ink);
        let status = if v.mixer_write_failed {
            tr("Не удалось изменить громкость.").into()
        } else if v.mixer_incomplete > 0 {
            format!("{}: {}",tr("Недоступно"),v.mixer_incomplete)
        } else { "".into() };
        g.text(&status,rect(100.,91.,210.,17.),10.5,false,2,p.secondary_ink);
    } else {
        g.text(hint,rect(LEFT,59.,if v.page == Page::List && v.profile_running {196.} else {WIDTH},18.),
            10.5,false,0,p.secondary_ink);
    }
    if v.count() == 0 && !matches!(v.page, Page::Edit | Page::Primary | Page::Safety | Page::Launch) {
        let text = match v.page {
            Page::Mixer if v.mixer_loading => tr("Загружаем сессии…"),
            Page::Mixer if v.mixer_unavailable => tr("Не удалось прочитать аудиосессии."),
            Page::Mixer if v.mixer_output.is_none() => tr("Выходные устройства не найдены"),
            Page::Mixer => tr("Запустите звук в нужном приложении."),
            Page::Apps => tr("Выбери приложения кнопкой внизу."),
            Page::PickApps | Page::TriggerPicker if !v.query.is_empty() => tr("Ничего не найдено"),
            Page::PickApps | Page::TriggerPicker => tr("Запусти приложение или выбери его EXE."),
            Page::Output | Page::Fallback | Page::MixerOutputs => tr("Других доступных выходов нет."),
            _ => "",
        };
        g.text(text,rect(LEFT,v.row_y(0)+4.,WIDTH,28.),12.,false,1,p.secondary_ink);
    }
    let control = |action: Action, r: RectF, text: &str, primary: bool| {
        let fill = if hover == Some(action) {p.hover} else if primary {p.primary} else {p.elevated};
        let ink = if primary && hover != Some(action) { WHITE } else { p.ink };
        button(g,p,r,8.,fill,focus == Some(action));
        g.text(text,rect(r.X+3.,r.Y,r.Width-6.,r.Height),11.5,true,1,ink);
    };
    for (action, r) in controls(v) {
        match action {
            Action::Profiles => control(action,r,tr("Профили"),false),
            Action::Mixer => control(action,r,tr("Микшер"),false),
            Action::MixerOutputs => {
                button(g,p,r,8.,if hover == Some(action) {p.hover} else {p.elevated},focus == Some(action));
                g.text(tr("Аудиовыход"),rect(r.X+8.,r.Y,70.,r.Height),10.5,false,0,p.secondary_ink);
                g.text(v.mixer_output.unwrap_or("—"),rect(r.X+81.,r.Y,r.Width-104.,r.Height),11.5,true,0,p.ink);
                g.text("›",rect(r.X+r.Width-20.,r.Y,15.,r.Height),14.,true,1,p.accent);
            }
            Action::MixerOutputAutomatic => control(action,r,tr("Следовать за выходом Windows"),true),
            Action::ProfileCapture => {
                button(g,p,r,10.,if hover == Some(action) {p.hover} else {p.elevated},focus == Some(action));
                g.text("+",rect(r.X,r.Y+8.,r.Width,29.),22.,true,1,p.accent);
                g.text(tr("Новый профиль"),rect(r.X+5.,r.Y+38.,r.Width-10.,25.),11.5,true,1,p.ink);
            }
            Action::ProfileImportApps => control(action,r,tr("+ Добавить приложения"),true),
            Action::ProfileCopyMixer => control(action,rect(r.X,r.Y,r.Width-23.,r.Height),tr("Из микшера"),false),
            Action::ProfilePreview => control(action,r,tr("Применить сейчас"),false),
            Action::ProfileTriggerClear => control(action,r,tr("Только вручную"),false),
            Action::ProfileBrowseExe => control(action,r,tr("Выбрать EXE"),v.page == Page::TriggerPicker),
            Action::ProfileAppSource(popular) => {
                control(action,r,if popular {tr("Популярные")} else {tr("Запущенные")},v.show_popular == popular);
            }
            Action::ProfileBack if r.Y > HEADER_Y => control(action,r,tr("Готово"),true),
            Action::ProfileRefreshApps => control(action,r,"↻",false),

            Action::ProfileMuteFallback => control(action,r,tr("Оставить без звука"),true),
            Action::ProfileSave => control(action,r,if v.editing_existing {tr("Сохранить")} else {tr("Создать профиль")},true),
            Action::ProfileDeactivate => control(action,r,tr("Выключить"),false),
            Action::ProfileDelete => control(action,r,if v.confirm_delete {tr("Подтвердить")} else {tr("Удалить")},false),

            Action::ProfileApply(i) => {
                let profile = &v.list[i];
                let active = v.selected == Some(i);
                button(g,p,r,10.,if active {p.active_fill} else if hover == Some(action) {p.hover} else {p.card_fill},
                    focus == Some(action));
                g.text(&profile.name,rect(r.X+9.,r.Y+8.,r.Width-44.,19.),12.,true,0,p.ink);
                g.text(&profile.output.name,rect(r.X+9.,r.Y+29.,r.Width-18.,16.),10.,false,0,p.secondary_ink);
                let level = if profile.volume.muted {tr("Без звука").into()} else {
                    format!("{}%", profile.volume.level.min(profile.primary_cap.into()))
                };
                g.text(&level,rect(r.X+9.,r.Y+51.,r.Width-18.,17.),11.,true,0,if active {p.accent} else {p.ink});
                if active {
                    g.circle(rect(r.X+r.Width-16.,r.Y+57.,6.,6.),p.accent);
                }
            }
            Action::ProfileOutputPick(i) | Action::MixerOutputPick(i) => {
                let selected = v.page == Page::MixerOutputs && v.mixer_selected
                    .map_or(v.outputs[i].selected,|id| v.outputs[i].id.eq_ignore_ascii_case(id));
                let fill = if hover == Some(action) {p.hover} else if selected {p.selected} else {p.card_fill};
                button(g,p,r,7.,fill,focus == Some(action));
                g.text(&v.outputs[i].name,rect(r.X+8.,r.Y,r.Width-16.,r.Height),11.5,true,0,p.ink);
            }
            Action::ProfileAppPick(i) => {
                let choice = &v.choices[i];
                let key = match choice {
                    Choice::Running(app) => Some(app.key.as_str()),
                    Choice::Popular(i) => v.resolved_popular.get(i).map(String::as_str),
                };
                let checked = key.is_some_and(|key| v.draft.is_some_and(|draft| {
                    if v.page == Page::TriggerPicker {
                        draft.automation.trigger.as_ref().is_some_and(|app| crate::app_picker::same_application(&app.key,key))
                    } else { draft.apps.iter().any(|app| crate::app_picker::same_application(&app.key,key)) }
                }));
                button(g,p,r,7.,if hover == Some(action) {p.hover} else if checked {p.active_fill} else {p.card_fill},
                    focus == Some(action));
                let box_rect = rect(r.X+r.Width-27.,r.Y+8.,18.,18.);
                g.rounded(box_rect,4.,if checked {p.primary} else {p.surface},if checked {None} else {Some(p.border)});
                g.text(if checked {"✓"} else {"+"},box_rect,13.,true,1,if checked {WHITE} else {p.accent});
                g.text(choice.name(),rect(r.X+9.,r.Y,r.Width-48.,r.Height),11.5,true,0,p.ink);
            }
            Action::AppVolume(i) => {
                let app = if v.page == Page::Apps { &v.draft.unwrap().apps[i] } else { &v.apps[i] };
                let y = r.Y-21.;
                let status = if app.volume.muted {tr("Без звука")}
                    else if v.page == Page::Mixer && v.playing.get(i) == Some(&true) {tr("Поток запущен")}
                    else {""};
                let title_width = if status.is_empty() {if v.page == Page::Apps {228.} else {252.}} else {174.};
                g.text(app.display_name(),rect(LEFT,y,title_width,19.),11.5,true,0,p.ink);
                if !status.is_empty() {
                    let status_r = if v.page == Page::Apps {rect(190.,y,51.,19.)} else {rect(203.,y,107.,19.)};
                    g.text(status,status_r,9.5,false,2,p.secondary_ink);
                }
                slider(g,p,r,app.volume.level,app.volume.muted,focus == Some(action));
            }
            Action::AppMute(i) => {
                let app = if v.page == Page::Apps { &v.draft.unwrap().apps[i] } else { &v.apps[i] };
                mute(g,p,r,app.volume.muted,hover == Some(action),focus == Some(action));
            }
            Action::AppForget(_) => control(action,r,"×",false),
            _ => (),
        }
    }
    if let Some(profile) = v.draft {
        draw_settings(g,p,v,profile,hover,focus);
    }
    for (action,r) in controls(v) {
        match action {
            Action::ProfileEdit(_) => control(action,r,"✎",false),
            Action::Help(_) => draw_help(g,p,r,hover == Some(action),focus == Some(action)),
            _ => (),
        }
    }
    if v.count() > v.rows() {
        let rows = if v.page == Page::List {v.rows()/2} else {v.rows()};
        let track = rows as f32 * v.row_step() - 6.;
        let thumb = (track * v.rows() as f32/v.count() as f32).max(11.);
        let y = v.row_y(0) + (track-thumb)*v.start as f32/(v.count()-v.rows()) as f32;
        g.rounded(rect(318.,y,2.,thumb),1.,p.secondary_ink,None);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Help {
    Name, Output, Apps, Fallback, Launch, Master, Mute, Cap, FallbackLevel, FallbackCap,
    Reconnect, Shortcut, Trigger, RequireOutput, Priority, AutoReturn, Copy, Search, AppRow(usize),
    Theme, Startup, Language, GlobalShortcut,
}

impl Help {
    pub fn text(self) -> &'static str {
        match self {
            Self::Theme => t("Выбери тему SoundSnap или используй цветовой режим Windows.", "Choose a SoundSnap theme or follow the Windows color mode."),
            Self::Startup => t("Запускать SoundSnap в трее при входе в Windows.", "Start SoundSnap in the tray when you sign in to Windows."),
            Self::Language => t("Язык интерфейса SoundSnap. По умолчанию используется язык Windows.", "Choose the SoundSnap language. By default it follows Windows."),
            Self::GlobalShortcut => t("Переключать основные устройства вывода из любого приложения.", "Switch between your main output devices from any application."),
            Self::Name => t("Имя на плитке и в уведомлениях.", "Name shown on the tile and in notifications."),
            Self::Output => t("Устройство, на которое профиль переключает звук.", "The device this profile switches audio to."),
            Self::Apps => t("Громкость и отключение звука для отдельных приложений.", "Set volume or mute individual apps."),
            Self::Fallback => t("При отключении основного устройства перевести звук сюда или заглушить его.", "When the primary device disconnects, switch here or mute the sound."),
            Self::Launch => t("Горячая клавиша и условия автоматического включения профиля.", "Set a hotkey or automate profile activation."),
            Self::Master => t("Громкость, которую профиль выставит на основном выходе.", "Volume set on the primary output when applying this profile."),
            Self::Mute => t("Включать профиль с отключённым звуком на основном выходе.", "Mute the primary output when applying this profile."),
            Self::Cap => t("Предел громкости основного выхода, пока активен профиль.", "Maximum primary output volume while the profile is active."),
            Self::FallbackLevel => t("Громкость при переключении на запасное устройство.", "Volume set when switching to the backup output."),
            Self::FallbackCap => t("Предел громкости запасного выхода, пока активен профиль.", "Maximum backup output volume while the profile is active."),
            Self::Reconnect => t("При подключении основного устройства снова направить звук на него.", "Switch back to the primary output when it reconnects."),
            Self::Shortcut => t("Включить этот профиль сочетанием клавиш из любого приложения.", "Apply this profile with a shortcut from any application."),
            Self::Trigger => t("Включать профиль, пока выбрано и запущено указанное приложение.", "Apply the profile automatically while the selected app is running."),
            Self::RequireOutput => t("Автоматически включать профиль только при подключённом основном выходе.", "Activate automatically only when the primary output is connected."),
            Self::Priority => t("Если подходят несколько профилей, включится тот, у которого число выше.", "When profiles overlap, the higher number wins."),
            Self::AutoReturn => t("После закрытия приложения вернуть прежний звук, если его не переключили вручную.", "Restore previous audio after the app closes unless you switched it manually."),
            Self::Copy => t("Скопировать текущие уровни и отключение звука из микшера.", "Copy the current mixer levels and mute states."),
            Self::Search => t("Ищи по названию приложения или имени EXE.", "Search by app name or executable filename."),
            Self::AppRow(_) => t("Ползунок задаёт громкость, динамик отключает звук, × убирает приложение из профиля.", "Set app volume with the slider, mute with the speaker, or remove it with ×."),
        }
    }
}

pub fn draw_help(g: &Painter, p: &Palette, r: RectF, hover: bool, focus: bool) {
    if hover || focus { button(g,p,r,6.,p.hover,focus); }
    g.rounded(rect(r.X+(r.Width-14.)/2.,r.Y+(r.Height-14.)/2.,14.,14.),7.,0,Some(p.accent));
    g.text("!",r,10.,true,1,p.accent);
}

pub fn draw_tooltip(g: &Painter, p: &Palette, help: Help, anchor: RectF, panel_height: f32) {
    const HEIGHT: f32 = 58.;
    const GAP: f32 = 6.;
    let below = anchor.Y + anchor.Height + GAP;
    let y = if below + HEIGHT <= panel_height - LEFT { below } else { anchor.Y - GAP - HEIGHT };
    let y = y.clamp(LEFT, panel_height - LEFT - HEIGHT);
    g.use_panel_typography();
    g.rounded(rect(LEFT,y,WIDTH,HEIGHT),9.,p.surface,Some(p.focus));
    g.wrapped_text(help.text(),rect(LEFT+10.,y+4.,WIDTH-20.,HEIGHT-8.),12.,p.ink);
}

fn t(ru: &'static str,en: &'static str) -> &'static str {
    if crate::locale::current() == crate::locale::Language::Russian {ru} else {en}
}

fn draw_settings(g: &Painter, p: &Palette, v: &View, profile: &Profile, hover: Option<Action>, focus: Option<Action>) {
    let yes = |enabled| if enabled {tr("Да")} else {tr("Нет")};
    let row = |action, y, label, value: &str| {
        pair(g,p,field(y),label,value,hover == Some(action),focus == Some(action));
    };
    let volume = |action, y, label: &str, level, muted| {
        g.text(label,rect(LEFT,y-25.,254.,18.),11.,false,0,p.secondary_ink);
        slider(g,p,bar(y),level,muted,focus == Some(action));
    };
    let backup = profile.fallback.as_ref().map_or(tr("Без звука"),|out| out.name.as_str());
    let trigger = profile.automation.trigger.as_ref().map_or(tr("Только вручную"),|app| app.name.as_str());
    match v.page {
        Page::Edit => {
            let name = if v.typing {format!("{}│",profile.name)} else {profile.name.clone()};
            row(Action::ProfileName,78.,tr("Название"),&name);
            let output = format!("{} · {}%",profile.output.name,profile.volume.level.min(profile.primary_cap.into()));
            let apps = format!("{}: {}",tr("Выбрано"),profile.apps.len());
            let sections = [
                (Action::ProfilePrimary, "◖", tr("Основной выход"), output.as_str()),
                (Action::ProfileApps, "≡", tr("Приложения"), apps.as_str()),
                (Action::ProfileSafety, "↻", tr("При отключении"), backup),
                (Action::ProfileLaunch, "↗", tr("Когда включать"), trigger),
            ];
            for (i,(action,icon,label,detail)) in sections.into_iter().enumerate() {
                let r = tile(i,120.,82.);
                let fill = if hover == Some(action) {p.hover} else {p.card_fill};
                button(g,p,r,10.,fill,focus == Some(action));
                g.text(icon,rect(r.X+10.,r.Y+5.,24.,22.),17.,true,0,p.accent);
                g.text(label,rect(r.X+10.,r.Y+30.,r.Width-18.,18.),11.5,true,0,p.ink);
                g.text(detail,rect(r.X+10.,r.Y+51.,r.Width-18.,19.),10.,false,0,p.secondary_ink);
            }
        }
        Page::Primary => {
            row(Action::ProfileOutput,80.,tr("Устройство вывода"),&profile.output.name);
            volume(Action::ProfileMaster,144.,tr("Общая громкость"),profile.volume.level,profile.volume.muted);
            row(Action::ProfileMasterMute,166.,tr("Без звука"),yes(profile.volume.muted));
            volume(Action::ProfilePrimaryCap,229.,tr("Предел громкости"),profile.primary_cap.into(),false);
        }
        Page::Safety => {
            row(Action::ProfileFallback,80.,tr("Запасной выход"),backup);
            let y = if profile.fallback.is_some() {
                volume(Action::ProfileFallbackLevel,144.,tr("Громкость запасного выхода"),profile.fallback_level.into(),false);
                volume(Action::ProfileFallbackCap,198.,tr("Предел громкости"),profile.fallback_cap.into(),false);
                232.
            } else {124.};
            row(Action::ProfileReconnect,y,tr("Вернуть звук на основной"),yes(profile.reconnect));
        }
        Page::Launch => {
            let shortcut = if v.capturing {tr("Нажмите клавиши…").into()}
                else {profile.shortcut.map_or_else(|| tr("Не задана").into(),|key| key.labels().join(" + "))};
            row(Action::ProfileShortcut,79.,tr("Горячая клавиша"),&shortcut);
            row(Action::ProfileTrigger,120.,tr("При запуске приложения"),trigger);
            row(Action::ProfileRequireOutput,161.,tr("Требовать основной выход"),yes(profile.automation.require_output));
            g.rounded(field(202.),7.,p.elevated,None);
            g.text(tr("Приоритет"),rect(23.,202.,153.,31.),10.5,false,0,p.secondary_ink);
            g.text(&format!("{}",profile.automation.priority),rect(239.,202.,23.,31.),11.5,true,1,p.ink);
            for (step,x,label) in [(-1,215.,"−"),(1,263.,"+")] {
                let action = Action::ProfilePriority(step);
                let r = rect(x,204.,23.,27.);
                button(g,p,r,6.,if hover == Some(action) {p.hover} else {p.card_fill},focus == Some(action));
                g.text(label,r,13.,true,1,p.accent);
            }
            row(Action::ProfileAutoReturn,243.,tr("Вернуть прежний звук"),yes(profile.automation.restore));
        }
        Page::PickApps | Page::TriggerPicker => {
            let action = Action::ProfileQuery;
            let r = field(119.);
            button(g,p,r,7.,if hover == Some(action) {p.hover} else {p.elevated},focus == Some(action) || v.typing);
            let text = if v.typing {format!("{}│",v.query)}
                else if v.query.is_empty() {tr("Поиск приложений…").into()} else {v.query.into()};
            g.text(&text,rect(r.X+9.,r.Y,r.Width-38.,r.Height),11.5,false,0,
                if !v.typing && v.query.is_empty() {p.secondary_ink} else {p.ink});
        }
        _ => (),
    }
}

fn pair(g: &Painter, p: &Palette, r: RectF, label: &str, value: &str, hover: bool, focus: bool) {
    button(g,p,r,7.,if hover {p.hover} else {p.elevated},focus);
    g.text(label,rect(r.X+7.,r.Y,141.,r.Height),10.5,false,0,p.secondary_ink);
    g.text(value,rect(r.X+150.,r.Y,r.Width-180.,r.Height),11.5,true,2,p.ink);
}

fn slider(g: &Painter, p: &Palette, r: RectF, value: i32, muted: bool, focus: bool) {
    let y = r.Y + r.Height/2.;
    let fill = SLIDER_WIDTH * value.clamp(0,100) as f32 / 100.;
    let ink = if muted {p.secondary_ink} else {p.accent};
    g.rounded(rect(SLIDER_LEFT,y-2.,SLIDER_WIDTH,4.),2.,p.volume_track,None);
    if fill > 0. { g.rounded(rect(SLIDER_LEFT,y-2.,fill,4.),fill.min(4.)/2.,ink,None); }
    g.circle(rect(SLIDER_LEFT+fill-4.,y-4.,8.,8.),ink);
    g.text(&format!("{value}%"),rect(234.,y-10.,30.,20.),10.5,true,2,p.ink);
    if focus { g.rounded(r,5.,0,Some(p.focus)); }
}

fn mute(g: &Painter, p: &Palette, r: RectF, muted: bool, hover: bool, focus: bool) {
    let fill = if hover {p.hover} else if muted {p.active_fill} else {p.card_fill};
    button(g,p,r,7.,fill,focus);
    g.speaker(r.X + (r.Width-16.)/2., r.Y+5.,0.66,if muted {p.accent} else {p.ink},muted,false);
}
