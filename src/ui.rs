use crate::{audio::{Device, Volume}, gfx::{point, rect, Color, Painter, WHITE}, palette::Palette,
    settings::{Mode, Shortcut}};
use alloc::{format, vec::Vec};
use windows_sys::Win32::Graphics::GdiPlus::RectF;

pub const PANEL_WIDTH: i32 = 326;
const WIDTH: f32 = PANEL_WIDTH as f32;

const ROW_SPACING: f32 = 41.;
const FOOTER_HEIGHT: f32 = 58.;
const TRACK_LEFT: f32 = 51.;
const TRACK_WIDTH: f32 = 164.;
const CLOSE_X: f32 = WIDTH - 40.;
const CLOSE_Y: f32 = 17.;
pub const SETTINGS_HEIGHT: i32 = 288;
pub const TOAST_WIDTH: i32 = 304;
pub const TOAST_HEIGHT: i32 = 68;
pub const MENU_WIDTH: i32 = 157;
pub const MENU_HEIGHT: i32 = 92;
pub const TRAY_VOLUME_WIDTH: i32 = 96;
pub const TRAY_VOLUME_HEIGHT: i32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Device(usize), QuickDevice(usize), EditDevices, OtherDevices, Volume, Mute, Close, Settings, Back,
    Shortcut, Theme(u8), Startup, Cancel, Save, Open, Exit,
}

pub struct Panel<'a> {
    pub devices: &'a [Device],
    pub excluded: &'a [bool],
    pub start: usize,
    pub rows: usize,
    pub error: bool,
    pub volume: Option<Volume>,
    pub editing: bool,
    pub expanded: bool,
    pub wiggle: f32,
}

impl Panel<'_> {
    fn shows_rows(&self) -> bool { self.editing || self.expanded }
    fn row_top(&self) -> f32 { if self.editing { 95. } else { 233. } }
    fn footer_top(&self) -> f32 {
        if self.shows_rows() {
            self.row_top() + self.rows.min(self.devices.len()) as f32 * ROW_SPACING + 12.
        } else { 232. }
    }
    pub fn height(&self) -> i32 { (self.footer_top() + FOOTER_HEIGHT) as i32 }
    fn row_rect(&self, row: usize) -> RectF {
        rect(16., self.row_top() + row as f32 * ROW_SPACING, WIDTH - 32., 35.)
    }
    fn tile_indices(&self) -> [Option<usize>; 2] {
        let active = self.devices.iter().position(|device| device.selected);
        let eligible = |i: usize| !self.excluded.get(i).copied().unwrap_or(false);
        let first = active.or_else(|| (0..self.devices.len()).find(|&i| eligible(i)))
            .or_else(|| (!self.devices.is_empty()).then_some(0));
        // The tray cycles real outputs in their enumeration order. Demo rows are
        // considered for preview only when there is no second real output.
        let next = |demo: bool| {
            let available = |i: usize| {
                Some(i) != first && eligible(i) && self.devices[i].id.starts_with("demo:") == demo
            };
            if let Some(index) = first.filter(|&i| eligible(i)) {
                ((index + 1)..self.devices.len()).find(|&i| available(i))
                    .or_else(|| (0..self.devices.len()).find(|&i| available(i)))
            } else {
                (0..self.devices.len()).find(|&i| available(i))
            }
        };
        [first, next(false).or_else(|| next(true))]
    }
}

pub fn rows_for_height(height: f32) -> usize {
    (((height - 245. - FOOTER_HEIGHT) / ROW_SPACING) as usize).clamp(1, 4)
}
pub fn volume_level(x: f32) -> i32 {
    ((x - TRACK_LEFT) * 100. / TRACK_WIDTH + 0.5).clamp(0., 100.) as i32
}
fn tile_rect(slot: usize) -> RectF { rect(16. + slot as f32 * 151.,73.,143.,113.) }
fn list_toggle() -> RectF { rect(16.,193.,147.,33.) }
fn edit_link() -> RectF { rect(167.,193.,143.,33.) }
fn edit_icon() -> RectF { rect(213.,17.,30.,28.) }
fn done_button() -> RectF { rect(225.,17.,56.,28.) }
fn gear_button() -> RectF { rect(249.,17.,30.,28.) }
fn slider_rect(bottom: f32) -> RectF { rect(TRACK_LEFT-7.,bottom+14.,TRACK_WIDTH+14.,30.) }
fn mute_rect(bottom: f32) -> RectF { rect(WIDTH-50.,bottom+12.,34.,34.) }

