use alloc::vec;
use core::{cell::Cell, ffi::c_void, mem::{size_of, zeroed}, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, SIZE},
    Graphics::{Gdi::*, GdiPlus::*},
    UI::{Shell::SHCreateMemStream, WindowsAndMessaging::{CreateIconIndirect, UpdateLayeredWindow, HICON, ICONINFO, ULW_ALPHA}},
};
use crate::wide;

const PARGB: i32 = 0x000E200B;
pub type Color = u32;
pub const WHITE: Color = 0xffffffff;
pub const fn rgb(color: u32) -> Color { color | 0xff000000 }
pub const fn rect(x: f32, y: f32, w: f32, h: f32) -> RectF { RectF { X: x, Y: y, Width: w, Height: h } }
pub const fn point(x: f32, y: f32) -> PointF { PointF { X: x, Y: y } }
pub fn contains(r: RectF, x: f32, y: f32) -> bool { x >= r.X && x < r.X + r.Width && y >= r.Y && y < r.Y + r.Height }

pub struct GraphicsSession(usize);

#[derive(Clone, Copy)]
struct FontPair {
    regular: *mut GpFontFamily,
    semibold: *mut GpFontFamily,
}

// Painter instances run only on the window's owner thread.
static mut FONTS: [Option<FontPair>; 2] = [None, None];
static mut BRAND_IMAGE: Option<BrandImage> = None;

struct BrandImage {
    image: *mut GpImage,
    stream: *mut c_void,
}

impl BrandImage {
    unsafe fn load() -> Option<Self> {
        let png = include_bytes!("soundsnap.png");
        let stream = SHCreateMemStream(png.as_ptr(), png.len() as u32);
        if stream.is_null() { return None; }

        let mut bitmap = null_mut();
        if GdipCreateBitmapFromStream(stream, &mut bitmap) != 0 {
            Self::release_stream(stream);
            return None;
        }
        Some(Self { image: bitmap.cast(), stream })
    }

    unsafe fn release_stream(stream: *mut c_void) {
        // IStream inherits IUnknown, whose third vtable slot is Release.
        let vtable = *stream.cast::<*const usize>();
        let release: unsafe extern "system" fn(*mut c_void) -> u32 =
            core::mem::transmute(*vtable.add(2));
        release(stream);
    }
}

impl Drop for BrandImage {
    fn drop(&mut self) {
        unsafe {
            GdipDisposeImage(self.image);
            Self::release_stream(self.stream);
        }
    }
}

unsafe fn resolve_family(names: &[&str]) -> *mut GpFontFamily {
    for name in names {
        let mut family = null_mut();
        if GdipCreateFontFamilyFromName(wide(name).as_ptr(), null_mut(), &mut family) == 0 {
            return family;
        }
    }
    null_mut()
}

unsafe fn font_pair(panel: bool) -> FontPair {
    let index = usize::from(panel);
    if let Some(pair) = FONTS[index] {
        return pair;
    }
    let pair = if panel {
        FontPair {
            regular: resolve_family(&["Segoe UI Variable Text", "Segoe UI"]),
            semibold: resolve_family(&["Segoe UI Variable Text Semibold", "Segoe UI Semibold"]),
        }
    } else {
        FontPair {
            regular: resolve_family(&["Segoe UI"]),
            semibold: resolve_family(&["Segoe UI Semibold"]),
        }
    };
    FONTS[index] = Some(pair);
    pair
}
impl GraphicsSession {
    pub fn new() -> Option<Self> {
        let input = GdiplusStartupInput { GdiplusVersion: 1, DebugEventCallback: 0,
            SuppressBackgroundThread: 0, SuppressExternalCodecs: 1 };
        let mut token = 0;
        (unsafe { GdiplusStartup(&mut token, &input, null_mut()) } == 0).then_some(Self(token))
    }
}
impl Drop for GraphicsSession {
    fn drop(&mut self) {
        unsafe {
            let fonts = FONTS;
            FONTS = [None, None];
            for pair in fonts.into_iter().flatten() {
                if !pair.regular.is_null() { GdipDeleteFontFamily(pair.regular); }
                if !pair.semibold.is_null() { GdipDeleteFontFamily(pair.semibold); }
            }
            BRAND_IMAGE = None;
            GdiplusShutdown(self.0);
        }
    }
}

