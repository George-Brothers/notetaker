//! The tray: Echo in the corner, state at a glance, essentials on right-click.
//!
//! The frontend drives state via `set_tray_status` (it already polls capture
//! status for the record bar). Open/Quit act natively; Start/Stop/Settings are
//! forwarded to the webview, which owns the capture flow.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime,
};

pub const TRAY_ID: &str = "main-tray";

/// The one menu item whose label changes with capture state.
pub struct TrayHandles<R: Runtime> {
    toggle: MenuItem<R>,
}

fn icon_bytes(state: &str) -> &'static [u8] {
    match state {
        "recording" => include_bytes!("../icons/tray/recording.png"),
        "paused" => include_bytes!("../icons/tray/paused.png"),
        _ => include_bytes!("../icons/tray/idle.png"),
    }
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<TrayIcon<R>> {
    let toggle = MenuItem::with_id(app, "tray-toggle", "Start recording", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "tray-open", "Open Notetaker", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "tray-settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit Notetaker", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &open, &sep, &settings, &quit])?;
    app.manage(TrayHandles { toggle });

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(icon_bytes("idle"))?)
        .tooltip("Notetaker")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            // Left-click only: right-click opens the menu and must not also
            // pop the window. Release only, too — Windows sends `Click` for
            // both the press and the release, so matching on the button alone
            // shows the window twice per click and four times on a double.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-toggle" => {
                // The webview decides start vs stop from its own state.
                show_main(app);
                let _ = app.emit("tray-toggle-recording", ());
            }
            "tray-open" => show_main(app),
            "tray-settings" => {
                show_main(app);
                let _ = app.emit("tray-open-settings", ());
            }
            // Quit asks the webview rather than exiting here. `app.exit(0)`
            // skips destructors, so quitting mid-recording dropped the last
            // unflushed buffer and left the take to be picked up as a crash
            // recovery on the next launch instead of a clean stop-and-save.
            // The frontend owns that decision — it is the same one the close
            // button already makes — so this shows the window (the guard
            // dialog has to be visible to be answered) and hands it over.
            // The webview calls `plugin:process|exit` when it is done.
            "tray-quit" => {
                show_main(app);
                let _ = app.emit("tray-quit-requested", ());
            }
            _ => {}
        })
        .build(app)
}

/// Called by the frontend whenever capture state changes.
pub fn set_state<R: Runtime>(app: &AppHandle<R>, state: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(icon) = Image::from_bytes(icon_bytes(state)) {
            let _ = tray.set_icon(Some(icon));
        }
        let _ = tray.set_tooltip(Some(match state {
            "recording" => "Notetaker — recording",
            "paused" => "Notetaker — paused",
            _ => "Notetaker",
        }));
    }
    if let Some(handles) = app.try_state::<TrayHandles<R>>() {
        let label = if state == "recording" || state == "paused" {
            "Stop recording"
        } else {
            "Start recording"
        };
        let _ = handles.toggle.set_text(label);
    }
}
