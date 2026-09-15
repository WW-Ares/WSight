//! The half of a snapshot that survives a reboot, persisted so the monitor
//! panel can open on real hardware facts instead of grey placeholders.
//!
//! Only slow-moving values live here: how many cores, how much RAM is fitted,
//! which volumes exist and how big they are, which physical drive each volume
//! sits on. Everything that is a *rate* or a momentary load - CPU %, free
//! memory, network and disk throughput - is deliberately absent. The panel
//! renders those as 0 until the collector has actually measured them, because
//! a cached speed would just be a number from the last session wearing this
//! session's clothes.

use serde::{Deserialize, Serialize};

use crate::collector::Snapshot;

/// A volume, minus its throughput.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskStatic {
    pub letter: String,
    pub name: String,
    pub mount_point: String,
    pub total: u64,
    /// Kept on purpose: a volume's used space moves slowly enough that the
    /// last figure is honest, whereas an empty bar would read as "brand new
    /// empty disk" for the second before the first sample lands.
    pub used: u64,
    pub percent: f32,
    pub is_ssd: Option<bool>,
    pub device: u32,
}

/// A physical drive, minus its throughput: `SSD C+D` and `HDD E` are already
/// the truth the moment the app starts, only their rates are not.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveStatic {
    pub device: u32,
    pub is_ssd: Option<bool>,
    pub letters: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuStatic {
    pub name: String,
    pub mem_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorCache {
    pub saved_at_ms: i64,
    pub cpu_brand: String,
    pub cpu_cores: usize,
    /// Rated clock. The live one is a measurement, so it is never stored.
    pub cpu_freq_mhz: u64,
    pub mem_total: u64,
    pub mem_swap_total: u64,
    pub mem_speed_mhz: u32,
    pub gpu: Option<GpuStatic>,
    /// The adapter that carried traffic last session, so the column already
    /// has its heading while the rates are still 0.
    pub net_name: Option<String>,
    pub disks: Vec<DiskStatic>,
    pub drives: Vec<DriveStatic>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Reduce a sample to the part worth keeping.
pub fn from_snapshot(snap: &Snapshot) -> MonitorCache {
    MonitorCache {
        saved_at_ms: now_ms(),
        cpu_brand: snap.cpu.brand.clone(),
        cpu_cores: snap.cpu.cores,
        cpu_freq_mhz: snap.cpu.freq_mhz,
        mem_total: snap.mem.total,
        mem_swap_total: snap.mem.swap_total,
        mem_speed_mhz: snap.mem.speed_mhz,
        gpu: snap.gpu.as_ref().map(|g| GpuStatic {
            name: g.name.clone(),
            mem_total: g.mem_total,
        }),
        net_name: snap.nets.first().map(|n| n.name.clone()),
        disks: snap
            .disks
            .iter()
            .map(|d| DiskStatic {
                letter: d.letter.clone(),
                name: d.name.clone(),
                mount_point: d.mount_point.clone(),
                total: d.total,
                used: d.used,
                percent: d.percent,
                is_ssd: d.is_ssd,
                device: d.device,
            })
            .collect(),
        drives: snap
            .drives
            .iter()
            .map(|d| DriveStatic {
                device: d.device,
                is_ssd: d.is_ssd,
                letters: d.letters.clone(),
                label: d.label.clone(),
            })
            .collect(),
    }
}

/// Write the cache. A failure here must never disturb the sample that
/// triggered it - this is an optimisation, so the error is logged and dropped.
pub fn save(path: &std::path::Path, snap: &Snapshot) {
    let cache = from_snapshot(snap);
    match serde_json::to_string(&cache) {
        Ok(text) => {
            if let Err(e) = std::fs::write(path, text) {
                eprintln!("[monitor] cache write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("[monitor] cache encode: {e}"),
    }
}

/// Read it back. Missing file, half-written file, an older schema - all of
/// them simply mean "no cache", which the panel treats as a first run and
/// answers with plain placeholders.
pub fn load(path: &std::path::Path) -> Option<MonitorCache> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<MonitorCache>(&text).ok()
}
