//! Windows helpers for the three things sysinfo does not expose.
//!
//! Everything here is plain FFI against system DLLs: no new crate, no WMI, no
//! child process. That matters because the whole point of this collector is
//! that it *never* shells out - the panel it replaced ran 8~10 PowerShell
//! processes every 3 s and that is what used to peg the CPU.
//!
//! * [`CpuClock`] - real-time CPU frequency. sysinfo's own `frequency()` reads
//!   `CallNtPowerInformation(ProcessorInformation).CurrentMhz`, which on
//!   desktop boards is pinned to the rated clock and therefore never moves
//!   (this machine reports exactly 3696 MHz whether it idles or runs flat
//!   out). The PDH counter below tracks the actual boost instead.
//! * [`memory_speed_mhz`] - installed DDR speed, read from the SMBIOS type-17
//!   records the firmware exposes.
//! * [`device_number`] - maps a volume (`C:`) to the physical disk behind it,
//!   so the per-volume I/O counters can be summed into a per-drive figure.
//!   The counters really are per volume: writing 24 MB to C: leaves D: and E:
//!   at zero.

use std::collections::HashMap;
use std::ffi::c_void;

// --------------------------------------------------------------- CPU clock

#[repr(C)]
struct PdhFmtCounterValue {
    status: u32,
    _pad: u32,
    value: f64,
}

#[link(name = "pdh")]
extern "system" {
    fn PdhOpenQueryW(data_source: *const u16, user_data: usize, query: *mut isize) -> u32;
    fn PdhAddEnglishCounterW(
        query: isize,
        path: *const u16,
        user_data: usize,
        counter: *mut isize,
    ) -> u32;
    fn PdhCollectQueryData(query: isize) -> u32;
    fn PdhGetFormattedCounterValue(
        counter: isize,
        format: u32,
        kind: *mut u32,
        value: *mut PdhFmtCounterValue,
    ) -> u32;
    fn PdhCloseQuery(query: isize) -> u32;
}

const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
const PDH_CSTATUS_VALID_DATA: u32 = 0x0000_0000;
const PDH_CSTATUS_NEW_DATA: u32 = 0x0000_0001;

/// The English path is required: `PdhAddCounterW` would resolve the localised
/// names, and this panel runs on a Chinese Windows where they differ.
const CLOCK_COUNTER: &str = r"\Processor Information(_Total)\% Processor Performance";

/// A live PDH query on the CPU clock counter.
pub struct CpuClock {
    query: isize,
    counter: isize,
}

impl CpuClock {
    pub fn new() -> Option<Self> {
        let path: Vec<u16> = CLOCK_COUNTER
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: `path` is NUL terminated and outlives the call; the two
        // out-parameters are valid, initialised locals.
        unsafe {
            let mut query: isize = 0;
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let mut counter: isize = 0;
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                PdhCloseQuery(query);
                return None;
            }
            // The first collect only primes the counter - a value can be read
            // from the second one onwards, and without this the panel would
            // show a zero for its first tick.
            PdhCollectQueryData(query);
            Some(Self { query, counter })
        }
    }

    /// How fast the CPU is running right now, as a share of its rated clock -
    /// 1.25 means a 25% boost. `None` when the platform does not implement the
    /// counter, so the caller can fall back to the rated value.
    pub fn ratio(&self) -> Option<f64> {
        // SAFETY: both handles come from a live query owned by `self`, and the
        // out-parameters are valid locals.
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let mut kind: u32 = 0;
            let mut value = PdhFmtCounterValue {
                status: 0,
                _pad: 0,
                value: 0.0,
            };
            if PdhGetFormattedCounterValue(self.counter, PDH_FMT_DOUBLE, &mut kind, &mut value) != 0
            {
                return None;
            }
            // Anything else means "no data yet" or a counter that cannot be
            // computed, both of which have to surface as "unknown" rather than
            // as a bogus frequency.
            if value.status != PDH_CSTATUS_VALID_DATA && value.status != PDH_CSTATUS_NEW_DATA {
                return None;
            }
            if !value.value.is_finite() || value.value <= 0.0 {
                return None;
            }
            Some(value.value / 100.0)
        }
    }
}

