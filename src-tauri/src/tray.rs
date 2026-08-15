//! Native status-item menu: Echo at a glance, essentials on right-click.
//!
//! The menu is intentionally a thin remote for the main webview. App.tsx owns
//! capture state and the stop-before-quit safety guard; this module owns the
//! native menu and emits intent events for those decisions. There is no tray
//! webview, timer, or second state machine.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    AppHandle, Emitter, Manager, Runtime,
};

pub const TRAY_ID: &str = "main-tray";

/// Every item whose label or enablement follows capture state.
pub struct TrayHandles<R: Runtime> {
    status: MenuItem<R>,
    window: MenuItem<R>,
    record_meeting: MenuItem<R>,
    record_in_person: MenuItem<R>,
    pause: MenuItem<R>,
    stop: MenuItem<R>,
    highlight: MenuItem<R>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaptureMenuState {
    capturing: bool,
    pause_label: &'static str,
}

fn capture_menu_state(state: &str) -> CaptureMenuState {
    match state {
        "recording" => CaptureMenuState {
            capturing: true,
            pause_label: "Pause",
        },
        "paused" => CaptureMenuState {
            capturing: true,
            pause_label: "Resume",
        },
        _ => CaptureMenuState {
            capturing: false,
            pause_label: "Pause",
        },
    }
}

fn window_menu_label(visible: bool) -> &'static str {
    if visible {
        "Hide Notetaker"
    } else {
        "Show Notetaker"
    }
}

#[cfg(target_os = "macos")]
fn icon_bytes(state: &str) -> &'static [u8] {
    // The 36px representation is the @2x source for an 18pt status item. The
    // matching @1x files live beside it for non-retina inspection and asset
    // completeness; tray-icon sizes the NSImage to the native 18pt height.
    match state {
        "recording" => include_bytes!("../icons/tray/macos/recording@2x.png"),
        "paused" => include_bytes!("../icons/tray/macos/paused@2x.png"),
        _ => include_bytes!("../icons/tray/macos/idle@2x.png"),
    }
}

#[cfg(not(target_os = "macos"))]
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

fn set_window_menu_label<R: Runtime>(app: &AppHandle<R>, visible: bool) {
    if let Some(handles) = app.try_state::<TrayHandles<R>>() {
        let _ = handles.window.set_text(window_menu_label(visible));
    }
}

fn sync_window_menu<R: Runtime>(app: &AppHandle<R>) {
    let visible = app
        .get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    set_window_menu_label(app, visible);
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        set_window_menu_label(app, true);
    }
}

fn toggle_main<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };

    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        set_window_menu_label(app, false);
    } else {
        show_main(app);
    }
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<TrayIcon<R>> {
    // Disabled: it is a reading, not a control. Menus have no other way to
    // show a line of state.
    let status = MenuItem::with_id(app, "tray-status", "Not recording", false, None::<&str>)?;
    let window = MenuItem::with_id(
        app,
        "tray-toggle-window",
        "Hide Notetaker",
        true,
        None::<&str>,
    )?;
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
    let settings = MenuItem::with_id(app, "tray-settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit Notetaker", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &window,
            &PredefinedMenuItem::separator(app)?,
            &record_meeting,
            &record_in_person,
            &pause,
            &stop,
            &PredefinedMenuItem::separator(app)?,
            &highlight,
            &copy_last_transcript,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &quit,
        ],
    )?;
    app.manage(TrayHandles {
        status,
        window,
        record_meeting,
        record_in_person,
        pause,
        stop,
        highlight,
    });

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon_image("idle")?)
        // macOS template assets are monochrome alpha masks. On other desktop
        // targets this is false so the existing colored Windows assets retain
        // their state colors and light-taskbar treatment.
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Notetaker")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_tray_icon_event(|tray, _event| {
            // A window can also be hidden by the close policy or global hotkey;
            // refresh the native label immediately before the next menu opens.
            sync_window_menu(tray.app_handle());
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            // Recording actions deliberately do NOT show the window: the point
            // of running them from the tray is not having to open the app. The
            // main webview remains the owner of capture and its safety checks.
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
            "tray-toggle-window" => toggle_main(app),
            "tray-settings" => {
                show_main(app);
                let _ = app.emit("tray-open-settings", ());
            }
            // Quit asks the webview rather than exiting here. app.exit(0)
            // skips destructors, so quitting mid-recording could drop the last
            // unflushed buffer. App.tsx applies the same stop-before-quit guard
            // as the native close button and exits only when it is safe.
            "tray-quit" => {
                show_main(app);
                let _ = app.emit("tray-quit-requested", ());
            }
            _ => {}
        })
        .build(app)
}

/// Called by the frontend when capture state changes. The frontend already
/// polls once per second while a capture is active; Rust does not add a timer.
pub fn set_state<R: Runtime>(app: &AppHandle<R>, state: &str, status_line: &str) {
    let menu_state = capture_menu_state(state);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        // Tauri's atomic setter keeps macOS from briefly rendering a normal
        // image before applying template behavior. Every macOS state is a
        // template image, including recording and paused.
        if let Ok(icon) = icon_image(state) {
            let _ = tray.set_icon_with_as_template(Some(icon), cfg!(target_os = "macos"));
        }
        let _ = tray.set_tooltip(Some(match state {
            "recording" => "Notetaker — recording",
            "paused" => "Notetaker — paused",
            _ => "Notetaker",
        }));
    }
    if let Some(handles) = app.try_state::<TrayHandles<R>>() {
        let _ = handles.status.set_text(status_line);
        let _ = handles.record_meeting.set_enabled(!menu_state.capturing);
        let _ = handles
            .record_in_person
            .set_enabled(!menu_state.capturing);
        let _ = handles.pause.set_enabled(menu_state.capturing);
        let _ = handles.pause.set_text(menu_state.pause_label);
        let _ = handles.stop.set_enabled(menu_state.capturing);
        let _ = handles.highlight.set_enabled(menu_state.capturing);
    }
}

#[cfg(test)]
mod tests {
    use super::{capture_menu_state, window_menu_label};

    #[test]
    fn native_menu_maps_idle_recording_paused_states() {
        assert_eq!(
            capture_menu_state("idle"),
            super::CaptureMenuState {
                capturing: false,
                pause_label: "Pause",
            }
        );
        assert_eq!(
            capture_menu_state("recording"),
            super::CaptureMenuState {
                capturing: true,
                pause_label: "Pause",
            }
        );
        assert_eq!(
            capture_menu_state("paused"),
            super::CaptureMenuState {
                capturing: true,
                pause_label: "Resume",
            }
        );
        assert!(!capture_menu_state("finishing").capturing);
    }

    #[test]
    fn native_menu_label_tracks_main_window_visibility() {
        assert_eq!(window_menu_label(true), "Hide Notetaker");
        assert_eq!(window_menu_label(false), "Show Notetaker");
    }
}
