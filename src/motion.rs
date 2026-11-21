use windows_sys::Win32::{System::SystemInformation::GetTickCount64,
    UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION}};

pub const FRAME_MS: u32 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub opacity: u8,
    pub offset: i32,
}

impl Frame {
    pub const REST: Self = Self { opacity: 255, offset: 0 };
}

pub struct Transition {
    start: u64,
    duration: u64,
    from: Frame,
    to: Frame,
}

impl Transition {
    pub fn new(from: Frame, to: Frame, duration: u64) -> Self {
        Self { start: now(), duration, from, to }
    }

    pub fn closing(&self) -> bool { self.to.opacity == 0 }

    pub fn frame(&self, time: u64) -> Option<Frame> {
        let elapsed = time.saturating_sub(self.start);
        if elapsed >= self.duration { return None; }
        let t = elapsed as f32 / self.duration as f32;
        let eased = if self.closing() {
            t * t
        } else {
            let remaining = 1. - t;
            1. - remaining * remaining * remaining
        };
        let interpolate = |from: i32,to: i32| {
            (from as f32 + (to-from) as f32 * eased + 0.5) as i32
        };
        Some(Frame {
            opacity: interpolate(self.from.opacity.into(), self.to.opacity.into()) as u8,
            offset: interpolate(self.from.offset, self.to.offset),
        })
    }
}

pub fn now() -> u64 { unsafe { GetTickCount64() } }

pub fn allowed() -> bool {
    let mut enabled = 0i32;
    unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION,0,(&mut enabled as *mut i32).cast(),0) != 0 && enabled != 0
    }
}