impl Drop for CpuClock {
    fn drop(&mut self) {
        // SAFETY: `query` is the live handle from `PdhOpenQueryW`.
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

// ------------------------------------------------------------------ SMBIOS

extern "system" {
    fn GetSystemFirmwareTable(signature: u32, table_id: u32, buffer: *mut u8, size: u32) -> u32;
}

/// `'RSMB'` written the way MSVC packs a multi-character constant: big-endian,
/// so `R` lands in the most significant byte. Passing the little-endian
/// `0x424D5352` instead makes every call fail with `ERROR_INVALID_FUNCTION`,
/// which looks exactly like the firmware table being unavailable.
const SMBIOS_SIGNATURE: u32 = 0x5253_4D42;

/// Installed memory speed in MHz.
///
/// SMBIOS reports MT/s, which is the number every BIOS screen and every tool
/// calls MHz, so it is passed through as-is. Returns 0 when the firmware table
/// cannot be read, and the UI then simply omits the figure.
pub fn memory_speed_mhz() -> u32 {
    // SAFETY: the first call only asks for the required size and passes a null
    // buffer; the second one passes a buffer of exactly that size.
    unsafe {
        let size = GetSystemFirmwareTable(SMBIOS_SIGNATURE, 0, std::ptr::null_mut(), 0);
        if size == 0 || size > 4 * 1024 * 1024 {
            return 0;
        }
        let mut raw = vec![0u8; size as usize];
        let written = GetSystemFirmwareTable(SMBIOS_SIGNATURE, 0, raw.as_mut_ptr(), size);
        if written == 0 || written > size {
            return 0;
        }
        raw.truncate(written as usize);
        parse_memory_speed(&raw)
    }
}

/// `RawSMBIOSData` is an 8 byte header (method, major, minor, revision, then a
/// u32 length) followed by the structure table.
fn parse_memory_speed(raw: &[u8]) -> u32 {
    if raw.len() < 8 {
        return 0;
    }
    let table_len = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
    let end = (8 + table_len).min(raw.len());
    let table = &raw[8..end];

    let mut best = 0u32;
    let mut i = 0usize;
    while i + 4 <= table.len() {
        let kind = table[i];
        let len = table[i + 1] as usize;
        // A length below 4 would make the string-table scan below walk
        // backwards; treat it as a malformed table and stop.
        if len < 4 {
            break;
        }

        // 17 = Memory Device. Offsets: 0x0C size in MB, 0x15 speed,
        // 0x20 configured (i.e. actually running) speed.
        if kind == 17 && len >= 0x17 {
            let size_mb = u16::from_le_bytes([table[i + 0x0C], table[i + 0x0D]]);
            let speed = u16::from_le_bytes([table[i + 0x15], table[i + 0x16]]) as u32;
            let configured = if len >= 0x22 {
                u16::from_le_bytes([table[i + 0x20], table[i + 0x21]]) as u32
            } else {
                0
            };
            // An empty slot reports a zero size, and the speed actually in use
            // beats the SPD's rated one when the board downclocks.
            if size_mb != 0 {
                best = best.max(if configured > 0 { configured } else { speed });
            }
        }

        // Each structure is followed by its string table, terminated by an
        // extra NUL; the next structure starts right after that pair.
        let mut j = i + len;
        while j + 1 < table.len() && !(table[j] == 0 && table[j + 1] == 0) {
            j += 1;
        }
        i = j + 2;
    }
    best
}

// ------------------------------------------------------- volume -> drive

extern "system" {
    fn CreateFileW(
        path: *const u16,
        access: u32,
        share: u32,
        attributes: *mut c_void,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    fn DeviceIoControl(
        handle: isize,
        code: u32,
        in_buffer: *mut c_void,
        in_size: u32,
        out_buffer: *mut c_void,
        out_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE_VALUE: isize = -1;
/// `IOCTL_STORAGE_GET_DEVICE_NUMBER`
const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;

/// Physical disk number behind a volume, e.g. `"C:\\"` -> 1.
///
/// `None` when the volume cannot be opened - a card reader with no card, a
/// volume only the SYSTEM account may touch, and so on.
pub fn device_number(mount_point: &str) -> Option<u32> {
    let trimmed = mount_point.trim_end_matches('\\');
    if !trimmed.ends_with(':') {
        return None;
    }
    let wide: Vec<u16> = format!(r"\\.\{trimmed}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: the path is NUL terminated; the output buffer is 12 bytes, which
    // is exactly `sizeof(STORAGE_DEVICE_NUMBER)`.
    unsafe {
        // Access 0 (query only) is deliberate: it needs no elevation, and the
        // three volumes of this machine all answer to it.
        let handle = CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            0,
        );
        if handle == INVALID_HANDLE_VALUE || handle == 0 {
            return None;
        }

        let mut out = [0u8; 12];
        let mut returned: u32 = 0;
        let ok = DeviceIoControl(
            handle,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            std::ptr::null_mut(),
            0,
            out.as_mut_ptr() as *mut c_void,
            out.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        );
        CloseHandle(handle);
        if ok == 0 || returned < 12 {
            return None;
        }
        // STORAGE_DEVICE_NUMBER is { DeviceType, DeviceNumber, PartitionNumber }.
        Some(u32::from_le_bytes([out[4], out[5], out[6], out[7]]))
    }
}

/// Resolve every mount point of the list in one go. The answer cannot change
/// while the app runs, so this is only ever called once, at startup.
pub fn device_numbers<'a, I>(mount_points: I) -> HashMap<String, u32>
where
    I: IntoIterator<Item = &'a str>,
{
    mount_points
        .into_iter()
        .filter_map(|mp| device_number(mp).map(|n| (mp.to_string(), n)))
        .collect()
}
