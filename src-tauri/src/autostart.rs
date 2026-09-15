//! Windows auto-start via the per-user `Run` registry key.
//!
//! Originally we shelled out to `reg.exe`, but that binary is blocked by some
//! security sandboxes and AV policies. Calling `RegSetValueExW` directly via
//! `winreg` avoids the child process, keeps quoting correct and never flashes
//! a console window.

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "WSight";

fn current_exe_string() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|e| format!("无法定位程序路径: {e}"))?;
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "程序路径含有非法字符".to_string())
}

/// Register the running exe under the `Run` key.
///
/// Re-writing on every start keeps the entry valid after the install folder is
/// moved or renamed.
pub fn enable() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu
        .create_subkey_with_flags(RUN_PATH, KEY_WRITE)
        .map_err(|e| format!("打开注册表 Run 项失败: {e}"))?;
    let exe = current_exe_string()?;
    // Quoted so a path with spaces is treated as the exe, not "exe + args".
    // `--startup` lets the new process know it was launched at logon: it then
    // waits a few seconds before creating any window, instead of racing every
    // other start-up entry for the disk and the WebView2 runtime.
    let value = format!("\"{exe}\" --startup");
    run.set_value(VALUE_NAME, &value)
        .map_err(|e| format!("写入开机启动项失败: {e}"))?;
    eprintln!("[autostart] enabled -> {value}");
    Ok(())
}

/// Remove the `Run` entry. A missing value is treated as success.
pub fn disable() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu
        .open_subkey_with_flags(RUN_PATH, KEY_WRITE)
        .map_err(|e| format!("打开注册表 Run 项失败: {e}"))?;
    match run.delete_value(VALUE_NAME) {
        Ok(()) => {
            eprintln!("[autostart] disabled");
            Ok(())
        }
        // Already gone - that is the end state we want.
        Err(ref e) if e.raw_os_error() == Some(2) => Ok(()),
        Err(e) => Err(format!("删除开机启动项失败: {e}")),
    }
}

pub fn is_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = match hkcu.open_subkey_with_flags(RUN_PATH, KEY_READ) {
        Ok(k) => k,
        Err(_) => return false,
    };
    run.get_value::<String, _>(VALUE_NAME).is_ok()
}

/// Apply the wanted state and report what the registry actually says.
pub fn set(enabled: bool) -> Result<bool, String> {
    if enabled {
        enable()?;
    } else {
        disable()?;
    }
    Ok(is_enabled())
}
