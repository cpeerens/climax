// AFK / idle detection — pre-emptive, Toggl-style.
//
// While a session is `active`, a background task polls the OS for the user's
// last input time. The moment the gap crosses the configured threshold, we
//   1. store an `IdlePayload` on AppState so the prompt window can read it
//   2. emit an `idle_detected` Tauri event (useful for already-open listeners)
//   3. show + focus the floating idle-prompt window (alwaysOnTop, centered)
//
// We never auto-close the modal: the user has to click one of
//   - Keep                — idle time counts as session time, dismiss
//   - Discard & Continue  — insert a 'idle' pause for the gap [started, now],
//                            session keeps running
//   - Discard & End       — end the session at `idle_started_at`
// The resolve commands hide the prompt window and clear `pending_idle`.
//
// If the user comes back BEFORE picking an option (idle_ms drops below the
// threshold), the loop silently resets its in-memory flag so the next idle
// stretch fires a fresh prompt. The pending modal isn't dismissed — the user
// still needs to resolve the previous one.
//
// Sleep / hibernate / process-kill recovery is a separate concern (the
// crash-recovery roadmap item). This loop intentionally cares only about
// "the keyboard / mouse hasn't been touched".

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::db::now_ms;
use crate::models::IdlePayload;
use crate::remote_session;
use crate::state::AppState;

/// How often we poll. 30s is a sensible balance: small enough that the prompt
/// appears within a tick of crossing the threshold, large enough that we don't
/// spam syscalls.
const POLL_INTERVAL_SECS: u64 = 30;

/// Event name emitted to the frontend when an idle stretch is detected. The
/// idle-prompt window can listen for this if it's already mounted; but it
/// also queries `idle_pending_get` on mount, so we don't rely on the event
/// being received.
const IDLE_DETECTED_EVENT: &str = "idle_detected";

/// Window we show + lift above other apps when an idle prompt fires.
/// Currently the tracker — it hosts the modal in-page. (We tried a dedicated
/// floating window but multi-window IPC scoping in Tauri 2 made it brittle.)
const PROMPT_WINDOW_LABEL: &str = "tracker";

#[derive(Default)]
struct PresenceState {
    /// True while we've already fired the prompt for the current idle stretch.
    /// Prevents re-firing on every tick while the user is still away.
    is_idle: bool,
    /// Wall-clock ms of the user's last input *before* going idle. Used as
    /// the "session end time" candidate if the user later picks Discard & End.
    idle_started_at_ms: i64,
    /// Active session id we last saw, so we can reset our flags when the
    /// session changes (user manually started/stopped between ticks).
    last_session_id: Option<i64>,
}

pub fn spawn<R: Runtime>(app: AppHandle<R>, state: AppState) {
    tauri::async_runtime::spawn(async move {
        let mut local = PresenceState::default();
        loop {
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
            if let Err(e) = tick(&app, &state, &mut local).await {
                tracing::warn!("presence tick failed: {:#}", e);
            }
        }
    });
}

