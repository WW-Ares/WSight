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
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
use winreg::RegKey;

// --------------------------------------------------------------- CPU clock

#[repr(C)]
struct PdhFmtCounterValue {
    status: u32,
    _pad: u32,
    value: f64,
}

/// One row of a PDH counter array. The instance name lives in a buffer that
/// follows the array itself, so `name` points into that same allocation.
#[repr(C)]
struct PdhFmtCounterValueItemW {
    name: *const u16,
    value: PdhFmtCounterValue,
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
    fn PdhGetFormattedCounterArrayW(
        counter: isize,
        format: u32,
        buffer_size: *mut u32,
        item_count: *mut u32,
        buffer: *mut PdhFmtCounterValueItemW,
    ) -> u32;
    fn PdhCloseQuery(query: isize) -> u32;
}

const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
const PDH_CSTATUS_VALID_DATA: u32 = 0x0000_0000;
const PDH_CSTATUS_NEW_DATA: u32 = 0x0000_0001;
/// "The buffer was too small" - the documented way to ask for the size.
const PDH_MORE_DATA: u32 = 0x8000_07D2;

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

// -------------------------------------------------------------- GPU load

/// Every GPU engine of every process, as one counter.
///
/// The `*` is a wildcard: Windows publishes one instance per (process, engine)
/// pair - `pid_1234_luid_0x0_0x…_phys_0_eng_0_engtype_3D` - so the value has
/// to be read as an array and aggregated by hand.
const GPU_COUNTER: &str = r"\GPU Engine(*)\Utilization Percentage";

/// A live PDH query on GPU engine utilisation.
///
/// This is what makes the GPU column honest on AMD and Intel machines. NVML
/// only speaks to NVIDIA; the registry knows those cards' names and their
/// memory but not their load, so without this the ring would sit at `--`
/// forever on a perfectly working Radeon. Task Manager reads the same counters.
///
/// Counters can be switched off by policy, and a machine with no 3D engine
/// reports no instances at all, so every method here is allowed to return
/// `None` - the caller renders "unknown", never a made-up zero.
pub struct GpuLoad {
    query: isize,
    counter: isize,
}

impl GpuLoad {
    pub fn new() -> Option<Self> {
        let path: Vec<u16> = GPU_COUNTER
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
            // Prime it: the first collect yields no comparable values.
            PdhCollectQueryData(query);
            Some(Self { query, counter })
        }
    }

    /// Utilisation of the busiest engine, 0..100.
    ///
    /// Summing per engine type and taking the maximum is what Task Manager's
    /// GPU graph shows. Summing *across* engines would routinely exceed 100%
    /// - a game using 3D and Copy at once reports both - and reporting one
    /// arbitrary instance would show whichever process happened to be first.
    pub fn percent(&self) -> Option<f32> {
        // SAFETY: handles come from a live query owned by `self`. The array
        // buffer is over-allocated as `u64` words so the cast to a struct
        // holding a pointer is correctly aligned; PDH writes only `size`
        // bytes and reports `count` entries, both of which are honoured.
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let mut size: u32 = 0;
            let mut count: u32 = 0;
            let probe = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                std::ptr::null_mut(),
            );
            // An idle GPU with nothing submitted legitimately has no
            // instances, which comes back as something other than MORE_DATA.
            if probe != PDH_MORE_DATA || size == 0 || count == 0 {
                return None;
            }

            let mut words = vec![0u64; size as usize / 8 + 1];
            let items = words.as_mut_ptr() as *mut PdhFmtCounterValueItemW;
            if PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                items,
            ) != 0
            {
                return None;
            }

            let mut by_engine: HashMap<String, f64> = HashMap::new();
            for item in std::slice::from_raw_parts(items, count as usize) {
                let status = item.value.status;
                if status != PDH_CSTATUS_VALID_DATA && status != PDH_CSTATUS_NEW_DATA {
                    continue;
                }
                let value = item.value.value;
                if !value.is_finite() || value <= 0.0 {
                    continue;
                }
                *by_engine.entry(engine_of(item.name)).or_insert(0.0) += value;
            }

            let busiest = by_engine.values().copied().fold(0.0f64, f64::max);
            if by_engine.is_empty() {
                // Instances existed but every one was idle.
                return Some(0.0);
            }
            Some(busiest.clamp(0.0, 100.0) as f32)
        }
    }
}