pub fn panel_controls(panel: &Panel) -> Vec<(Action, RectF)> {
    let bottom = panel.footer_top();
    let mut controls = Vec::new();
    if panel.editing {
        controls.push((Action::EditDevices, done_button()));
    } else {
        controls.push((Action::Settings, gear_button()));
        if !panel.devices.is_empty() {
            controls.push((Action::EditDevices, edit_icon()));
            for (slot, index) in panel.tile_indices().into_iter().enumerate() {
                if let Some(index) = index { controls.push((Action::QuickDevice(index), tile_rect(slot))); }
            }
            controls.push((Action::OtherDevices, list_toggle()));
            controls.push((Action::EditDevices, edit_link()));
        }
    }
    if panel.shows_rows() {
        for (row, index) in (panel.start..(panel.start + panel.rows).min(panel.devices.len())).enumerate() {
            controls.push((Action::Device(index), panel.row_rect(row)));
        }
    }
    if panel.volume.is_some() {
        controls.extend([(Action::Volume, slider_rect(bottom)), (Action::Mute, mute_rect(bottom))]);
    }
    controls.push((Action::Close, rect(CLOSE_X,CLOSE_Y,26.,26.)));
    controls
}

#[derive(Clone, Copy)]
enum OutputIcon { Speaker, Headphones, Monitor, Dac }

impl OutputIcon {
    fn from_name(name: &str) -> Self {
        let name = name.to_lowercase();
        if ["наушник", "headset", "headphone", "earbud", "гарнитур"].iter().any(|word| name.contains(word)) {
            Self::Headphones
        } else if ["монитор", "hdmi", "display", "телевизор"].iter().any(|word| name.contains(word)) {
            Self::Monitor
        } else if ["цап", "dac", "usb audio", "линейный выход"].iter().any(|word| name.contains(word)) {
            Self::Dac
        } else { Self::Speaker }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Speaker => "Колонки", Self::Headphones => "Наушники",
            Self::Monitor => "Монитор", Self::Dac => "Аудиоинтерфейс",
        }
    }

    fn draw(self, g: &Painter, x: f32, y: f32, s: f32, ink: Color) {
        let fill = |u, v, w, h| rect(x+u*s,y+v*s,w*s,h*s);
        let p = |u, v| point(x+u*s,y+v*s);
        match self {
            Self::Speaker => {
                g.rounded(fill(5.,1.,14.,22.),3.*s,0,Some(ink));
                g.circle(fill(10.,5.,4.,4.),ink);
                g.stroke(&[p(9.5,15.),p(10.,13.),p(12.,12.2),p(14.,13.),p(14.5,15.),
                    p(14.,17.),p(12.,17.8),p(10.,17.),p(9.5,15.)],1.35*s,ink);
            }
            Self::Headphones => {
                g.stroke(&[p(3.,16.),p(3.,11.),p(4.,7.5),p(7.,4.5),p(12.,3.),p(17.,4.5),
                    p(20.,7.5),p(21.,11.),p(21.,16.)],1.8*s,ink);
                g.rounded(fill(2.,13.,5.,8.),2.*s,ink,None);
                g.rounded(fill(17.,13.,5.,8.),2.*s,ink,None);
            }
            Self::Monitor => {
                g.rounded(fill(2.,4.,20.,13.),2.*s,0,Some(ink));
                g.stroke(&[p(12.,17.),p(12.,21.)],1.5*s,ink);
                g.stroke(&[p(8.,21.),p(16.,21.)],1.5*s,ink);
            }
            Self::Dac => {
                g.rounded(fill(2.,6.,20.,12.),3.*s,0,Some(ink));
                g.circle(fill(6.,10.,4.,4.),ink);
                g.circle(fill(14.,10.,4.,4.),ink);
            }
        }
    }
}

