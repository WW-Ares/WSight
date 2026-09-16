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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    // The three ways this process can come into being. `--handover` is the
    // self-updater's, and it is the only one that must not refuse to start when
    // another WSight is still running - see below.
    //
    // The two flags are independent on purpose: a launch-time update hands over
    // at logon, so the build that takes over is a handover *and* an auto-start,
    // and it needs the delay below for the same reason any logon launch does.
    // Making `--startup` conditional on `--handover` being absent would have new
    // loads in exactly the window the delay exists to avoid.
    let handover = has("--handover");
    let startup = has("--startup");

    // Set by the updater together with `--handover`: the build it stepped
    // aside, which is ours to decide the fate of once its process is gone.
    let replaced = args
        .iter()
        .position(|a| a == "--replaced")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_default();

    wsight_lib::log_launch(match (handover, startup) {
        (true, true) => "launch (handover, auto-start)",
        (true, false) => "launch (handover)",
        (false, true) => "launch (auto-start)",
        _ => "launch (manual)",
    });

    if handover {
        // The build that started us has already renamed itself to
        // `WSight.exe.old-<version>` and is on its way out, but it still holds
        // the single-instance mutex - it cannot not hold it, that is what stops
        // the logon double-launch. So we wait for the mutex to disappear rather
        // than exiting on sight, which is also why no helper exe is needed.
        if !wsight_lib::await_instance() {
            wsight_lib::log_launch("handover: previous instance still running - exiting");
            return;
        }
        // Safe now: the old process is dead, so the file is no longer locked.
        wsight_lib::clean_up_replaced(&replaced);
    } else {
        // Checked before the delay: a second copy started five seconds from now
        // is just as redundant as one started right away.
        if !wsight_lib::claim_instance() {
            wsight_lib::log_launch("another instance already running - exiting");
            return;
        }
        // A verified update is already sitting on disk. Install it now, while
        // there is no window on screen and nothing to lose, and let the new
        // build take over - the user's next launch is simply the new version.
        //
        // The mutex is taken first on purpose: renaming our own exe out from
        // under a live instance is not something to do casually, and the
        // handover the new build performs assumes it is the only one waiting.
        if wsight_lib::auto_apply_staged(startup) {
            return;
        }
    }

    if startup {
        std::thread::sleep(Duration::from_secs(STARTUP_DELAY_SECS));
    }

    wsight_lib::run();
}
