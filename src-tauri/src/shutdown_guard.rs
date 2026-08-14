// Shutdown guard (Windows). While a Climax session is active, register a
// "shutdown block reason" on the tracker window so that trying to shut down /
// log off / restart shows "Climax is tracking a session" and asks you to
// confirm — the unsaved-changes prompt — instead of silently killing an
// in-progress session. The reason is cleared the instant no session is active.
//
// Best-effort and Windows-only: a background poll mirrors the active-session
// state onto the block reason (catches every start/stop path — button, hotkey,
// auto-discard, capture-accept). Off Windows it's a no-op.

use std::time::Duration;

use tauri::{AppHandle, Runtime};

use crate::remote_session;
use crate::state::AppState;

const POLL_SECS: u64 = 5;
// Only the Windows arm resolves a window to hang the block reason on, so off
// Windows this is dead and warns. Gated with the code that uses it.
#[cfg(windows)]
const WINDOW_LABEL: &str = "tracker";

pub fn spawn<R: Runtime>(app: AppHandle<R>, state: AppState) {
    tauri::async_runtime::spawn(async move {
        // The guard protects whatever owns the session. We can only block THIS
        // machine's shutdown, so it's meaningful only when shutting this PC down
        // would take the session's owner with it = the server is LOCAL (the
        // in-process backend, or a client pointed at localhost). A remote (NAS/
        // LAN) server's session survives this PC's shutdown, so don't guard - and
        // the headless server can't pop its own block anyway (no HWND). See
        // CLAUDE.md "SHUTDOWN GUARD semantics in the split". This is a boot
        // snapshot (the server address can't change without a relaunch).
        if !remote_session::server_is_local(&state) {
            tracing::info!(
                "shutdown guard: server is remote - not blocking this machine's shutdown"
            );
            return;
        }
        let mut blocked = false;
        loop {
            tokio::time::sleep(Duration::from_secs(POLL_SECS)).await;
            // In client mode this reads the LOCAL server's active-session state
            // over /rpc; an unreachable server reads as "no session" -> the block
            // is cleared (the safe default: never wedge shutdown on a stale yes).
            let active = remote_session::active_id(&state).await.is_some();
            if active != blocked && set_shutdown_block(&app, active) {
                blocked = active;
            }
        }
    });
}

/// Register (on=true) or clear (on=false) the shutdown block reason on the
/// tracker window. Returns true if the call was attempted (so the poll only
/// flips its cached state when the window/HWND was actually reachable).
#[cfg(windows)]
fn set_shutdown_block<R: Runtime>(app: &AppHandle<R>, on: bool) -> bool {
    use tauri::Manager;
    use windows::core::PCWSTR;
    use windows::Win32::System::Shutdown::{
        ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy,
    };

    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return false;
    };
    // Tauri's hwnd() may come from a different `windows` crate version than the
    // one we link for ShutdownBlockReason*, so bridge through the raw pointer.
    // h.0 is already `*mut c_void` for the currently-linked windows-crate
    // versions; keep the cast as a defensive bridge for the version skew the
    // comment above describes (a future skew would need it back).
    #[allow(clippy::unnecessary_cast)]
    let hwnd = match win.hwnd() {
        Ok(h) => windows::Win32::Foundation::HWND(h.0 as *mut core::ffi::c_void),
        Err(_) => return false,
    };
    unsafe {
        if on {
            // Wide, NUL-terminated reason string.
            let reason: Vec<u16> = "Climax is tracking a session."
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let _ = ShutdownBlockReasonCreate(hwnd, PCWSTR(reason.as_ptr()));
        } else {
            let _ = ShutdownBlockReasonDestroy(hwnd);
        }
    }
    true
}

/// No equivalent of `ShutdownBlockReasonCreate` exists off Windows, so nothing
/// is ever armed here.
///
/// Returns FALSE, i.e. "the call was not made". This used to return `true`,
/// which made the poll cache `blocked = active` and believe a shutdown block
/// was in place when none was - the app reporting success for something it had
/// not done. Cost is nil: the caller short-circuits on `active != blocked`
/// first, so this is only reached while a session is live, and it does nothing.
#[cfg(not(windows))]
fn set_shutdown_block<R: Runtime>(_app: &AppHandle<R>, _on: bool) -> bool {
    false
}