fn check(g: &Painter, x: f32, y: f32, ink: Color) {
    g.stroke(&[point(x-3.5,y),point(x-0.7,y+2.8),point(x+4.,y-3.)],1.65,ink);
}

pub fn panel(g: &Painter, p: &Palette, model: Panel, hover: Option<Action>, focus: Option<Action>) {
    g.use_panel_typography();
    let bottom = model.footer_top();
    g.gradient(rect(0.5,0.5,WIDTH-1.,model.height() as f32-1.),18.,p.glass_start,p.glass_end,p.glass_edge);
    g.brand_logo(17.,16.,30.);
    g.text("SoundSnap",rect(55.,15.,149.,32.),21.,true,0,p.ink);
    close(g,p,CLOSE_X,CLOSE_Y,hover == Some(Action::Close),focus == Some(Action::Close));
    g.fill(rect(16.,59.,WIDTH-32.,1.),p.line);

    if model.editing {
        let done = done_button();
        button(g,p,done,8.,if hover == Some(Action::EditDevices) {p.hover} else {0},focus == Some(Action::EditDevices));
        g.text("Готово",done,12.,true,1,p.accent);
        g.text("Показывать в быстром выборе",rect(17.,65.,252.,23.),13.,true,0,p.ink);
        let total = model.devices.len();
        let chosen = model.excluded.iter().filter(|excluded| !**excluded).count();
        g.text(&format!("{chosen}/{total}"),rect(268.,65.,41.,23.),10.5,true,2,p.secondary_ink);
    } else {
        let gear = gear_button();
        button(g,p,gear,8.,if hover == Some(Action::Settings) {p.hover} else {0},focus == Some(Action::Settings));
        g.gear(gear.X+6.,gear.Y+5.,0.75,p.ink);
        if model.devices.is_empty() {
            let text = if model.error { "Не удалось получить список устройств" } else { "Выходные устройства не найдены" };
            g.text(text,rect(16.,92.,WIDTH-32.,48.),12.,false,1,p.secondary_ink);
        } else {
            draw_quick_tiles(g,p,&model,hover,focus);
            let list = list_toggle();
            button(g,p,list,7.,if hover == Some(Action::OtherDevices) {p.hover} else {0},focus == Some(Action::OtherDevices));
            let x = list.X+10.;
            let y = list.Y+16.;
            let d = if model.expanded { -1. } else { 1. };
            g.stroke(&[point(x-4.,y-2.*d),point(x,y+2.*d),point(x+4.,y-2.*d)],1.4,p.ink);
            g.text("Все устройства",rect(list.X+22.,list.Y,list.Width-23.,list.Height),12.5,true,0,p.ink);
            let edit = edit_link();
            button(g,p,edit,7.,if hover == Some(Action::EditDevices) {p.hover} else {0},focus == Some(Action::EditDevices));
            g.text("Настроить список",rect(edit.X+4.,edit.Y,edit.Width-8.,edit.Height),12.,true,2,p.accent);
            let pen = edit_icon();
            button(g,p,pen,8.,if hover == Some(Action::EditDevices) {p.hover} else {0},focus == Some(Action::EditDevices));
            let x = pen.X+8.;
            let y = pen.Y+6.;
            g.stroke(&[point(x,y+15.),point(x+1.5,y+10.5),point(x+11.,y+1.),point(x+14.5,y+4.5),
                point(x+5.,y+14.),point(x,y+15.)],1.35,p.ink);
        }
    }
    if model.shows_rows() { draw_output_rows(g,p,&model,hover,focus); }
    draw_panel_volume(g,p,model.volume,bottom,hover,focus);
}

