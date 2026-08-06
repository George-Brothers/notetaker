//! The tray: Echo in the corner, state at a glance, essentials on right-click.
//!
//! The frontend drives state via `set_tray_status` (it already polls capture
//! status for the record bar, so the elapsed time in the status line rides the
//! same poll). Open/Quit act natively; record/pause/stop/Settings are
//! forwarded to the webview, which owns the capture flow.
//!
//! One static menu whose items change text and enablement, never a rebuild:
//! `set_state` arrives once a second while recording, and swapping the menu
//! out from under a pointer that has it open closes it.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime,
};
use tauri_plugin_positioner::{Position, WindowExt as PositionerWindowExt};

use crate::windowing;

pub const TRAY_ID: &str = "main-tray";

/// Every item whose label or enablement follows capture state.
pub struct TrayHandles<R: Runtime> {
    status: MenuItem<R>,
    record_meeting: MenuItem<R>,
    record_in_person: MenuItem<R>,
    pause: MenuItem<R>,
    stop: MenuItem<R>,
    highlight: MenuItem<R>,
}

fn icon_bytes(state: &str) -> &'static [u8] {
    match state {
        "recording" => include_bytes!("../icons/tray/recording.png"),
        "paused" => include_bytes!("../icons/tray/paused.png"),
        _ => include_bytes!("../icons/tray/idle.png"),
    }
}

#[cfg(target_os = "windows")]
fn windows_uses_light_theme() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        .and_then(|key| key.get_value::<u32, _>("SystemUsesLightTheme"))
        .map(|value| value != 0)
        .unwrap_or(true)
}

fn icon_image(state: &str) -> tauri::Result<Image<'static>> {
    let source = Image::from_bytes(icon_bytes(state))?;

    #[cfg(target_os = "windows")]
    if windows_uses_light_theme() {
        // The source tray artwork is optimized for a dark taskbar. Windows
        // exposes its taskbar preference through this registry value; invert
        // only near-gray pixels for the light variant so the red/yellow state
        // dots retain their meaning. This keeps the two variants in one
        // checked-in asset set rather than shipping a second hand-maintained
        // copy that can drift from the recording state artwork.
        let mut rgba = source.rgba().to_vec();
        for pixel in rgba.chunks_exact_mut(4) {
            let max = pixel[..3].iter().copied().max().unwrap_or_default();
            let min = pixel[..3].iter().copied().min().unwrap_or_default();
            if pixel[3] != 0 && max.saturating_sub(min) <= 16 {
                pixel[..3]
                    .iter_mut()
                    .for_each(|channel| *channel = 255 - *channel);
            }
        }
        return Ok(Image::new_owned(rgba, source.width(), source.height()));
    }

    Ok(source.to_owned())
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn show_panel<R: Runtime>(app: &AppHandle<R>) {
    let Some(panel) = app.get_webview_window("tray-panel") else {
        return;
    };

    #[cfg(target_os = "macos")]
    let position = Position::TrayCenter;
    #[cfg(target_os = "windows")]
    let position = Position::TrayBottomCenter;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let position = Position::TrayCenter;

    // The positioner callback must receive every tray event, and the move
    // happens before show so the panel never flashes at the top-left corner.
    let _ = panel.move_window(position);
    windowing::show_panel(app, "tray-panel");
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<TrayIcon<R>> {
    // Disabled: it is a reading, not a control. Menus have no other way to
    // show a line of state.
    let status = MenuItem::with_id(app, "tray-status", "Not recording", false, None::<&str>)?;
    let record_meeting = MenuItem::with_id(
        app,
        "tray-record-meeting",
        "Record meeting",
        true,
        None::<&str>,
    )?;
    let record_in_person = MenuItem::with_id(
        app,
        "tray-record-in-person",
        "Record in person",
        true,
        None::<&str>,
    )?;
    let pause = MenuItem::with_id(app, "tray-pause", "Pause", false, None::<&str>)?;
    let stop = MenuItem::with_id(app, "tray-stop", "Stop recording", false, None::<&str>)?;
    let highlight = MenuItem::with_id(
        app,
        "tray-highlight",
        "Star highlight moment",
        false,
        None::<&str>,
    )?;
    let copy_last_transcript = MenuItem::with_id(
        app,
        "tray-copy-last-transcript",
        "Copy last transcript",
        true,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "tray-open", "Open Notetaker", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "tray-settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit Notetaker", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &record_meeting,
            &record_in_person,
            &pause,
            &stop,
            &PredefinedMenuItem::separator(app)?,
            &highlight,
            &copy_last_transcript,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &quit,
        ],
    )?;
    app.manage(TrayHandles {
        status,
        record_meeting,
        record_in_person,
        pause,
        stop,
        highlight,
    });

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon_image("idle")?)
        .icon_as_template(true)
        .tooltip("Notetaker")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            // Positioner must see every event. Calling this only for the
            // left-click branch makes the native tray resolve coordinates
            // from the top-left on macOS and Windows.
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
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
                show_panel(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            // Recording actions deliberately do NOT show the window: the point
            // of running them from the tray is not having to open the app.
            // The webview stays the owner of the capture flow either way.
            "tray-record-meeting" => {
                let _ = app.emit("tray-record", "meeting");
            }
            "tray-record-in-person" => {
                let _ = app.emit("tray-record", "in_person");
            }
            "tray-pause" => {
                let _ = app.emit("tray-pause-resume", ());
            }
            "tray-stop" => {
                let _ = app.emit("tray-stop", ());
            }
            "tray-highlight" => {
                let _ = app.emit("tray-highlight", ());
            }
            "tray-copy-last-transcript" => {
                let _ = app.emit("tray-copy-last-transcript", ());
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

/// Called by the frontend whenever capture state changes, and once a second
/// while recording so the status line's elapsed time keeps up. `status_line`
/// arrives ready-made ("Recording — 12:34") because the frontend already
/// formats elapsed time for the record bar; Rust stays dumb about wording.
pub fn set_state<R: Runtime>(app: &AppHandle<R>, state: &str, status_line: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(icon) = icon_image(state) {
            let _ = tray.set_icon(Some(icon));
        }
        let _ = tray.set_icon_as_template(state != "recording");
        let _ = tray.set_tooltip(Some(match state {
            "recording" => "Notetaker — recording",
            "paused" => "Notetaker — paused",
            _ => "Notetaker",
        }));
    }
    if let Some(handles) = app.try_state::<TrayHandles<R>>() {
        let capturing = state == "recording" || state == "paused";
        let _ = handles.status.set_text(status_line);
        let _ = handles.record_meeting.set_enabled(!capturing);
        let _ = handles.record_in_person.set_enabled(!capturing);
        let _ = handles.pause.set_enabled(capturing);
        let _ = handles
            .pause
            .set_text(if state == "paused" { "Resume" } else { "Pause" });
        let _ = handles.stop.set_enabled(capturing);
        let _ = handles.highlight.set_enabled(capturing);
    }
}
