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

/// The tail of `text` that survives a trim: the last `KEEP_BYTES`, moved
/// forward to the next line start so no half line (and no half character)
/// is left behind.
///
/// Cutting on a raw byte offset is how this module once killed the app. A
/// Chinese log line is three bytes per character, so `len - KEEP_BYTES` lands
/// inside a character sooner or later; slicing a `&str` there panics, and
/// release builds are `panic = "abort"`, so the panic is a hard exit. Because
/// the file stays over the limit, *every* launch then dies the same way —
/// 2026-11-14: 131125 bytes, offset 98357, in the middle of 「面」 in
/// 「桌面在前」. Walking bytes and splitting on `\n` (ASCII, never part of a
/// multi-byte sequence) cannot land mid-character at all.
fn tail_of(text: &[u8]) -> &[u8] {
    if text.len() <= KEEP_BYTES {
        return text;
    }
    let mut start = text.len() - KEEP_BYTES;
    // Forward to the next line start. If the offset already sits on one this
    // costs a single line, which is nothing against 32 KiB.
    while start < text.len() && text[start] != b'\n' {
        start += 1;
    }
    if start < text.len() {
        start += 1;
    }
    &text[start..]
}

/// Append one line. Logging must never keep the app from starting, so every
/// failure is swallowed.
pub fn log(line: &str) {
    let path = path();
    let _ = (|| -> std::io::Result<()> {
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() > MAX_BYTES {
                let text = fs::read(&path)?;
                fs::write(&path, tail_of(&text))?;
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(file, "{} {}", stamp(), line)
    })();
}

#[cfg(test)]
mod tests {
    use super::{tail_of, KEEP_BYTES};

    #[test]
    fn trim_keeps_whole_lines_and_whole_chars() {
        // 三种落点各来一遍：切点正好在行首、落在汉字中间、落在行中间。
        for pad in 0..12usize {
            let mut text = String::new();
            while text.len() < KEEP_BYTES * 2 + pad {
                text.push_str("2026-11-14 17:15:47 z: monitor 落回桌面层（前台=Chrome_Win）\n");
            }
            let total = text.len();
            let tail = tail_of(text.as_bytes());
            // 绝不能是半行：要么为空，要么以完整行开头、以换行结尾。
            assert!(tail.len() < total);
            if !tail.is_empty() {
                assert_eq!(*tail.last().unwrap(), b'\n');
                // 能当 UTF-8 读（说明没切在字符中间）。
                let s = std::str::from_utf8(tail).expect("tail 不是合法 UTF-8");
                assert!(s.starts_with("2026-"), "尾巴从半行开始: {:?}", &s[..30]);
            }
        }
    }

    #[test]
    fn trim_without_newline_yields_empty() {
        // 一整块没有换行的垃圾，剪完只剩空 —— 不能 panic。
        let text = vec![b'\xe9'; KEEP_BYTES * 2];
        assert!(tail_of(&text).is_empty());
    }

    #[test]
    fn trim_short_file_is_untouched() {
        let text = "2026-11-14 17:15:47 短日志\n".as_bytes();
        assert_eq!(tail_of(text), text);
    }
}