fn draw_quick_tiles(g: &Painter, p: &Palette, model: &Panel, hover: Option<Action>, focus: Option<Action>) {
    for (slot, index) in model.tile_indices().into_iter().enumerate() {
        let r = tile_rect(slot);
        let Some(index) = index else {
            g.rounded(r,12.,p.card_fill,Some(p.card_edge));
            g.text("Нет второго выхода",rect(r.X+10.,r.Y+28.,r.Width-20.,25.),11.5,false,1,p.secondary_ink);
            g.text("Настроить список ↓",rect(r.X+10.,r.Y+54.,r.Width-20.,25.),10.,false,1,p.secondary_ink);
            continue;
        };
        let device = &model.devices[index];
        let action = Action::QuickDevice(index);
        let selected = device.selected;
        let edge = if selected || focus == Some(action) {p.accent} else if hover == Some(action) {p.hover_edge} else {p.card_edge};
        let fill = if selected {p.active_fill} else if hover == Some(action) {p.hover} else {p.card_fill};
        g.rounded(r,12.,fill,Some(edge));
        if selected {
            g.rounded(rect(r.X+1.,r.Y+1.,r.Width-2.,r.Height-2.),11.,0,Some(p.accent));
            let cx = r.X + r.Width-16.;
            let cy = r.Y + 15.;
            g.circle(rect(cx-8.5,cy-8.5,17.,17.),p.primary);
            check(g,cx,cy,WHITE);
        }
        let icon = OutputIcon::from_name(&device.name);
        icon.draw(g,r.X+(r.Width-36.)/2.,r.Y+19.,1.5,p.ink);
        g.text(icon.label(),rect(r.X+6.,r.Y+63.,r.Width-12.,20.),14.,true,1,p.ink);
        g.text(&device.name,rect(r.X+8.,r.Y+84.,r.Width-16.,17.),10.5,false,1,p.secondary_ink);
    }
}

fn draw_output_rows(g: &Painter, p: &Palette, model: &Panel, hover: Option<Action>, focus: Option<Action>) {
    for (row, index) in (model.start..(model.start+model.rows).min(model.devices.len())).enumerate() {
        let device = &model.devices[index];
        let action = Action::Device(index);
        let mut r = model.row_rect(row);
        r.X += if row % 2 == 0 {model.wiggle} else {-model.wiggle};
        let selected = device.selected && !model.editing;
        let edge = if focus == Some(action) || selected {p.accent} else {p.card_edge};
        let fill = if selected {p.active_fill} else if hover == Some(action) {p.hover} else {p.card_fill};
        g.rounded(r,7.,fill,Some(edge));
        if model.editing {
            let cx = r.X+17.;
            let cy = r.Y+r.Height/2.;
            let included = !model.excluded.get(index).copied().unwrap_or(false);
            g.rounded(rect(cx-7.5,cy-7.5,15.,15.),3.5,if included {p.primary} else {p.surface},
                if included {None} else {Some(p.secondary_ink)});
            if included { check(g,cx,cy,WHITE); }
        }
        let x = r.X + if model.editing {36.} else {10.};
        OutputIcon::from_name(&device.name).draw(g,x,r.Y+8.,0.8,p.ink);
        g.text(&device.name,rect(x+30.,r.Y,r.X+r.Width-x-52.,r.Height),12.5,true,0,p.ink);
        if selected { check(g,r.X+r.Width-15.,r.Y+r.Height/2.,p.accent); }
    }
    if model.devices.len() > model.rows {
        let height = model.rows as f32 * ROW_SPACING - 5.;
        let thumb = (height * model.rows as f32 / model.devices.len() as f32).max(10.);
        let y = model.row_top() + (height-thumb)*model.start as f32/(model.devices.len()-model.rows) as f32;
        g.rounded(rect(WIDTH-8.,y,2.,thumb),1.,p.secondary_ink,None);
    }
}