impl Drop for GpuLoad {
    fn drop(&mut self) {
        // SAFETY: `query` is the live handle from `PdhOpenQueryW`.
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

/// `…_engtype_3D` -> `3D`; anything unrecognised groups under `other` so its
/// values still count towards some engine rather than being dropped.
fn engine_of(instance: *const u16) -> String {
    if instance.is_null() {
        return "other".to_string();
    }
    // The buffer is NUL terminated by PDH, and lives as long as the array.
    let mut len = 0usize;
    // SAFETY: walking a NUL-terminated UTF-16 buffer owned by the caller's
    // array, which is alive for the duration of this call.
    unsafe {
        while *instance.add(len) != 0 {
            len += 1;
        }
        let name = String::from_utf16_lossy(std::slice::from_raw_parts(instance, len));
        match name.split_once("engtype_") {
            Some((_, rest)) => rest.split('_').next().unwrap_or("other").to_string(),
            None => "other".to_string(),
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
    memory_modules()
        .iter()
        .map(|m| {
            // A board that downclocks below the SPD's rating is running at the
            // configured figure, so that one is the honest answer.
            if m.configured_mhz > 0 {
                m.configured_mhz
            } else {
                m.speed_mhz
            }
        })
        .max()
        .unwrap_or(0)
}

/// One populated DIMM, straight out of an SMBIOS type-17 record.
///
/// Everything here is a property of the hardware: a module cannot change its
/// part number mid-session, so the collector reads these once and the panel
/// may show them before a single measurement has been taken.
#[derive(Debug, Clone)]
pub struct MemoryModule {
    /// 0 only for an empty slot, which is filtered out entirely
    pub size_mb: u64,
    /// what the SPD advertises, 0 when the firmware omits it
    pub speed_mhz: u32,
    /// what the board is actually driving the module at
    pub configured_mhz: u32,
    /// "DDR4" and friends; empty when the byte says nothing useful
    pub type_name: String,
    pub vendor: String,
    pub part_no: String,
}

/// Read the raw SMBIOS table. Empty when the firmware refuses to hand it over.
fn read_smbios() -> Vec<u8> {
    // SAFETY: the first call only asks for the required size and passes a null
    // buffer; the second one passes a buffer of exactly that size.
    unsafe {
        let size = GetSystemFirmwareTable(SMBIOS_SIGNATURE, 0, std::ptr::null_mut(), 0);
        if size == 0 || size > 4 * 1024 * 1024 {
            return Vec::new();
        }
        let mut raw = vec![0u8; size as usize];
        let written = GetSystemFirmwareTable(SMBIOS_SIGNATURE, 0, raw.as_mut_ptr(), size);
        if written == 0 || written > size {
            return Vec::new();
        }
        raw.truncate(written as usize);
        raw
    }
}

/// One structure of the table: its type, the fixed-area bytes, and the string
/// table that trails them.
struct SmbiosStruct {
    kind: u8,
    area: Vec<u8>,
    strings: Vec<String>,
}

impl SmbiosStruct {
    /// SMBIOS string indices are 1-based, and 0 means "not present" - so an
    /// absent field really is absent rather than "the first string".
    fn string(&self, offset: usize) -> &str {
        let index = self.area.get(offset).copied().unwrap_or(0);
        if index == 0 {
            return "";
        }
        self.strings
            .get(index as usize - 1)
            .map(String::as_str)
            .unwrap_or("")
    }

    fn u16(&self, offset: usize) -> u16 {
        match self.area.get(offset..offset + 2) {
            Some(b) => u16::from_le_bytes([b[0], b[1]]),
            None => 0,
        }
    }

    fn u32(&self, offset: usize) -> u32 {
        match self.area.get(offset..offset + 4) {
            Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            None => 0,
        }
    }
    fn byte(&self, offset: usize) -> u8 {
        self.area.get(offset).copied().unwrap_or(0)
    }
}

/// The string table after a structure: NUL separated, closed by an extra NUL.
fn decode_strings(blob: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    for &b in blob {
        if b == 0 {
            out.push(String::from_utf8_lossy(&cur).trim().to_string());
            cur.clear();
        } else {
            cur.push(b);
        }
    }
    if !cur.is_empty() {
        out.push(String::from_utf8_lossy(&cur).trim().to_string());
    }
    out
}

/// `RawSMBIOSData` is an 8 byte header (method, major, minor, revision, then a
/// u32 length) followed by the structure table.
fn parse_smbios(raw: &[u8]) -> Vec<SmbiosStruct> {
    if raw.len() < 8 {
        return Vec::new();
    }
    let table_len = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
    let end = (8 + table_len).min(raw.len());
    let table = &raw[8..end];

    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= table.len() {
        let kind = table[i];
        let len = table[i + 1] as usize;
        // A length below 4 would make the string-table scan below walk
        // backwards; treat it as a malformed table and stop.
        if len < 4 {
            break;
        }
        let area = table[i..i + len].to_vec();

        // Each structure is followed by its string table, terminated by an
        // extra NUL; the next structure starts right after that pair.
        let mut j = i + len;
        while j + 1 < table.len() && !(table[j] == 0 && table[j + 1] == 0) {
            j += 1;
        }
        let strings = decode_strings(&table[i + len..j.min(table.len())]);
        out.push(SmbiosStruct {
            kind,
            area,
            strings,
        });
        i = j + 2;
    }
    out
}

/// SMBIOS "Memory Type", type-17 offset 0x12. Anything not in here is either
/// exotic or obsolete, and an empty string is more honest than a guess.
fn memory_type_name(kind: u8) -> &'static str {
    match kind {
        0x12 => "DDR",
        0x13 => "DDR2",
        0x14 => "DDR2 FB",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        0x20 => "HBM",
        0x21 => "HBM2",
        _ => "",
    }
}

/// Every populated memory slot. Empty when the firmware table is unreadable,
/// which the callers treat as "unknown" rather than as an error.
pub fn memory_modules() -> Vec<MemoryModule> {
    let raw = read_smbios();
    let mut out = Vec::new();

    // 17 = Memory Device. Offsets: 0x0C size, 0x12 type, 0x15 speed,
    // 0x17 manufacturer, 0x1A part number, 0x1C extended size,
    // 0x20 configured speed.
    for s in parse_smbios(&raw).iter().filter(|s| s.kind == 17) {
        let raw_size = s.u16(0x0C);
        let size_mb: u64 = match raw_size {
            // 0 = nothing in the slot, 0xFFFF = the firmware will not say.
            0 | 0xFFFF => 0,
            // 0x7FFF means "too big for this field", and SMBIOS 2.7+ then
            // carries the real figure in the dword below.
            0x7FFF => (s.u32(0x1C) & 0x7FFF_FFFF) as u64,
            // Bit 15 set switches the unit from MB to KB - a 512 MB module in
            // a table that predates the wider field.
            n if n & 0x8000 != 0 => ((n & 0x7FFF) as u64) / 1024,
            n => n as u64,
        };
        if size_mb == 0 {
            continue;
        }

        out.push(MemoryModule {
            size_mb,
            speed_mhz: s.u16(0x15) as u32,
            configured_mhz: s.u16(0x20) as u32,
            type_name: memory_type_name(s.byte(0x12)).to_string(),
            vendor: s.string(0x17).to_string(),
            part_no: s.string(0x1A).to_string(),
        });
    }
    out
}

/// Mainboard model, from the SMBIOS type-2 record - `ROG MAXIMUS XII HERO
/// (WI-FI)` on this machine.
///
/// Two candidate strings live there (maker and product) and the maker is the
/// less useful one: a user recognises their board by its model, not by
/// "ASUSTeK COMPUTER INC.". The maker is only used when the product is blank.
pub fn motherboard() -> String {
    let raw = read_smbios();
    for s in parse_smbios(&raw).iter().filter(|s| s.kind == 2) {
        let product = s.string(0x05);
        if !product.is_empty() {
            return product.to_string();
        }
        let maker = s.string(0x04);
        if !maker.is_empty() {
            return maker.to_string();
        }
    }
    String::new()
}

// ------------------------------------------------------- display adapters

/// Where Windows keeps the driver state of every display adapter. Each one
/// installed gets a four-digit subkey plus the subkeys belonging to the class
/// itself (`Properties`, `Configuration`).
const DISPLAY_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

/// Name fragments that mean "this is not a graphics card".
///
/// The `PCI\` test below catches the indirect displays - RDP, Sunlogin,
/// GameViewer all register under `Root\` or as `RdpIdd_*`. It does *not* catch
/// the emulated ones: VMware SVGA, VirtualBox, QXL and VirtIO are genuine PCI
/// devices and would win the ranking on a virtual machine. Hence both tests.
const VIRTUAL_HINTS: &[&str] = &[
    "virtual",
    "indirect",
    "idd",
    "remote display",
    "basic display",
    "basic render",
    "hyper-v",
    "vmware",
    "virtualbox",
    "qxl",
    "virtio",
    "svga",
    "mirror",
    "vga graphics adapter",
    "display only",
    "splashtop",
    "parsec",
    "todesk",
    "oray",
    "sunlogin",
    "anydesk",
    "rustdesk",
    "gameviewer",
    "citrix",
    "meta virtual",
    "usb mobile monitor",
    "astral",
    "dameware",
    "windows virtual display",
    "amazon vdi",
    "ngfx",
];

/// A display adapter, as Windows itself describes it.
pub struct DisplayAdapter {
    /// `NVIDIA GeForce RTX 2080 Ti` - the driver's own string, which is also
    /// what the caption rules in `src/monitor/hwName.ts` are written against.
    pub name: String,
    /// Bytes of VRAM, 0 when the driver does not publish it.
    pub vram_bytes: u64,
}

fn decode_int(bytes: &[u8]) -> u64 {
    match bytes.len() {
        8 => u64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]),
        4 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
        _ => 0,
    }
}

/// VRAM in bytes, plus whether it came from the trustworthy 64-bit value.
///
/// `HardwareInformation.qwMemorySize` is a `REG_QWORD` and matches NVML to the
/// byte on this machine (11.0 GiB for an RTX 2080 Ti). Intel instead publishes
/// the 32-bit `HardwareInformation.MemorySize`, whose value is a fixed claim
/// rather than a measurement - an integrated GPU reports 4 GB of shared memory
/// no matter what the driver actually hands out. That is why the two are not
/// simply maxed: the flag lets the ranking prefer a card that states its memory
/// properly over one that guesses.
fn vram_from(key: &RegKey) -> (u64, bool) {
    if let Ok(v) = key.get_raw_value("HardwareInformation.qwMemorySize") {
        let n = decode_int(&v.bytes);
        if n > 0 {
            return (n, true);
        }
    }
    if let Ok(v) = key.get_raw_value("HardwareInformation.MemorySize") {
        let n = decode_int(&v.bytes);
        if n > 0 {
            return (n, false);
        }
    }
    (0, false)
}

fn is_virtual_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    VIRTUAL_HINTS.iter().any(|hint| lower.contains(hint))
}

