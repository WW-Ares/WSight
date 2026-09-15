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
        claim_windows()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[cfg(windows)]
fn claim_windows() -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    /// `ERROR_ALREADY_EXISTS` - someone else created the mutex first.
    const ERROR_ALREADY_EXISTS: u32 = 183;

    extern "system" {
        fn CreateMutexW(
            attrs: *mut core::ffi::c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut core::ffi::c_void;
        fn GetLastError() -> u32;
    }

    let wide: Vec<u16> = OsStr::new("WSight-Single-Instance-Mutex")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateMutexW(std::ptr::null_mut(), 0, wide.as_ptr());
        if handle.is_null() {
            // Cannot tell - better to start than to refuse to start at all.
            return true;
        }
        // The handle is deliberately never closed: the claim has to last for
        // the whole process, and the mutex dies with us anyway.
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}