fn draw_panel_volume(g: &Painter, p: &Palette, volume: Option<Volume>, bottom: f32, hover: Option<Action>, focus: Option<Action>) {
    g.fill(rect(16.,bottom,WIDTH-32.,1.),p.line);
    g.speaker(16.,bottom+20.,0.87,p.ink,false,false);
    let y = bottom+29.;
    let accent = if volume.is_some_and(|status| status.muted) {p.secondary_ink} else {p.accent};
    g.rounded(rect(TRACK_LEFT,y-2.5,TRACK_WIDTH,5.),2.5,p.volume_track,None);
    if let Some(status) = volume {
        let fill = TRACK_WIDTH * status.level as f32 / 100.;
        if fill > 0. {
            g.rounded(rect(TRACK_LEFT,y-2.5,fill,5.),fill.min(5.)/2.,accent,None);
        }
        g.rounded(rect(TRACK_LEFT+fill-7.5,y-7.5,15.,15.),7.5,p.thumb,Some(accent));
    }
    if focus == Some(Action::Volume) {
        g.rounded(slider_rect(bottom),8.,0,Some(p.focus));
    }
    let text = volume.map_or_else(|| "—".into(), |status| format!("{}%",status.level));
    g.text(&text,rect(222.,bottom+13.,46.,32.),12.,true,1,p.ink);
    let mute = mute_rect(bottom);
    let muted = volume.is_some_and(|status| status.muted);
    let fill = if hover == Some(Action::Mute) {p.hover}
        else if muted {p.active_fill} else {p.card_fill};
    g.rounded(mute,8.,fill,Some(if focus == Some(Action::Mute) {p.focus} else {p.card_edge}));
    g.speaker(mute.X+7.,mute.Y+7.,0.82,if muted {p.accent} else {p.ink},!muted,false);
}
fn button(g: &Painter, p: &Palette, r: RectF, radius: f32, fill: Color, focus: bool) {
    g.rounded(r,radius,fill,focus.then_some(p.focus));
}
fn close(g: &Painter, p: &Palette, x: f32, y: f32, hover: bool, focus: bool) {
    button(g,p,rect(x,y,26.,26.),7.,if hover { p.hover } else { 0 },focus);
    g.stroke(&[point(x+8.,y+8.),point(x+18.,y+18.)],1.5,p.secondary_ink);
    g.stroke(&[point(x+18.,y+8.),point(x+8.,y+18.)],1.5,p.secondary_ink);
}

pub struct SettingsView {
    pub shortcut: Shortcut,
    pub startup: bool,
    pub capture: bool,
    pub feedback: &'static str,
    pub theme_hint: &'static str,
}
pub const MODES: [Mode; 3] = [Mode::Light, Mode::Dark, Mode::System];
pub fn settings_controls() -> [(Action, RectF);9] {
    [(Action::Back,rect(16.,17.,34.,34.)),(Action::Shortcut,rect(16.,89.,WIDTH-32.,32.)),
     (Action::Theme(0),rect(16.,162.,94.,30.)),(Action::Theme(1),rect(116.,162.,94.,30.)),
     (Action::Theme(2),rect(216.,162.,94.,30.)),(Action::Startup,rect(17.,213.,WIDTH-34.,24.)),
     (Action::Cancel,rect(WIDTH-184.,249.,78.,29.)),(Action::Save,rect(WIDTH-98.,249.,82.,29.)),
     (Action::Close,rect(CLOSE_X,CLOSE_Y,26.,26.))]
}
pub fn settings(g: &Painter,p: &Palette,form: &SettingsView,mode: Mode, hover: Option<Action>,focus: Option<Action>) {
    g.gradient(rect(0.5,0.5,WIDTH-1.,SETTINGS_HEIGHT as f32-1.),23.,p.glass_start,p.glass_end,p.glass_edge);
    g.text("Настройки",rect(57.,17.,CLOSE_X-61.,34.),18.,true,0,p.ink);
    close(g,p,CLOSE_X,CLOSE_Y,hover == Some(Action::Close),focus == Some(Action::Close));
    g.fill(rect(16.,60.,WIDTH-32.,1.),p.line);
    g.text("ГОРЯЧАЯ КЛАВИША",rect(16.,68.,WIDTH-32.,18.),11.,true,0,p.secondary_ink);
    g.text("ОФОРМЛЕНИЕ",rect(16.,141.,WIDTH-32.,18.),11.,true,0,p.secondary_ink);
    g.text(form.feedback,rect(16.,122.,WIDTH-32.,17.),10.5,false,0,p.secondary_ink);
    g.text(form.theme_hint,rect(16.,194.,WIDTH-32.,17.),10.5,false,0,p.secondary_ink);
    g.fill(rect(16.,242.,WIDTH-32.,1.),p.line);
    for (action, r) in settings_controls() {
        let active = hover == Some(action);
        let focused = focus == Some(action);
        match action {
            Action::Back => {
                button(g,p,r,10.,if active {p.hover} else {p.action_fill},focused);
                let (x,y) = (r.X + r.Width / 2., r.Y + r.Height / 2.);
                g.stroke(&[point(x+6.,y),point(x-6.,y)],1.75,p.ink);
                g.stroke(&[point(x-1.,y-5.),point(x-6.,y),point(x-1.,y+5.)],1.75,p.ink);
            }
            Action::Shortcut => {
                button(g,p,r,11.,if active {p.hover} else {p.selected},focused);
                let text = if form.capture { "Нажмите клавишу или сочетание…".into() }
                    else { form.shortcut.labels().join(" + ") };
                g.text(&text,r,11.5,true,1,p.accent);
            }
            Action::Theme(i) => {
                let index = i as usize;
                let checked = MODES[index] == mode;
                let fill = if active {p.hover} else if checked {p.selected} else {p.elevated};
                g.rounded(r,9.,fill,Some(if focused {p.focus} else {p.border}));
                g.text(["Светлая","Тёмная","Как в Windows"][index],r,11.5,false,1,p.ink);
            }
            Action::Startup => {
                let y = r.Y + 6.;
                g.rounded(rect(r.X + 1.,y,14.,14.),2.,if form.startup {p.save} else {p.surface},
                    Some(if focused {p.focus} else {p.border}));
                if form.startup { g.stroke(&[point(r.X+4.,y+7.),point(r.X+7.,y+10.),point(r.X+12.,y+3.)],1.6,WHITE); }
                g.text("Запускать вместе с Windows",rect(r.X+19.,r.Y,r.Width-19.,r.Height),12.,false,0,p.ink);
            }
            Action::Cancel | Action::Save => {
                let save = action == Action::Save;
                let fill = match (save, active) { (true,true)=>p.save_hover,(true,false)=>p.save,
                    (false,true)=>p.hover,(false,false)=>p.elevated };
                button(g,p,r,11.,fill,focused);
                g.text(if save {"Сохранить"} else {"Отмена"},r,11.5,true,1,if save {WHITE} else {p.ink});
            }
            _ => (),
        }
    }
}

