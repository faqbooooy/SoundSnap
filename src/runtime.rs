use alloc::{string::String, vec, vec::Vec};
use core::{alloc::{GlobalAlloc, Layout}, ptr::{null, null_mut}, slice};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::*,
    System::{Environment::{GetCommandLineW, GetEnvironmentVariableW}, LibraryLoader::GetModuleFileNameW,
        Memory::{GetProcessHeap, HeapAlloc, HeapFree, HeapReAlloc}, Threading::ExitProcess},
    UI::Shell::CommandLineToArgvW,
};

struct WindowsHeap;

#[global_allocator]
static HEAP: WindowsHeap = WindowsHeap;

unsafe impl GlobalAlloc for WindowsHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let heap = GetProcessHeap();
        if layout.align() <= 16 {
            return HeapAlloc(heap, 0, layout.size()).cast();
        }
        let Some(size) = layout.size().checked_add(layout.align()) else { return null_mut() };
        let base: *mut u8 = HeapAlloc(heap, 0, size).cast();
        if base.is_null() { return base; }
        let offset = layout.align() - (base as usize % layout.align());
        let aligned = base.add(offset);
        aligned.cast::<*mut u8>().sub(1).write(base);
        aligned
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let base = if layout.align() <= 16 { ptr } else { ptr.cast::<*mut u8>().sub(1).read() };
        HeapFree(GetProcessHeap(), 0, base.cast());
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if layout.align() <= 16 { return HeapReAlloc(GetProcessHeap(),0,ptr.cast(),size).cast(); }
        let Ok(next) = Layout::from_size_align(size, layout.align()) else { return null_mut() };
        let out = self.alloc(next);
        if !out.is_null() {
            core::ptr::copy_nonoverlapping(ptr, out, layout.size().min(size));
            self.dealloc(ptr, layout);
        }
        out
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { ExitProcess(111); }
}

// Prebuilt GNU alloc can reference the personality symbol even in an abort-only executable.
// A foreign unwind cannot be handled by this process.
#[no_mangle]
extern "C" fn rust_eh_personality() -> ! {
    unsafe { ExitProcess(111); }
}

pub fn argument() -> Option<String> {
    unsafe {
        let mut count = 0;
        let argv = CommandLineToArgvW(GetCommandLineW(), &mut count);
        if argv.is_null() { return None; }
        let result = if count <= 1 { None } else {
            let value = *argv.add(1);
            let mut length = 0;
            while *value.add(length) != 0 { length += 1; }
            Some(String::from_utf16_lossy(slice::from_raw_parts(value, length)))
        };
        LocalFree(argv.cast());
        result
    }
}

pub fn executable_path() -> Option<String> {
    let mut buffer = vec![0u16; 32768];
    let size = unsafe { GetModuleFileNameW(null_mut(), buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if size == 0 || size >= buffer.len() { return None; }
    Some(String::from_utf16_lossy(&buffer[..size]))
}

fn data_paths(name: &str, temporary: &str) -> Option<(Vec<u16>, Vec<u16>, Vec<u16>)> {
    let variable = crate::wide("APPDATA");
    let length = unsafe { GetEnvironmentVariableW(variable.as_ptr(), null_mut(), 0) } as usize;
    if length == 0 { return None; }
    let mut directory = vec![0u16; length];
    let used = unsafe { GetEnvironmentVariableW(variable.as_ptr(), directory.as_mut_ptr(), length as u32) } as usize;
    if used == 0 || used >= length { return None; }
    directory.truncate(used);
    directory.extend("\\SoftAudioSwitch".encode_utf16());
    let mut path = directory.clone();
    path.extend(crate::wide(name));
    let mut temp = directory.clone();
    temp.extend(crate::wide(temporary));
    directory.push(0);
    Some((directory, path, temp))
}

pub fn read_settings() -> Option<String> {
    String::from_utf8(read_data("\\settings.json", 65536)?).ok()
}

pub fn read_data(name: &str, limit: i64) -> Option<Vec<u8>> {
    let (_, path, _) = data_paths(name, "")?;
    unsafe {
        let file = CreateFileW(path.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_DELETE, null(),
            OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, null_mut());
        if file == INVALID_HANDLE_VALUE { return None; }
        let mut size = 0i64;
        if GetFileSizeEx(file,&mut size) == 0 || !(0..=limit).contains(&size) {
            CloseHandle(file);
            return None;
        }
        let mut text = vec![0u8; size as usize];
        let mut read = 0;
        let ok = ReadFile(file,text.as_mut_ptr(),size as u32,&mut read,null_mut()) != 0;
        CloseHandle(file);
        (ok && read as usize == text.len()).then_some(text)
    }
}

pub fn write_settings(text: &str) -> bool {
    write_data("\\settings.json", "\\settings.tmp", text.as_bytes())
}

pub fn write_data(name: &str, temporary: &str, text: &[u8]) -> bool {
    let Some((directory, path, temp)) = data_paths(name, temporary) else { return false; };
    unsafe {
        CreateDirectoryW(directory.as_ptr(),null());
        let file = CreateFileW(temp.as_ptr(), GENERIC_WRITE, 0, null(), CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL, null_mut());
        if file == INVALID_HANDLE_VALUE { return false; }
        let mut written = 0;
        let ok = WriteFile(file,text.as_ptr(),text.len() as u32,&mut written,null_mut()) != 0 &&
            written as usize == text.len() && FlushFileBuffers(file) != 0;
        CloseHandle(file);
        if ok && MoveFileExW(temp.as_ptr(),path.as_ptr(),MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) != 0 {
            return true;
        }
        DeleteFileW(temp.as_ptr());
        false
    }
}
