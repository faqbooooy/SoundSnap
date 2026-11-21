use alloc::{format, vec::Vec};
use windows_sys::Win32::Graphics::GdiPlus::RectF;
use crate::{audio::Device, gfx::{point, rect, Painter, WHITE}, locale::tr, palette::Palette,
    profile_app::AppChoice, profiles::{AppLevel, Profile, LIMIT}, ui::{button, close, Action}};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page { List, Mixer, MixerOutputs, Edit, Apps, PickApps, Output, Fallback }

pub struct View<'a> {
    pub page: Page,
    pub list: &'a [Profile],
    pub selected: Option<usize>,
    pub draft: Option<&'a Profile>,
    pub editing_existing: bool,
    pub confirm_delete: bool,
    pub apps: &'a [AppLevel],
    pub playing: &'a [bool],
    pub choices: &'a [AppChoice],
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
            Page::List => self.list.len(),
            Page::Mixer => self.apps.len(),
            Page::Apps => self.draft.map_or(0, |profile| profile.apps.len()),
            Page::PickApps => self.choices.len(),
            Page::Output | Page::Fallback | Page::MixerOutputs => self.outputs.len(),
            Page::Edit => 0,
        }
    }
    pub fn height(&self) -> i32 {
        match self.page {
            Page::Edit => 405,
            _ => (144. + self.mixer_extra() + self.count().clamp(1,self.rows()) as f32 * self.row_step()) as i32,
        }
    }
    pub fn rows(&self) -> usize {
        (((self.available_height - 164. - self.mixer_extra()) / self.row_step()) as usize).clamp(1,MAX_ROWS)
    }
    fn row_step(&self) -> f32 {
        if matches!(self.page, Page::Mixer | Page::Apps) { 48. } else { 39. }
    }
    fn mixer_extra(&self) -> f32 { if self.page == Page::Mixer { 28. } else { 0. } }
    fn row_y(&self, row: usize) -> f32 { 87. + self.mixer_extra() + row as f32 * self.row_step() }
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
            if v.page == Page::List && v.list.len() < LIMIT {
                controls.push((Action::ProfileCapture,command(v.footer())));
            }
            if v.page == Page::List && v.selected.is_some() {
                controls.push((Action::ProfileDeactivate,rect(216.,58.,94.,22.)));
            }
            if v.page == Page::Mixer { controls.push((Action::MixerOutputs,rect(LEFT,58.,WIDTH,29.))); }
        }
        Page::Edit => {
            controls.extend([
                (Action::ProfileName,rect(LEFT,78.,WIDTH,25.)),
                (Action::ProfileOutput,rect(LEFT,106.,WIDTH,29.)),
                (Action::ProfileMaster,bar(158.)),
                (Action::ProfileMasterMute,rect(276.,145.,34.,26.)),
                (Action::ProfileApps,field(177.)),
                (Action::ProfileFallback,field(215.)),
                (Action::ProfileReconnect,field(282.)),
                (Action::ProfileShortcut,field(318.)),
                (Action::ProfileSave,rect(if v.editing_existing {150.} else {16.},v.footer(),
                    if v.editing_existing {160.} else {WIDTH},32.)),
            ]);
            if v.draft.is_some_and(|profile| profile.fallback.is_some()) {
                controls.push((Action::ProfileFallbackLevel,bar(263.)));
            }
            if v.editing_existing { controls.push((Action::ProfileDelete,rect(16.,v.footer(),128.,32.))); }
        }
        Page::Apps => controls.push((Action::ProfileImportApps,command(v.footer()))),
        Page::PickApps => {
            controls.push((Action::ProfileRefreshApps,rect(LEFT,v.footer(),142.,32.)));
            controls.push((Action::ProfileApps,rect(166.,v.footer(),144.,32.)));
        }
        Page::Fallback => controls.push((Action::ProfileMuteFallback,command(v.footer()))),
        Page::MixerOutputs => controls.push((Action::MixerOutputAutomatic,command(v.footer()))),
        Page::Output => (),
    }
    for (row, index) in (v.start..(v.start + v.rows()).min(v.count())).enumerate() {
        let y = v.row_y(row);
        match v.page {
            Page::List => controls.extend([(Action::ProfileApply(index),rect(LEFT,y,246.,33.)),
                (Action::ProfileEdit(index),rect(266.,y,44.,33.))]),
            Page::Mixer | Page::Apps => {
                controls.push((Action::AppVolume(index),bar(y+32.)));
                controls.push((Action::AppMute(index),rect(268.,y+11.,v_mute_width(v.page),27.)));
                if v.page == Page::Apps { controls.push((Action::AppForget(index),rect(293.,y+11.,17.,27.))); }
            }
            Page::Output | Page::Fallback => controls.push((Action::ProfileOutputPick(index),field(y))),
            Page::MixerOutputs => controls.push((Action::MixerOutputPick(index),field(y))),
            Page::PickApps if v.choices[index].persistent => controls.push((Action::ProfileAppPick(index),rect(LEFT,y,WIDTH,34.))),
            Page::PickApps => (),
            Page::Edit => (),
        }
    }
    controls
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
        Page::Output => tr("Основной выход"), Page::Fallback => tr("Запасной выход"),
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

    let hint = if !v.error.is_empty() { v.error } else { match v.page {
        Page::List => if v.list.is_empty() { tr("Сохрани настройки для игры, работы или музыки.") }
            else if v.selected.is_some() { tr("Профиль включён") } else { tr("Выбери профиль, чтобы включить его.") },
        Page::Mixer => "",
        Page::MixerOutputs => tr("Выбери устройство, сессии которого хочешь видеть."),
        Page::Apps => tr("Уровни применяются при включении профиля."),
        Page::PickApps => tr("Отметь приложения, которыми управляет профиль."),
        Page::Output => tr("Куда выводить звук при включении профиля"),
        Page::Fallback => tr("Куда вывести звук, если основной выход пропадёт"),
        Page::Edit => "",
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
        g.text(hint,rect(LEFT,59.,if v.page == Page::List && v.selected.is_some() {196.} else {WIDTH},18.),
            10.5,false,0,p.secondary_ink);
    }
    if v.page != Page::Edit && v.count() == 0 {
        let text = match v.page {
            Page::Mixer if v.mixer_loading => tr("Загружаем сессии…"),
            Page::Mixer if v.mixer_unavailable => tr("Не удалось прочитать аудиосессии."),
            Page::Mixer if v.mixer_output.is_none() => tr("Выходные устройства не найдены"),
            Page::Mixer => tr("Запустите звук в нужном приложении."),
            Page::Apps => tr("Выбери приложения кнопкой внизу."),
            Page::PickApps => tr("Запусти звук в приложении и обнови список."),
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
            Action::ProfileCapture => control(action,r,tr("+ Создать профиль"),true),
            Action::ProfileImportApps => control(action,r,tr("Выбрать приложения"),true),
            Action::ProfileRefreshApps => control(action,r,tr("Обновить список"),false),
            Action::ProfileApps if v.page == Page::PickApps => control(action,r,tr("Готово"),true),
            Action::ProfileMuteFallback => control(action,r,tr("Оставить без звука"),true),
            Action::ProfileSave => control(action,r,if v.editing_existing {tr("Сохранить")} else {tr("Создать профиль")},true),
            Action::ProfileDeactivate => control(action,r,tr("Выключить"),false),
            Action::ProfileDelete => control(action,r,if v.confirm_delete {tr("Подтвердить")} else {tr("Удалить")},false),
            Action::ProfileEdit(_) => control(action,r,"✎",false),
            Action::ProfileApply(i) => {
                let profile = &v.list[i];
                let active = v.selected == Some(i);
                button(g,p,r,7.,if active {p.active_fill} else if hover == Some(action) {p.hover} else {p.card_fill},
                    focus == Some(action));
                g.text(&profile.name,rect(r.X+8.,r.Y+1.,228.,17.),12.,true,0,p.ink);
                g.text(&profile.output.name,rect(r.X+8.,r.Y+17.,228.,14.),9.5,false,0,p.secondary_ink);
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
                let checked = v.draft.is_some_and(|draft| draft.apps.iter().any(|app| app.key == choice.app.key));
                button(g,p,r,7.,if checked {p.active_fill} else if hover == Some(action) {p.hover} else {p.card_fill},
                    focus == Some(action));
                let box_rect = rect(r.X+8.,r.Y+9.,16.,16.);
                g.rounded(box_rect,4.,if checked {p.primary} else {p.surface},if checked {None} else {Some(p.border)});
                if checked {
                    g.stroke(&[point(box_rect.X+3.,box_rect.Y+8.),point(box_rect.X+6.,box_rect.Y+11.),
                        point(box_rect.X+12.,box_rect.Y+5.)],1.4,WHITE);
                }
                let detail = match choice.sessions {
                    0 => tr("Закрыто").into(),
                    1 => "".into(),
                    count => format!("{}: {count}",tr("Сессий")),
                };
                g.text(choice.app.display_name(),rect(r.X+34.,r.Y,174.,r.Height),11.5,true,0,p.ink);
                g.text(&detail,rect(r.X+211.,r.Y,74.,r.Height),10.,false,2,p.secondary_ink);
            }
            Action::AppVolume(i) => {
                let app = if v.page == Page::Apps { &v.draft.unwrap().apps[i] } else { &v.apps[i] };
                let y = r.Y-21.;
                let status = if app.volume.muted {tr("Без звука")}
                    else if v.page == Page::Mixer && v.playing.get(i) == Some(&true) {tr("Поток запущен")}
                    else {""};
                let title_width = if status.is_empty() {252.} else {186.};
                g.text(app.display_name(),rect(LEFT,y,title_width,19.),11.5,true,0,p.ink);
                if !status.is_empty() {
                    g.text(status,rect(203.,y,107.,19.),9.5,false,2,p.secondary_ink);
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
    if v.page == Page::PickApps {
        for (row,choice) in v.choices.iter().skip(v.start).take(v.rows()).enumerate() {
            if choice.persistent { continue; }
            let r = rect(LEFT,v.row_y(row),WIDTH,34.);
            button(g,p,r,7.,p.card_fill,false);
            g.text(choice.app.display_name(),rect(r.X+9.,r.Y,176.,r.Height),11.5,true,0,p.secondary_ink);
            g.text(tr("Только в микшере"),rect(r.X+188.,r.Y,96.,r.Height),9.5,false,2,p.secondary_ink);
        }
    }
    if v.page == Page::Edit {
        if let Some(profile) = v.draft {
            let name = if v.typing { format!("{}│",profile.name) } else { profile.name.clone() };
            pair(g,p,rect(LEFT,78.,WIDTH,25.),tr("Название"),&name,focus == Some(Action::ProfileName) || v.typing);
            pair(g,p,rect(LEFT,106.,WIDTH,29.),tr("Устройство вывода"),&profile.output.name,focus == Some(Action::ProfileOutput));
            g.text(tr("Общая громкость"),rect(LEFT,137.,188.,14.),9.5,false,0,p.secondary_ink);
            slider(g,p,bar(158.),profile.volume.level,profile.volume.muted,focus == Some(Action::ProfileMaster));
            mute(g,p,rect(276.,145.,34.,26.),profile.volume.muted,hover == Some(Action::ProfileMasterMute),
                focus == Some(Action::ProfileMasterMute));
            pair(g,p,field(177.),tr("Громкость приложений"),&format!("{}: {} ›",tr("Выбрано"),profile.apps.len()),focus == Some(Action::ProfileApps));
            pair(g,p,field(215.),tr("Если выход отключится"),profile.fallback.as_ref().map_or(tr("Без звука"),|output| &output.name),
                focus == Some(Action::ProfileFallback));
            if profile.fallback.is_some() {
                g.text(tr("Громкость запасного выхода"),rect(LEFT,245.,250.,14.),9.5,false,0,p.secondary_ink);
                slider(g,p,bar(263.),profile.fallback_level.into(),false,focus == Some(Action::ProfileFallbackLevel));
            }
            pair(g,p,field(282.),tr("Вернуть звук на основной"),if profile.reconnect {tr("Да")} else {tr("Нет")},
                focus == Some(Action::ProfileReconnect));
            let shortcut = if v.capturing { tr("Нажмите клавиши…").into() }
                else {profile.shortcut.map_or_else(|| tr("Не задана").into(),|shortcut| shortcut.labels().join(" + ")) };
            pair(g,p,field(318.),tr("Горячая клавиша"),&shortcut,focus == Some(Action::ProfileShortcut) || v.capturing);
        }
    }
    if v.count() > v.rows() {
        let track = v.rows() as f32 * v.row_step() - 6.;
        let thumb = (track * v.rows() as f32/v.count() as f32).max(11.);
        let y = v.row_y(0) + (track-thumb)*v.start as f32/(v.count()-v.rows()) as f32;
        g.rounded(rect(318.,y,2.,thumb),1.,p.secondary_ink,None);
    }
}

fn pair(g: &Painter, p: &Palette, r: RectF, label: &str, value: &str, focus: bool) {
    button(g,p,r,7.,p.elevated,focus);
    g.text(label,rect(r.X+7.,r.Y,141.,r.Height),10.5,false,0,p.secondary_ink);
    g.text(value,rect(r.X+150.,r.Y,r.Width-158.,r.Height),11.5,true,2,p.ink);
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