/// Real graphics adapters on this machine, best first.
///
/// This exists because NVML only speaks to NVIDIA cards. Before it, every AMD
/// and Intel user saw an empty GPU column - and the column is only blank *if
/// the driver is missing*, which is not what a monitor should say about a
/// working machine. The registry knows the name and the memory for everyone;
/// it supplies neither usage nor temperature, and those the caller reports as
/// unavailable rather than as zero (see `collector.rs`).
pub fn display_adapters() -> Vec<DisplayAdapter> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(class) = hklm.open_subkey_with_flags(DISPLAY_CLASS, KEY_READ) else {
        return Vec::new();
    };

    let mut ranked: Vec<(f64, DisplayAdapter)> = Vec::new();
    for slot in class.enum_keys().flatten() {
        if slot.len() != 4 || !slot.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(key) = class.open_subkey_with_flags(&slot, KEY_READ) else {
            continue;
        };
        let name: String = key.get_value("DriverDesc").unwrap_or_default();
        let device_id: String = key.get_value("MatchingDeviceId").unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let device_id = device_id.to_ascii_lowercase();
        // A real card reaches us over PCI. Nothing indirect does.
        if !device_id.starts_with("pci\\") {
            continue;
        }
        if is_virtual_name(&name) {
            continue;
        }
        let vendor = device_id
            .split_once("ven_")
            .map(|(_, rest)| rest.chars().take(4).collect::<String>())
            .unwrap_or_default();
        let (vram, dedicated) = vram_from(&key);
        // Vendor alone must not outrank memory - an 8 GB Arc is the GPU an
        // Intel user wants named, not the 1 GB iGPU beside it. So the vendor
        // term is only a tie-breaker, and stating the memory properly is worth
        // more than claiming a large number.
        let vendor_bias = match vendor.as_str() {
            "10de" => 3.0,
            "1002" | "1022" => 2.0,
            "8086" => 1.0,
            _ => 0.0,
        };
        let score = (if dedicated { 16.0 } else { 0.0 })
            + vram as f64 / 1024f64.powi(3)
            + vendor_bias / 100.0;
        ranked.push((
            score,
            DisplayAdapter {
                name,
                vram_bytes: vram,
            },
        ));
    }

    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    ranked.into_iter().map(|(_, a)| a).collect()
}

