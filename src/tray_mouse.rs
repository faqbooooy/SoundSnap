use alloc::boxed::Box;
use core::{ffi::c_void, mem::zeroed, ptr::{null, null_mut}};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::{CreateEventW, CreateThread, SetEvent, WaitForSingleObject, INFINITE}},
    UI::WindowsAndMessaging::*,
};
use crate::{tray_icon_at, TRAY_MUTE, TRAY_WHEEL};

// The mouse hook is installed, called, and removed on this worker only.
static mut HOST: HWND = null_mut();
static mut MIDDLE_CAPTURED: bool = false;

struct Start { host: HWND, stop: HANDLE }

pub struct TrayMouse { thread: HANDLE, stop: HANDLE }

impl TrayMouse {
    pub fn start(host: HWND) -> Option<Self> {
        unsafe {
            let stop = CreateEventW(null(), 1, 0, null());
            if stop.is_null() { return None; }
            let args = Box::into_raw(Box::new(Start { host, stop }));
            let thread = CreateThread(null(), 0, Some(run), args.cast(), 0, null_mut());
            if thread.is_null() {
                drop(Box::from_raw(args));
                CloseHandle(stop);
                return None;
            }
            Some(Self { thread, stop })
        }
    }
}

impl Drop for TrayMouse {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.stop);
            // If the shell is stalled, leave the event open until process exit
            // rather than closing a handle the worker may still be waiting on.
            if WaitForSingleObject(self.thread, 1000) == WAIT_OBJECT_0 {
                CloseHandle(self.stop);
            }
            CloseHandle(self.thread);
        }
    }
}

unsafe extern "system" fn run(args: *mut c_void) -> u32 {
    let Start { host, stop } = *Box::from_raw(args.cast());
    HOST = host;
    let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(on_mouse), GetModuleHandleW(null()), 0);
    if hook.is_null() { return 0; }
    let mut message: MSG = zeroed();
    while MsgWaitForMultipleObjectsEx(1, &stop, INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) == WAIT_OBJECT_0 + 1 {
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
            DispatchMessageW(&message);
        }
    }
    UnhookWindowsHookEx(hook);
    0
}

unsafe extern "system" fn on_mouse(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        match message as u32 {
            WM_MBUTTONUP if MIDDLE_CAPTURED => {
                MIDDLE_CAPTURED = false;
                return 1;
            }
            WM_MOUSEWHEEL | WM_MBUTTONDOWN => {
                let input = &*(data as *const MSLLHOOKSTRUCT);
                if tray_icon_at(HOST, input.pt).is_some() {
                    if message as u32 == WM_MOUSEWHEEL {
                        let delta = (input.mouseData >> 16) as i16;
                        if PostMessageW(HOST, TRAY_WHEEL, delta as isize as usize, 0) != 0 { return 1; }
                    } else if PostMessageW(HOST, TRAY_MUTE, 0, 0) != 0 {
                        MIDDLE_CAPTURED = true;
                        return 1;
                    }
                }
            }
            _ => (),
        }
    }
    CallNextHookEx(null_mut(), code, message, data)
}
