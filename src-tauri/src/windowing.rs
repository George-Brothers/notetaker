//! Native configuration for the optional recording overlay.
//!
//! The main window remains an ordinary decorated Tauri window. Only the
//! recording overlay is converted to a non-activating macOS panel, so it can
//! stay above a meeting without making Notetaker feel like a menu-bar-only
//! accessory or stealing the meeting app's focus.

use tauri::{Runtime, WebviewWindow};

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

/// Configure the optional overlay as a non-activating panel on macOS.
pub fn configure_overlay_panel<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
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
        panel.set_level(PanelLevel::ScreenSaver.value());
    }

    #[cfg(not(target_os = "macos"))]
    let _ = window;

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