// -------------------------------------------------------------- adapters

#[repr(C)]
struct SocketAddress {
    sockaddr: *const u8,
    len: i32,
}

/// Only the head of `IP_ADAPTER_UNICAST_ADDRESS` - the link to the next entry
/// and the address itself. Everything after them is lifetime bookkeeping.
#[repr(C)]
struct IpAdapterUnicastAddress {
    _length: u32,
    _flags: u32,
    next: *const IpAdapterUnicastAddress,
    address: SocketAddress,
}

/// `IP_ADAPTER_ADDRESSES`, laid out only as far as the fields we read.
///
/// The MAC is 8 *bytes* here, not eight `WCHAR`s: the SDK header spells it
/// `WCHAR PhysicalAddress[8]`, but every field after it then lands 8 bytes
/// late - `IfType` reads as the interface index and the two link speeds come
/// out as noise. Probed against this machine's six adapters; the offsets below
/// are the ones that produce real `IfType` / `OperStatus` / link values.
#[repr(C)]
struct IpAdapterAddresses {
    _length: u32,
    _if_index: u32,
    next: *const IpAdapterAddresses,
    _adapter_name: *const u8,
    first_unicast: *const IpAdapterUnicastAddress,
    _first_anycast: *const c_void,
    _first_multicast: *const c_void,
    _first_dns: *const c_void,
    _dns_suffix: *const u16,
    _description: *const u16,
    friendly_name: *const u16,
    _physical_address: [u8; 8],
    _physical_address_length: u32,
    _flags: u32,
    _mtu: u32,
    if_type: u32,
    oper_status: u32,
    _ipv6_if_index: u32,
    _zone_indices: [u32; 16],
    _first_prefix: *const c_void,
    transmit_link_speed: u64,
    receive_link_speed: u64,
}