pub fn menu_controls() -> [(Action, RectF);3] {
    [(Action::Open,rect(4.,4.,149.,25.)),(Action::Settings,rect(4.,29.,149.,25.)),(Action::Exit,rect(4.,63.,149.,25.))]
}
pub fn menu(g: &Painter,p: &Palette,hover: Option<Action>,focus: Option<Action>) {
    g.rounded(rect(0.5,0.5,156.,91.),5.,p.surface,Some(p.border));
    g.fill(rect(6.,58.,145.,1.),p.border);
    for ((action,r), text) in menu_controls().into_iter().zip(["Открыть","Настройки","Выход"]) {
        if hover == Some(action) || focus == Some(action) { g.rounded(r,4.,p.hover,None); }
        g.text(text,rect(r.X+9.,r.Y,r.Width-9.,r.Height),12.,false,0,p.ink);
    }
}

pub fn toast(g: &Painter,p: &Palette,title: &str, text: &str) {
    let w = TOAST_WIDTH as f32;
    g.gradient(rect(1.,1.,w-2.,TOAST_HEIGHT as f32-2.),15.,p.surface,p.background,p.border);
    g.rounded(rect(14.,16.,36.,36.),10.,p.selected,None);
    g.speaker(20.,22.,0.94,p.accent,false,true);
    g.text(title,rect(59.,13.,w-72.,18.),10.5,true,0,p.secondary_ink);
    g.text(text,rect(59.,30.,w-72.,22.),13.,true,0,p.ink);
}

pub fn tray_volume(g: &Painter, p: &Palette, volume: Volume) {
    let track = rect(1., 1., TRAY_VOLUME_WIDTH as f32 - 2., TRAY_VOLUME_HEIGHT as f32 - 2.);
    let track_color = 0xA0000000 | (p.volume_track & 0x00FFFFFF);
    g.rounded(track, track.Height / 2., track_color, None);
    let fill = track.Width * volume.level as f32 / 100.;
    if fill > 0. {
        let accent = if volume.muted { p.secondary_ink } else { p.accent };
        g.rounded(rect(track.X, track.Y, fill, track.Height), fill.min(track.Height) / 2., accent, None);
    }
}
