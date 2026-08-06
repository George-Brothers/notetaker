//! Native configuration shared by the tray popover and the recording overlay.
//!
//! The webviews remain ordinary Tauri windows on Windows. On macOS they are
//! converted to panels by tauri-nspanel so showing a control does not activate
//! Notetaker or steal the meeting app's focus. Keeping the conversion here is
//! important: a hand-cast objc2 style mask is an easy way to reintroduce the
//! crash fixed by tauri-nspanel issue #19.

use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

#[derive(Clone, Copy)]
pub enum FloatingWindow {
    TrayPanel,
    Overlay,
}

#[cfg(target_os = "macos")]
mod macos {
    use tauri::Manager;
    use tauri_nspanel::tauri_panel;

    tauri_panel! {
        panel!(NotetakerPanel {
            config: {
                can_become_key_window: true,
                can_become_main_window: false,
                is_floating_panel: true
            }
        })
    }
}

/// Apply the non-activating panel behavior on macOS.
pub fn configure_panel<R: Runtime>(
    window: &WebviewWindow<R>,
    kind: FloatingWindow,
) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};

        let panel = window.to_panel::<macos::NotetakerPanel<R>>()?;
        // This is deliberately the plugin's StyleMask helper. Do not replace
        // it with an objc2 cast: that is the crash-prone path the plugin is
        // designed to keep out of application code.
        panel.set_style_mask(StyleMask::empty().borderless().nonactivating_panel().into());
        panel.set_collection_behavior(
            CollectionBehavior::new()
                .full_screen_auxiliary()
                .can_join_all_spaces()
                .ignores_cycle()
                .into(),
        );
        panel.set_level(match kind {
            FloatingWindow::TrayPanel => PanelLevel::PopUpMenu.value(),
            FloatingWindow::Overlay => PanelLevel::ScreenSaver.value(),
        });
    }

    #[cfg(not(target_os = "macos"))]
    let _ = (window, kind);

    Ok(())
}

/// Apply the platform glass material. The CSS fallback is always present in
/// the webview, because native material APIs can be unavailable in a VM or on
/// an older Windows build.
pub fn apply_vibrancy<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
        let _ = apply_vibrancy(
            window,
            NSVisualEffectMaterial::HudWindow,
            Some(NSVisualEffectState::Active),
            Some(16.0),
        );
    }

    #[cfg(target_os = "windows")]
    {
        if window_vibrancy::apply_mica(window, None).is_err() {
            let _ = window_vibrancy::apply_acrylic(window, None);
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = window;
}

/// Show a floating window without routing through the activating main-window
/// path. On macOS the panel's `orderFrontRegardless` is the plugin-supported
/// non-activating operation; Windows uses the ordinary flyout show behavior.
pub fn show_panel<R: Runtime>(app: &AppHandle<R>, label: &str) {
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        if let Ok(panel) = app.get_webview_panel(label) {
            panel.show();
            return;
        }
    }

    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
    }
}
