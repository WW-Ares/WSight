//! Refuse to run a second copy of WSight.
//!
//! Two instances share one WebView2 user-data folder, and that is a reliable
//! way to get a window that never paints: the second webview blocks on the
//! profile the first one already owns, leaving an empty frame that ignores the
//! close button. That is the shape of the auto-start bug - the Run entry fires
//! at logon, the user clicks the icon because nothing appeared yet, and now
//! there are two. A named mutex turns the second launch into a no-op.

/// True when this process is the only WSight running.
pub fn claim() -> bool {
    #[cfg(windows)]
    {
        matches!(claim_windows(), Some(true))
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// The `--handover` path of the self-updater: the previous build renamed itself
/// out of the way and started us while it was still alive, so we are a
/// "duplicate" for exactly as long as it takes it to exit.
///
/// Polling rather than waiting on the handle: the mutex is not signalled, it
/// *ceases to exist* when its owner dies, and `CreateMutexW` is the only call
/// that reports a difference between "no such mutex" and "someone owns it".
/// Returns false on timeout, and the caller then gives up quietly - starting a
/// second copy next to a live one is the very thing this module exists to stop.
#[cfg(windows)]
pub fn wait_for_release(timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match claim_windows() {
            Some(true) => return true,
            // Someone still owns it. `claim_windows` already let go of the
            // handle it was given, so this loop is not itself keeping the
            // mutex alive.
            Some(false) => {}
            None => return false,
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(not(windows))]
pub fn wait_for_release(_timeout: std::time::Duration) -> bool {
    true
}

/// `Some(true)` when this process now owns the mutex, `Some(false)` when
/// another copy owns it, `None` when Windows would not tell us.
#[cfg(windows)]
fn claim_windows() -> Option<bool> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicIsize, Ordering};

    /// `ERROR_ALREADY_EXISTS` - someone else created the mutex first.
    const ERROR_ALREADY_EXISTS: u32 = 183;

    extern "system" {
        fn CreateMutexW(
            attrs: *mut core::ffi::c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut core::ffi::c_void;
        fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
        fn GetLastError() -> u32;
    }

    /// The winning handle, kept for the life of the process. Deliberately never
    /// closed: the claim has to outlive every other thread, and the mutex dies
    /// with us anyway.
    static HELD: AtomicIsize = AtomicIsize::new(0);

    let wide: Vec<u16> = OsStr::new("WSight-Single-Instance-Mutex")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateMutexW(std::ptr::null_mut(), 0, wide.as_ptr());
        if handle.is_null() {
            // Cannot tell - better to start than to refuse to start at all.
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Dropping this handle matters: holding one would put us in the
            // same "somebody owns it" state as the real owner, and
            // `wait_for_release` would never see the mutex go away.
            CloseHandle(handle);
            return Some(false);
        }
        HELD.store(handle as isize, Ordering::SeqCst);
        Some(true)
    }
}