extern "system" {
    fn GetAdaptersAddresses(
        family: u32,
        flags: u32,
        reserved: *mut c_void,
        adapters: *mut IpAdapterAddresses,
        size: *mut u32,
    ) -> u32;
}

/// A network adapter, as the second line of the network column may want it.
#[derive(Debug, Clone)]
pub struct AdapterInfo {
    /// The friendly name Windows shows in the Control Panel. sysinfo reports
    /// the same string, which is how a sample finds its adapter again.
    pub name: String,
    /// First IPv4 address, empty when the link is down or unconfigured.
    pub ipv4: String,
    /// Negotiated link speed in Mbit/s, 0 when the driver will not say.
    pub link_mbps: u64,
}

/// SAFETY: `ptr` must be a NUL terminated UTF-16 string, or null.
unsafe fn wide_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

/// Every adapter that is currently up, IPv4 addresses included.
pub fn adapters() -> Vec<AdapterInfo> {
    const AF_INET: u32 = 2;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
    const IF_OPER_STATUS_UP: u32 = 1;

    let mut out = Vec::new();
    // SAFETY: `buf` is a plain byte block of `size` bytes; the API writes a
    // linked list of `IpAdapterAddresses` into it, and the pointers we follow
    // stay inside that block.
    unsafe {
        let mut size: u32 = 16 * 1024;
        for _ in 0..3 {
            let mut buf = vec![0u8; size as usize];
            let status = GetAdaptersAddresses(
                AF_INET,
                0,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut IpAdapterAddresses,
                &mut size,
            );
            if status == ERROR_BUFFER_OVERFLOW {
                continue;
            }
            if status != 0 {
                return out;
            }

            let mut node: *const IpAdapterAddresses = buf.as_ptr() as *const _;
            while !node.is_null() {
                let a = &*node;
                // A loopback address or a link that is down says nothing about
                // the machine's connection, and would only crowd the list.
                if a.oper_status == IF_OPER_STATUS_UP && a.if_type != IF_TYPE_SOFTWARE_LOOPBACK {
                    out.push(AdapterInfo {
                        name: wide_to_string(a.friendly_name),
                        ipv4: first_ipv4(a.first_unicast),
                        link_mbps: (a.transmit_link_speed / 1_000_000)
                            .max(a.receive_link_speed / 1_000_000),
                    });
                }
                node = a.next;
            }
            break;
        }
    }
    out
}