pub struct Surface {
    pub width: i32, pub height: i32,
    dc: HDC, dib: HBITMAP, previous: HGDIOBJ, pixels: *mut u8,
}
impl Surface {
    pub fn new(width: i32, height: i32) -> Option<Self> {
        if width < 1 || height < 1 { return None; }
        unsafe {
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() { return None; }
            let mut pixels = null_mut();
            let dib = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
            if dib.is_null() || pixels.is_null() {
                if !dib.is_null() { DeleteObject(dib); }
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, dib);
            if previous.is_null() || previous as isize == -1 {
                DeleteObject(dib); DeleteDC(dc); return None;
            }
            Some(Self { width, height, dc, dib, previous, pixels: pixels.cast() })
        }
    }
    pub fn draw(&mut self, scale: f32, action: impl FnOnce(&Painter)) -> bool {
        unsafe {
            let mut bitmap = null_mut();
            if GdipCreateBitmapFromScan0(self.width, self.height, self.width * 4, PARGB, self.pixels, &mut bitmap) != 0 {
                return false;
            }
            let mut graphics = null_mut();
            let ready = GdipGetImageGraphicsContext(bitmap.cast(), &mut graphics) == 0;
            if ready {
                GdipGraphicsClear(graphics, 0);
                GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias8x8);
                GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
                GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
                GdipScaleWorldTransform(graphics, scale, scale, MatrixOrderPrepend);
                let painter = Painter::new(graphics);
                action(&painter);
                drop(painter);
                GdipDeleteGraphics(graphics);
            }
            GdipDisposeImage(bitmap.cast());
            ready
        }
    }
    pub fn present(&self, window: HWND, position: POINT) -> bool {
        let size = SIZE { cx: self.width, cy: self.height };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe { UpdateLayeredWindow(window, null_mut(), &position, &size, self.dc, &source, 0, &blend, ULW_ALPHA) != 0 }
    }
    pub fn rounded_alpha(&self) -> bool {
        let alpha = |x, y| unsafe { *self.pixels.add(((y * self.width + x) * 4 + 3) as usize) };
        [(3,3),(self.width-4,3),(3,self.height-4),(self.width-4,self.height-4)]
            .into_iter().all(|(x,y)| alpha(x,y) == 0) && alpha(self.width/2, 10) == 255
    }
    fn icon(&self) -> HICON {
        unsafe {
            let stride = ((self.width + 15) / 16 * 2) as usize;
            let mask_data = vec![0u8; stride * self.height as usize];
            let mask = CreateBitmap(self.width, self.height, 1, 1, mask_data.as_ptr().cast());
            if mask.is_null() { return null_mut(); }
            let data = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: self.dib };
            let icon = CreateIconIndirect(&data);
            DeleteObject(mask);
            icon
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) { unsafe { SelectObject(self.dc, self.previous); DeleteObject(self.dib); DeleteDC(self.dc); } }
}

struct Path(*mut GpPath);
impl Path {
    fn new() -> Self {
        let mut path = null_mut();
        unsafe { GdipCreatePath(FillModeAlternate, &mut path); }
        Self(path)
    }
    fn rounded(r: RectF, radius: f32) -> Self {
        let path = Self::new();
        let d = (2. * radius).min(r.Width).min(r.Height);
        if d <= 0. { return path; }
        unsafe {
            GdipAddPathArc(path.0, r.X, r.Y, d, d, 180., 90.);
            GdipAddPathArc(path.0, r.X + r.Width - d, r.Y, d, d, 270., 90.);
            GdipAddPathArc(path.0, r.X + r.Width - d, r.Y + r.Height - d, d, d, 0., 90.);
            GdipAddPathArc(path.0, r.X, r.Y + r.Height - d, d, d, 90., 90.);
            GdipClosePathFigure(path.0);
        }
        path
    }
}
impl Drop for Path { fn drop(&mut self) { unsafe { if !self.0.is_null() { GdipDeletePath(self.0); } } } }

