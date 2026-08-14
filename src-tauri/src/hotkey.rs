// Global hotkey registration.
// Default: Ctrl+Shift+L toggles session start/stop AND opens the tracker
// widget so you can see what just happened.

use anyhow::Result;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::state::AppState;

pub fn register<R: Runtime>(app: &AppHandle<R>, accelerator: &str) -> Result<()> {
    let shortcut: Shortcut = accelerator.parse()?;
    let gs = app.global_shortcut();
    let app_for_handler = app.clone();

    gs.on_shortcut(shortcut, move |_app_handle, _shortcut, event| {
        if event.state() != tauri_plugin_global_shortcut::ShortcutState::Pressed {
            return;
        }
        let app = app_for_handler.clone();
        tauri::async_runtime::spawn(async move {
            let Some(state) = app.try_state::<AppState>() else { return };
            // In client mode this reads/starts the SERVER's session over /rpc.
            // Hotkey starts a session if none, but NEVER stops one. Stopping is
            // a deliberate action - use the Stop button in the tracker. Use the
            // error-surfacing read so a transient unreachable server doesn't read
            // as "no session" and make us start a duplicate (which ENDs the real
            // one - start isn't idempotent); only auto-start on a CONFIRMED-empty
            // read, and never stop here.
            match crate::remote_session::active_checked(&state).await {
                Ok(None) => {
                    let _ = crate::remote_session::start(&state).await;
                }
                Ok(Some(_)) => {}
                Err(e) => {
                    tracing::warn!("hotkey: couldn't read session state, not starting: {:#}", e);
                }
            }
            // Always show + focus the tracker so the user sees the current state.
            // show_solo also hides the dashboard if it's currently visible
            // (tracker + dashboard are mutually exclusive surfaces).
            crate::commands::show_solo(&app, "tracker");
        });
    })?;

    tracing::info!("global hotkey registered: {}", accelerator);
    Ok(())
}

/// Unregister the global hotkey (used when the user disables it in settings).
pub fn unregister<R: Runtime>(app: &AppHandle<R>, accelerator: &str) -> Result<()> {
    let shortcut: Shortcut = accelerator.parse()?;
    app.global_shortcut().unregister(shortcut)?;
    tracing::info!("global hotkey unregistered: {}", accelerator);
    Ok(())
}
