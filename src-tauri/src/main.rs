// Prevents an extra console window from appearing on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

/// Seconds the auto-start launch waits before building any window.
///
/// At logon the disk is still busy with every other start-up entry and the
/// WebView2 runtime has to be paged in; creating webviews inside that rush is
/// how a window ends up empty and unwilling to close. Waiting a few seconds
/// costs nothing - the widgets are not much use before the desktop has settled
/// anyway - and it takes the panic out of the race.
const STARTUP_DELAY_SECS: u64 = 5;

fn main() {
    let startup = std::env::args().skip(1).any(|a| a == "--startup");

    wsight_lib::log_launch(if startup {
        "launch (auto-start)"
    } else {
        "launch (manual)"
    });

    // Checked before the delay: a second copy started five seconds from now is
    // just as redundant as one started right away.
    if !wsight_lib::claim_instance() {
        wsight_lib::log_launch("another instance already running - exiting");
        return;
    }

    if startup {
        std::thread::sleep(Duration::from_secs(STARTUP_DELAY_SECS));
    }

    wsight_lib::run();
}