async fn tick<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    local: &mut PresenceState,
) -> anyhow::Result<()> {
    // In client mode this reads the idle setting from the SERVER (the user
    // configures it there; the local cache is empty), so the loop honours the
    // real enabled flag + threshold. Locally it reads the in-process settings.
    let setting = remote_session::idle_setting(state).await;
    if !setting.enabled {
        // If the user disabled the feature mid-idle, clear our flag so the
        // next enable will fire a fresh prompt at the next threshold cross.
        local.is_idle = false;
        return Ok(());
    }
    let threshold_ms: i64 = (setting.threshold_minutes as i64) * 60 * 1000;

    // Only watch ACTIVE sessions. Paused / ended sessions don't get prompted.
    // `active()` returns the active-OR-paused session (the server's in client
    // mode, the local manager otherwise), so filter to 'active'.
    let session_id: Option<i64> = remote_session::active(state)
        .await
        .filter(|s| s.status == "active")
        .map(|s| s.id);

    if session_id != local.last_session_id {
        local.is_idle = false;
        local.last_session_id = session_id;
    }
    let Some(sid) = session_id else { return Ok(()) };

    let Some(idle_ms) = read_idle_ms() else {
        return Ok(());
    };

    let now = now_ms();
    // Approximate wall-clock time of the user's last input.
    let last_input_at = now - idle_ms;

    if idle_ms >= threshold_ms {
        if !local.is_idle {
            local.is_idle = true;
            local.idle_started_at_ms = last_input_at;
            let payload = IdlePayload {
                session_id: sid,
                idle_started_at: local.idle_started_at_ms,
            };

            tracing::info!(
                "presence: idle threshold crossed (threshold {}m, idle_for={}s, idle_started_at={})",
                setting.threshold_minutes,
                idle_ms / 1000,
                local.idle_started_at_ms
            );

            // Stash on AppState so the prompt window can read it on mount.
            {
                let mut slot = state.pending_idle.write().await;
                *slot = Some(payload.clone());
            }

            if let Err(e) = app.emit(IDLE_DETECTED_EVENT, &payload) {
                tracing::warn!("emit idle_detected failed: {:#}", e);
            }

            show_prompt_window(app);
        }
    } else if local.is_idle {
        // User came back BEFORE resolving the current prompt. We silently
        // reset so the next idle stretch fires a fresh prompt. The existing
        // prompt window stays open — the user still has to act on it.
        local.is_idle = false;
        tracing::info!(
            "presence: user input returned while prompt still open (was idle, now active)"
        );
    }

    Ok(())
}

fn show_prompt_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window(PROMPT_WINDOW_LABEL) {
        // Float above all apps while the prompt is up. The resolve commands
        // (idle_keep / discard_continue / discard_end) set this back to false
        // after the user picks an option.
        let _ = win.set_always_on_top(true);
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    } else {
        tracing::warn!(
            "presence: prompt host window '{}' not found - check tauri.conf.json",
            PROMPT_WINDOW_LABEL
        );
    }
}

/// Returns milliseconds since the last keyboard / mouse input, or None if
/// the platform doesn't support it or the syscall failed.
#[cfg(windows)]
fn read_idle_ms() -> Option<i64> {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    let ok = unsafe { GetLastInputInfo(&mut info) };
    if !ok.as_bool() {
        return None;
    }
    let now_ticks = unsafe { GetTickCount() };
    let idle_ms = now_ticks.wrapping_sub(info.dwTime);
    Some(idle_ms as i64)
}

/// macOS counterpart of `GetLastInputInfo`. CoreGraphics reports seconds since
/// the last input event, so this is the same measurement expressed differently.
///
/// `kCGEventSourceStateHIDSystemState` (1) is deliberate: it counts real
/// HARDWARE input, matching what the Windows arm measures. The combined-session
/// source (0) also counts SYNTHETIC events, so anything posting events
/// programmatically would look like a present user and suppress the idle prompt
/// forever. `kCGAnyInputEventType` is `~0`, i.e. every input type.
///
/// No Accessibility or Input Monitoring permission is needed - this reads a
/// timestamp, it does not observe or tap the events themselves.
#[cfg(target_os = "macos")]
fn read_idle_ms() -> Option<i64> {
    const K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE: u32 = 1;
    const K_CG_ANY_INPUT_EVENT_TYPE: u32 = !0;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state_id: u32, event_type: u32) -> f64;
    }

    let secs = unsafe {
        CGEventSourceSecondsSinceLastEventType(
            K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE,
            K_CG_ANY_INPUT_EVENT_TYPE,
        )
    };
    // Guard the cast: a NaN or negative would wrap to a nonsense idle time and
    // could fire the prompt instantly. Treat it as "can't tell", like Windows
    // does when the syscall fails.
    if secs.is_finite() && secs >= 0.0 {
        Some((secs * 1000.0) as i64)
    } else {
        None
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn read_idle_ms() -> Option<i64> {
    None
}