/// `AF_INET` as it appears in the `sa_family` word of a `sockaddr_in`.
const AF_INET_SOCKADDR: u16 = 2;

/// Walk a unicast address list for the first IPv4 entry.
fn first_ipv4(head: *const IpAdapterUnicastAddress) -> String {
    let mut node = head;
    while !node.is_null() {
        // SAFETY: the list belongs to the buffer `adapters` owns, and every
        // entry is followed by a sockaddr whose length we check before reading.
        unsafe {
            let entry = &*node;
            let sa = &entry.address;
            if sa.len >= 16 && !sa.sockaddr.is_null() {
                let family = u16::from_le_bytes([*sa.sockaddr, *sa.sockaddr.add(1)]);
                if family == AF_INET_SOCKADDR {
                    let octets = [
                        *sa.sockaddr.add(4),
                        *sa.sockaddr.add(5),
                        *sa.sockaddr.add(6),
                        *sa.sockaddr.add(7),
                    ];
                    return format!(
                        "{}.{}.{}.{}",
                        octets[0], octets[1], octets[2], octets[3]
                    );
                }
            }
            node = entry.next;
        }
    }
    String::new()
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
    fn CloseHandle(handle: *mut c_void) -> i32;
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
        CloseHandle(handle as *mut c_void);
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
