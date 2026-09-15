//! A one-line-per-event launch log, kept beside the config.
//!
//! Auto-start bugs are the worst kind to diagnose: the only witness is a
//! window that appeared before the user was at the keyboard, and by the time
//! they look, the evidence is gone. So every start writes a few facts -
//! whether this is the auto-start launch, whether another instance was already
//! running, which config file was read and whether it had a city - into
//! `launch.log`. When the user reports "it opened the settings window again"
//! the file answers why without a single reproduction attempt.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config;

/// Beyond this the file is trimmed: the newest lines are the interesting ones.
const MAX_BYTES: u64 = 128 * 1024;
/// What survives a trim.
const KEEP_BYTES: usize = 32 * 1024;

fn path() -> PathBuf {
    config::data_dir().join("launch.log")
}

fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Local wall clock without pulling in a timezone crate: the offset is
    // fixed at +08:00, which is where this app is used.
    let total = secs as i64 + 8 * 3600;
    let days = total.div_euclid(86400);
    let time = total.rem_euclid(86400);
    let (h, m, s) = (time / 3600, (time % 3600) / 60, time % 60);
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, m, s)
}

/// Append one line. Logging must never keep the app from starting, so every
/// failure is swallowed.
pub fn log(line: &str) {
    let path = path();
    let _ = (|| -> std::io::Result<()> {
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() > MAX_BYTES {
                let text = fs::read_to_string(&path)?;
                let keep = &text[text.len().saturating_sub(KEEP_BYTES)..];
                fs::write(&path, keep)?;
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(file, "{} {}", stamp(), line)
    })();
}