pub struct Painter {
    graphics: *mut GpGraphics,
    panel_typography: Cell<bool>,
}
impl Painter {
    fn new(graphics: *mut GpGraphics) -> Self {
        Self { graphics, panel_typography: Cell::new(false) }
    }
    pub fn use_panel_typography(&self) {
        self.panel_typography.set(true);
    }
    pub fn brand_logo(&self, x: f32, y: f32, size: f32) {
        unsafe {
            if matches!(BRAND_IMAGE, None) {
                BRAND_IMAGE = BrandImage::load();
            }
            if let Some(ref image) = BRAND_IMAGE {
                GdipDrawImageRect(self.graphics, image.image, x, y, size, size);
            }
        }
    }
    pub fn rounded(&self, r: RectF, radius: f32, fill: Color, edge: Option<Color>) {
        if r.Width <= 0. || r.Height <= 0. { return; }
        let path = Path::rounded(r, radius);
        self.fill_path(&path, fill);
        if let Some(edge) = edge { self.paint_path(&path, 1., edge); }
    }
    pub fn gradient(&self, r: RectF, radius: f32, start: Color, end: Color, edge: Color) {
        let path = Path::rounded(r, radius);
        unsafe {
            let mut brush = null_mut();
            if GdipCreateLineBrush(&point(r.X, r.Y), &point(r.X + r.Width * 0.72, r.Y + r.Height),
                    start, end, WrapModeTileFlipXY, &mut brush) == 0 {
                GdipFillPath(self.graphics, brush.cast(), path.0);
                GdipDeleteBrush(brush.cast());
            } else { self.fill_path(&path, start); }
        }
        self.paint_path(&path, 1., edge);
    }
    pub fn fill(&self, r: RectF, color: Color) {
        unsafe {
            let mut brush = null_mut();
            if GdipCreateSolidFill(color, &mut brush) == 0 {
                GdipFillRectangle(self.graphics, brush.cast(), r.X, r.Y, r.Width, r.Height);
                GdipDeleteBrush(brush.cast());
            }
        }
    }
    pub fn circle(&self, r: RectF, color: Color) {
        unsafe {
            let mut brush = null_mut();
            if GdipCreateSolidFill(color, &mut brush) == 0 {
                GdipFillEllipse(self.graphics, brush.cast(), r.X, r.Y, r.Width, r.Height);
                GdipDeleteBrush(brush.cast());
            }
        }
    }
    fn fill_path(&self, path: &Path, color: Color) {
        if color >> 24 == 0 { return; }
        unsafe {
            let mut brush = null_mut();
            if GdipCreateSolidFill(color, &mut brush) == 0 {
                GdipFillPath(self.graphics, brush.cast(), path.0);
                GdipDeleteBrush(brush.cast());
            }
        }
    }
    fn paint_path(&self, path: &Path, width: f32, color: Color) {
        unsafe {
            let mut pen = null_mut();
            if GdipCreatePen1(color, width, UnitPixel, &mut pen) == 0 {
                GdipSetPenStartCap(pen, LineCapRound);
                GdipSetPenEndCap(pen, LineCapRound);
                GdipSetPenLineJoin(pen, LineJoinRound);
                GdipDrawPath(self.graphics, pen, path.0);
                GdipDeletePen(pen);
            }
        }
    }
    pub fn stroke(&self, points: &[PointF], width: f32, color: Color) {
        let path = Path::new();
        unsafe { GdipAddPathLine2(path.0, points.as_ptr(), points.len() as i32); }
        self.paint_path(&path, width, color);
    }
    pub fn speaker(&self, x: f32, y: f32, scale: f32, color: Color, muted: bool, filled: bool) {
        let p = |u, v| point(x + u * scale, y + v * scale);
        let body = Path::new();
        let points = [p(2.,8.),p(6.,8.),p(12.,3.),p(12.,21.),p(6.,16.),p(2.,16.)];
        unsafe { GdipAddPathPolygon(body.0, points.as_ptr(), points.len() as i32); }
        if filled { self.fill_path(&body, color); } else { self.paint_path(&body, 1.75 * scale, color); }
        if muted {
            self.stroke(&[p(17.,9.), p(23.,15.)], 1.65 * scale, color);
            self.stroke(&[p(23.,9.), p(17.,15.)], 1.65 * scale, color);
        } else {
            let wave = Path::new();
            for points in [[p(16.,8.),p(18.,10.),p(18.,14.),p(16.,16.)], [p(19.,4.),p(24.,8.),p(24.,16.),p(19.,20.)]] {
                unsafe { GdipStartPathFigure(wave.0); GdipAddPathBeziers(wave.0, points.as_ptr(), 4); }
            }
            self.paint_path(&wave, 1.75 * scale, color);
        }
    }
    pub fn gear(&self, x: f32, y: f32, scale: f32, color: Color) {
        let path = Path::new();
        let points = [(10.,3.),(14.,3.),(14.6,5.4),(16.3,6.4),(18.7,5.7),(20.7,9.1),(19.,10.9),
            (19.,13.1),(20.7,14.9),(18.7,18.3),(16.3,17.6),(14.6,18.6),(14.,21.),(10.,21.),
            (9.4,18.6),(7.7,17.6),(5.3,18.3),(3.3,14.9),(5.,13.1),(5.,10.9),(3.3,9.1),(5.3,5.7),(7.7,6.4),(9.4,5.4)]
            .map(|(u,v)| point(x + u*scale, y + v*scale));
        unsafe {
            GdipAddPathPolygon(path.0, points.as_ptr(), points.len() as i32);
            GdipAddPathEllipse(path.0, x + 9.*scale, y + 9.*scale, 6.*scale, 6.*scale);
        }
        self.paint_path(&path, 2.25 * scale, color);
    }
    fn setup_text(&self, size: f32, bold: bool, align: i32) -> (*mut GpFont, *mut GpStringFormat) {
        let mut font = null_mut();
        let mut format = null_mut();
        unsafe {
            let pair = font_pair(self.panel_typography.get());
            let semibold = bold && !pair.semibold.is_null();
            let family = if semibold { pair.semibold } else { pair.regular };
            if GdipCreateFont(family, size, i32::from(bold && !semibold), UnitPixel, &mut font) != 0 {
                GdipCreateFont(pair.regular, size, 0, UnitPixel, &mut font);
            }
            GdipCreateStringFormat(StringFormatFlagsNoWrap, 0, &mut format);
            GdipSetStringFormatAlign(format, align);
            GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
            GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter);
        }
        (font, format)
    }
    pub fn text(&self, text: &str, r: RectF, size: f32, bold: bool, align: i32, color: Color) {
        let s = wide(text);
        let (font, format) = self.setup_text(size, bold, align);
        unsafe {
            let mut brush = null_mut();
            if !font.is_null() && GdipCreateSolidFill(color, &mut brush) == 0 {
                // Reinforce the antialiased stroke without changing glyph advances or font sizes.
                let reinforcement = ((color >> 24) * 2 / 5) << 24 | (color & 0x00FFFFFF);
                let mut softer_brush = null_mut();
                if GdipCreateSolidFill(reinforcement, &mut softer_brush) == 0 {
                    let reinforced = rect(r.X + 0.14, r.Y, r.Width, r.Height);
                    GdipDrawString(self.graphics, s.as_ptr(), s.len() as i32 - 1, font, &reinforced, format, softer_brush.cast());
                    GdipDeleteBrush(softer_brush.cast());
                }
                GdipDrawString(self.graphics, s.as_ptr(), s.len() as i32 - 1, font, &r, format, brush.cast());
                GdipDeleteBrush(brush.cast());
            }
            if !font.is_null() { GdipDeleteFont(font); }
            if !format.is_null() { GdipDeleteStringFormat(format); }
        }
    }
}
pub fn tray_icon(dark: bool, muted: bool, size: i32) -> HICON {
    let Some(mut surface) = Surface::new(size, size) else { return null_mut() };
    surface.draw(size as f32 / 32., |g| {
        let speaker = if dark { WHITE } else { rgb(0x142335) };
        let arrow = if dark { rgb(0xADDAFF) } else { rgb(0x0052A7) };
        let edge = if dark { 0xE1172537 } else { 0xEBF7FBFF };
        let body = Path::new();
        let points = [point(2.4,11.),point(7.6,11.),point(15.,5.5),point(15.,26.5),point(7.6,21.),point(2.4,21.)];
        let arc = Path::new();
        let head = [point(23.3,4.4),point(19.9,7.8),point(23.2,11.2)];
        unsafe {
            GdipAddPathPolygon(body.0, points.as_ptr(), 6);
            GdipAddPathBezier(arc.0, 19.5,22.7, 27.,23.5, 30.,16., 26.4,10.4);
            GdipAddPathBezier(arc.0, 26.4,10.4, 25.,8., 22.3,7.8, 21.2,7.8);
            GdipStartPathFigure(arc.0);
            GdipAddPathLine2(arc.0, head.as_ptr(), 3);
        }
        g.paint_path(&body, 2.4, edge);
        g.fill_path(&body, speaker);
        if muted {
            let mute = if dark { rgb(0xFFADB7) } else { rgb(0x9E2941) };
            for stroke in [[point(20.,12.),point(28.,20.)],[point(28.,12.),point(20.,20.)]] {
                g.stroke(&stroke, 6., edge);
                g.stroke(&stroke, 3.4, mute);
            }
        } else {
            g.paint_path(&arc, 6., edge);
            g.paint_path(&arc, 3.6, arrow);
        }
    });
    surface.icon()
}
