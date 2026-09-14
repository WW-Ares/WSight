//! Hardware sampling.
//!
//! Design notes — this is the whole point of the rewrite:
//!   * One long-lived `System` / `Networks` / `Disks` instance, refreshed in
//!     place. The original WidgetJS panel shelled out to PowerShell once per
//!     metric per tick (8~10 processes every 3 s), which is what pegged the CPU
//!     and hammered Winmgmt. Here everything comes from plain syscalls.
//!   * Rate metrics (CPU %, network throughput, disk throughput) are *deltas*,
//!     so they need a previous sample. sysinfo handles that internally via
//!     refresh().
//!   * GPU data comes from NVML (in-process, ~microseconds). When no NVIDIA
//!     GPU is present the field is simply `None` - never an error.
//!   * The three figures Windows does not hand out for free - the live CPU
//!     clock, the memory speed and the volume -> physical disk mapping - come
//!     from `native`, which is FFI only.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use nvml_wrapper::Nvml;
use serde::{Deserialize, Serialize};
use sysinfo::{
    CpuRefreshKind, Disks, DiskKind, MemoryRefreshKind, Networks, RefreshKind, System,
};

use crate::native;

/// Stand-in device number for a volume whose backing disk could not be
/// resolved. Such volumes are grouped together so the panel still shows one
/// aggregate row instead of nothing at all.
const UNKNOWN_DEVICE: u32 = u32::MAX;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CpuInfo {
    pub load: f32,
    pub cores: usize,
    pub per_core: Vec<f32>,
    /// The rated clock reported by the OS. It never changes, and is only used
    /// as the denominator for [`Self::freq_live_mhz`].
    pub freq_mhz: u64,
    /// Clock the CPU is actually running at, derived from the PDH
    /// "% Processor Performance" counter. 0 when the counter is unavailable,
    /// in which case the UI shows the rated clock on its own.
    pub freq_live_mhz: u64,
    pub brand: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemInfo {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub percent: f32,
    pub swap_total: u64,
    pub swap_used: u64,
    /// Installed DDR speed in MHz, 0 when the firmware table is unreadable.
    pub speed_mhz: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    /// 0..100, -1 when the driver does not report utilisation
    pub load: f32,
    pub mem_total: u64,
    pub mem_used: u64,
    pub temp_c: Option<f32>,
    /// 0..100, -1 when unknown
    pub fan_percent: f32,
    /// board power draw in watts, `None` when the driver does not expose it
    pub power_w: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetInfo {
    pub name: String,
    pub rx_sec: f64,
    pub tx_sec: f64,
    pub rx_total: u64,
    pub tx_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    /// Volume label from `GetVolumeInformationW`, e.g. "Win11" / "软件".
    /// Empty when the volume carries no label.
    pub name: String,
    pub mount_point: String,
    /// `"C:"` - always present: letterless partitions are filtered out, they
    /// are the EFI / recovery volumes and have nothing to show.
    pub letter: String,
    pub total: u64,
    pub used: u64,
    pub percent: f32,
    /// bytes per second since the previous sample, for this volume alone
    pub read_sec: f64,
    pub write_sec: f64,
    /// `Some(true)` = solid state, `Some(false)` = spinning rust,
    /// `None` = the bus would not say.
    pub is_ssd: Option<bool>,
    /// Physical disk behind this volume; volumes sharing a number live on the
    /// same drive. `u32::MAX` when it could not be resolved.
    pub device: u32,
}

/// One *physical* drive, i.e. everything sharing a disk number. Read/write
/// counters are per volume, so the figures here are the sum over the volumes
/// of that drive - which is what "the SSD's throughput" means to a user who
/// has one 932 GB NVMe split into C: and D:.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    pub device: u32,
    pub is_ssd: Option<bool>,
    /// Drive letters on this disk, smallest first, e.g. `"C/D"` or `"E"`.
    pub letters: String,
    /// Shortest label of its volumes, used as a caption fallback.
    pub label: String,
    pub read_sec: f64,
    pub write_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub ts: u64,
    pub uptime_sec: u64,
    pub cpu: CpuInfo,
    pub mem: MemInfo,
    pub gpu: Option<GpuInfo>,
    pub nets: Vec<NetInfo>,
    /// One entry per volume that has a drive letter, ordered by letter.
    pub disks: Vec<DiskInfo>,
    /// One entry per physical drive, ordered by its first letter.
    pub drives: Vec<DriveInfo>,
}

pub struct Collector {
    sys: System,
    networks: Networks,
    disks: Disks,
    nvml: Option<Nvml>,
    cpu_brand: String,
    cpu_rated_mhz: u64,
    clock: Option<native::CpuClock>,
    mem_speed_mhz: u32,
    /// mount point -> physical disk number. Resolved once: a disk cannot move
    /// to another slot while the app is running.
    volume_device: HashMap<String, u32>,
}

/// Interfaces that are never worth showing: kernel pseudo-adapters only.
const HARD_SKIP: &[&str] = &[
    "loopback",
    "bluetooth",
    "isatap",
    "teredo",
    "6to4",
    "wan miniport",
];

/// Adapters created by filter drivers / VPN / virtualisation software. They
/// often mirror the real NIC's counters (e.g. Huorong's "NDIS Filter" adapter
/// on this machine), which makes them look like the busiest interface and
/// hides the real one. Prefer to hide them, but fall back to them when there
/// is no physical candidate at all - otherwise the panel would show nothing.
const SOFT_SKIP: &[&str] = &[
    "huorong",
    "火绒",
    "hyper-v",
    "vethernet",
    "vmware",
    "virtualbox",
    "vbox",
    "virtual",
    "tap-",
    "wintun",
    "npcap",
    "pcap",
    "docker",
    "zerotier",
    "tailscale",
    "wireguard",
    "openvpn",
    "anyconnect",
    "sangfor",
    "深信服",
    "ndis filter",
    "wi-fi direct",
];

fn matches_any(name: &str, needles: &[&str]) -> bool {
    let lower = name.to_lowercase();
    needles.iter().any(|n| lower.contains(n))
}

fn is_hard_skipped(name: &str) -> bool {
    matches_any(name, HARD_SKIP)
}

fn is_soft_skipped(name: &str) -> bool {
    matches_any(name, SOFT_SKIP)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `"C:\"` -> `"C:"`. Anything else (a folder mount point, or a volume with no
/// letter at all) has no caption the panel could use, and is skipped.
fn drive_letter(mount_point: &str) -> Option<String> {
    let bytes = mount_point.as_bytes();
    if bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
    {
        Some(mount_point[..2].to_string())
    } else {
        None
    }
}

impl Collector {
    pub fn new() -> Self {
        // CPU and memory only. Now that the busiest-processes row is gone there
        // is no reason to walk the process table at all - `System::new_all()`
        // would do it once here for nothing.
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        sys.refresh_cpu_all();

        let mut networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();

        // Prime the counters so the first reported rate is meaningful instead
        // of being a lifetime average since boot.
        std::thread::sleep(Duration::from_millis(200));
        networks.refresh(true);
        sys.refresh_cpu_all();

        let brand = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();
        // sysinfo answers with the rated clock here; keep the largest reading
        // across cores so a single core reporting zero cannot win.
        let rated = sys
            .cpus()
            .iter()
            .map(|c| c.frequency() as u64)
            .max()
            .unwrap_or(0);

        // Resolve the volume -> physical disk mapping once. A disk cannot
        // change slots while the app runs, and the mapping costs one
        // CreateFileW + DeviceIoControl per volume.
        let mount_points: Vec<String> = disks
            .iter()
            .filter_map(|d| {
                let mount_point = d.mount_point().to_string_lossy().to_string();
                drive_letter(&mount_point).map(|_| mount_point)
            })
            .collect();
        let volume_device = native::device_numbers(mount_points.iter().map(String::as_str));

        Self {
            sys,
            networks,
            disks,
            nvml: Nvml::init().ok(),
            cpu_brand: brand,
            cpu_rated_mhz: rated,
            clock: native::CpuClock::new(),
            mem_speed_mhz: native::memory_speed_mhz(),
            volume_device,
        }
    }

    fn sample_gpu(&self) -> Option<GpuInfo> {
        let nvml = self.nvml.as_ref()?;
        let device = nvml.device_by_index(0).ok()?;

        let name = device.name().unwrap_or_else(|_| "GPU".to_string());

        let (load, mem_total, mem_used) = match device.utilization_rates() {
            Ok(u) => {
                let mem = device.memory_info().ok();
                (
                    u.gpu as f32,
                    mem.as_ref().map(|m| m.total).unwrap_or(0),
                    mem.as_ref().map(|m| m.used).unwrap_or(0),
                )
            }
            Err(_) => {
                let mem = device.memory_info().ok();
                (
                    -1.0,
                    mem.as_ref().map(|m| m.total).unwrap_or(0),
                    mem.as_ref().map(|m| m.used).unwrap_or(0),
                )
            }
        };

        let temp_c = device
            .temperature(TemperatureSensor::Gpu)
            .ok()
            .map(|t| t as f32);

        let fan_percent = device
            .fan_speed(0)
            .ok()
            .map(|f| f as f32)
            .unwrap_or(-1.0);

        // NVML reports milliwatts. Some boards (and every laptop dGPU in
        // power-saving) refuse the query, so this stays optional.
        let power_w = device
            .power_usage()
            .ok()
            .map(|mw| mw as f32 / 1000.0)
            .filter(|w| *w > 0.0);

        Some(GpuInfo {
            name,
            load,
            mem_total,
            mem_used,
            temp_c,
            fan_percent,
            power_w,
        })
    }

    /// Produce one sample. `elapsed_ms` is the wall time since the previous
    /// sample and is only used to turn byte counters into per-second rates.
    pub fn sample(&mut self, elapsed_ms: u64) -> Snapshot {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.disks.refresh(true);
        self.networks.refresh(true);

        let cpus = self.sys.cpus();
        let per_core: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();
        let load = self.sys.global_cpu_usage();

        // The rated clock is the only stable reading sysinfo gives, and it is
        // what the live figure is expressed against.
        let rated_now = cpus
            .iter()
            .map(|c| c.frequency() as u64)
            .max()
            .unwrap_or(0);
        if rated_now > 0 {
            self.cpu_rated_mhz = rated_now;
        }

        // Applied against the rated clock, the PDH ratio turns into the clock
        // the CPU is really running at - including boost, which is the whole
        // reason this exists.
        let freq_live_mhz = self
            .clock
            .as_ref()
            .and_then(|c| c.ratio())
            .map(|r| (self.cpu_rated_mhz as f64 * r).round() as u64)
            .unwrap_or(0);

        let total = self.sys.total_memory();
        let used = self.sys.used_memory();
        let free = self.sys.free_memory();
        let percent = if total > 0 {
            (used as f32 / total as f32) * 100.0
        } else {
            0.0
        };

        let cpu = CpuInfo {
            load,
            cores: cpus.len(),
            per_core,
            freq_mhz: self.cpu_rated_mhz,
            freq_live_mhz,
            brand: self.cpu_brand.clone(),
        };

        let mem = MemInfo {
            total,
            used,
            free,
            percent,
            swap_total: self.sys.total_swap(),
            swap_used: self.sys.used_swap(),
            speed_mhz: self.mem_speed_mhz,
        };

        let dt_sec = (elapsed_ms as f64 / 1000.0).max(0.05);
        let candidates: Vec<NetInfo> = self
            .networks
            .iter()
            .filter(|(name, _)| !is_hard_skipped(name))
            .map(|(name, data)| NetInfo {
                name: name.to_string(),
                rx_sec: data.received() as f64 / dt_sec,
                tx_sec: data.transmitted() as f64 / dt_sec,
                rx_total: data.total_received(),
                tx_total: data.total_transmitted(),
            })
            .collect();

        // Drop filter-driver / virtual adapters unless that would leave us with
        // nothing at all to show.
        let physical: Vec<NetInfo> = candidates
            .iter()
            .filter(|n| !is_soft_skipped(&n.name))
            .cloned()
            .collect();
        let mut nets = if physical.is_empty() {
            candidates
        } else {
            physical
        };

        // Ranked by *lifetime* traffic rather than by the current rate: the
        // adapter name is the column heading now, so it must not flip every
        // time another interface happens to burst for a second. sysinfo
        // already keeps link-down adapters out of the list, so this only
        // matters on a machine with two live links.
        nets.sort_by(|a, b| {
            (b.rx_total + b.tx_total)
                .cmp(&(a.rx_total + a.tx_total))
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut disks: Vec<DiskInfo> = self
            .disks
            .iter()
            // Volumes without a drive letter are the EFI / recovery partitions
            // - they have no caption and no business being on the panel.
            .filter_map(|d| {
                let mount_point = d.mount_point().to_string_lossy().to_string();
                let letter = drive_letter(&mount_point)?;
                let total = d.total_space();
                if total == 0 {
                    return None;
                }
                let avail = d.available_space();
                let used = total.saturating_sub(avail);
                // `DiskUsage::read_bytes` is "since the last refresh", which is
                // exactly one tick here.
                let usage = d.usage();
                let is_ssd = match d.kind() {
                    DiskKind::SSD => Some(true),
                    DiskKind::HDD => Some(false),
                    DiskKind::Unknown(_) => None,
                };
                Some(DiskInfo {
                    name: d.name().to_string_lossy().to_string(),
                    letter,
                    mount_point,
                    total,
                    used,
                    percent: (used as f32 / total as f32) * 100.0,
                    read_sec: usage.read_bytes as f64 / dt_sec,
                    write_sec: usage.written_bytes as f64 / dt_sec,
                    is_ssd,
                    device: UNKNOWN_DEVICE,
                })
            })
            .collect();

        // The OS hands the volumes over in enumeration order, which is stable
        // per boot but not across boots. Sort by drive letter so C: always
        // leads and a row does not jump around.
        disks.sort_by(|a, b| a.letter.cmp(&b.letter));
        for disk in &mut disks {
            disk.device = self
                .volume_device
                .get(&disk.mount_point)
                .copied()
                .unwrap_or(UNKNOWN_DEVICE);
        }

        let drives = build_drives(&disks);
        let gpu = self.sample_gpu();

        Snapshot {
            ts: now_millis(),
            uptime_sec: System::uptime(),
            cpu,
            mem,
            gpu,
            nets,
            disks,
            drives,
        }
    }
}

/// Fold the per-volume counters into per-drive figures.
///
/// The counters are per volume, so a drive's throughput is the sum over its
/// volumes; a drive is identified by the physical disk number the volumes
/// share, which is what makes C: and D: add up into "the SSD" while E: stays
/// on its own.
fn build_drives(disks: &[DiskInfo]) -> Vec<DriveInfo> {
    let mut groups: BTreeMap<u32, Vec<&DiskInfo>> = BTreeMap::new();
    for disk in disks {
        groups.entry(disk.device).or_default().push(disk);
    }

    let mut drives: Vec<DriveInfo> = groups
        .into_iter()
        .map(|(device, volumes)| {
            let mut letters: Vec<&str> = volumes.iter().map(|v| v.letter.as_str()).collect();
            letters.sort_unstable();
            DriveInfo {
                device,
                // Volumes of one drive always agree on the bus type, so the
                // first answer that is not "unknown" settles it.
                is_ssd: volumes.iter().find_map(|v| v.is_ssd),
                letters: letters.join("/"),
                label: volumes
                    .iter()
                    .map(|v| v.name.as_str())
                    .find(|n| !n.is_empty())
                    .unwrap_or("")
                    .to_string(),
                read_sec: volumes.iter().map(|v| v.read_sec).sum(),
                write_sec: volumes.iter().map(|v| v.write_sec).sum(),
            }
        })
        .collect();

    // Ordered by the first letter on the drive, so an SSD holding C: and D:
    // leads and the order does not shuffle between ticks.
    drives.sort_by(|a, b| a.letters.cmp(&b.letters));
    drives
}
