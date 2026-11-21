use core::cell::UnsafeCell;
use windows_sys::Win32::System::Threading::{AcquireSRWLockExclusive, ReleaseSRWLockExclusive, SRWLOCK};

pub struct Lock<T> {
    native: UnsafeCell<SRWLOCK>,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Lock<T> {}
unsafe impl<T: Send> Send for Lock<T> {}

impl<T> Lock<T> {
    pub fn new(value: T) -> Self {
        Self { native: UnsafeCell::new(SRWLOCK { Ptr: core::ptr::null_mut() }), value: UnsafeCell::new(value) }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        unsafe {
            AcquireSRWLockExclusive(self.native.get());
            let value = f(&mut *self.value.get());
            ReleaseSRWLockExclusive(self.native.get());
            value
        }
    }
}
