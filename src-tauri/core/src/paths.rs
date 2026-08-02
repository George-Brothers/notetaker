//! Where the app's two directories live on each operating system.
//!
//! There are exactly two, and they are deliberately different kinds of thing:
//!
//! - The **storage root** holds the user's recordings. It is
//!   `<home>/Notetaker` on *every* platform, on purpose — that layout is a
//!   public contract, and keeping the shape identical means
//!   a `Notetaker` folder copied from a Mac to a PC still opens, with the
//!   SQLite index rebuilt from the files.
//! - The **app dir** holds the index, queue state and settings: things the app
//!   owns and can rebuild. This one follows each OS's convention, because it
//!   belongs in the place that OS backs up, syncs and cleans up. Nothing in it
//!   is precious.
//!
//! Everything here splits the same way as [`crate::power::probe`]: the
//! *decisions* are pure functions over inputs and are fully tested; the thin
//! layer that reads environment variables off the real machine is not, since a
//! test that sets `HOME` would be reading global state shared with every other
//! test in the binary.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// The folder name used for the user's recordings, under their home directory.
/// Part of the storage contract; changing it orphans existing libraries.
pub const LIBRARY_DIR: &str = "Notetaker";

/// Which OS convention to follow for the app dir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    /// Linux and the other XDG-style unixes.
    Xdg,
}

/// The platform this build targets. `const` so it costs nothing, and a plain
/// `cfg` chain so a new OS is a compile-time decision rather than a silent
/// fallthrough that puts files somewhere surprising.
pub const fn current_platform() -> Platform {
    #[cfg(target_os = "windows")]
    {
        Platform::Windows
    }
    #[cfg(target_os = "macos")]
    {
        Platform::MacOs
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Platform::Xdg
    }
}

/// The user's recording library: `<home>/Notetaker`, identically everywhere.
pub fn storage_root_in(home: &Path) -> PathBuf {
    home.join(LIBRARY_DIR)
}

/// Where the app keeps the index, queue and settings.
///
/// `appdata` is Windows' `%APPDATA%` and `xdg_data` is `$XDG_DATA_HOME`; both
/// are `Option` because both are routinely unset, and each falls back to the
/// documented default for its platform rather than failing. A missing
/// `%APPDATA%` on Windows is unusual but survivable — `<home>\AppData\Roaming`
/// is what it would have said anyway.
pub fn app_dir_in(
    platform: Platform,
    home: &Path,
    appdata: Option<&Path>,
    xdg_data: Option<&Path>,
) -> PathBuf {
    match platform {
        Platform::Windows => match appdata {
            Some(p) => p.join(LIBRARY_DIR),
            None => home.join("AppData").join("Roaming").join(LIBRARY_DIR),
        },
        Platform::MacOs => home
            .join("Library")
            .join("Application Support")
            .join(LIBRARY_DIR),
        Platform::Xdg => match xdg_data {
            Some(p) => p.join(LIBRARY_DIR),
            None => home.join(".local").join("share").join(LIBRARY_DIR),
        },
    }
}

/// The current user's home directory, read from the environment.
///
/// Windows uses `USERPROFILE`; everything else uses `HOME`. An empty value is
/// treated as absent, because an empty `HOME` would otherwise resolve the
/// library to the relative path `Notetaker` in whatever directory the app
/// happened to be launched from.
pub fn home_dir() -> Result<PathBuf> {
    let var = if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    };
    non_empty_env(var)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("Notetaker could not work out where your home folder is ({var} is not set), so it does not know where to keep your recordings."))
}

/// The default storage root for this machine.
pub fn default_storage_root() -> Result<PathBuf> {
    Ok(storage_root_in(&home_dir()?))
}

/// The default app dir for this machine.
pub fn default_app_dir() -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(app_dir_in(
        current_platform(),
        &home,
        non_empty_env("APPDATA").map(PathBuf::from).as_deref(),
        non_empty_env("XDG_DATA_HOME").map(PathBuf::from).as_deref(),
    ))
}

/// `std::env::var` but treating an empty string as unset, which is how an
/// unset variable usually reaches a process that was launched by a script.
fn non_empty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the storage contract ------------------------------------------

    /// The whole point of this module: the library path has the same shape on
    /// every OS, so a folder moved between machines still opens. If this test
    /// ever needs a per-platform branch, the contract has been broken.
    #[test]
    fn library_path_is_identical_on_every_platform() {
        assert_eq!(
            storage_root_in(Path::new("/home/alex")),
            Path::new("/home/alex/Notetaker")
        );
        assert_eq!(
            storage_root_in(Path::new(r"C:\Users\alex")),
            Path::new(r"C:\Users\alex").join("Notetaker")
        );
    }

    #[test]
    fn storage_root_never_depends_on_the_platform_argument() {
        let home = Path::new("/home/alex");
        // There is no platform parameter to pass — that is the assertion.
        // This test exists so that adding one is a visible change here.
        assert!(storage_root_in(home).ends_with(LIBRARY_DIR));
    }

    // --- app dir, per platform ------------------------------------------

    #[test]
    fn windows_app_dir_uses_appdata_when_set() {
        let dir = app_dir_in(
            Platform::Windows,
            Path::new(r"C:\Users\alex"),
            Some(Path::new(r"C:\Users\alex\AppData\Roaming")),
            None,
        );
        assert_eq!(
            dir,
            Path::new(r"C:\Users\alex\AppData\Roaming").join("Notetaker")
        );
    }

    /// `%APPDATA%` unset must not produce a relative path. "Rooted at the home
    /// directory we were handed" is the portable way to say that — testing
    /// `is_absolute()` would be checking the host's path semantics rather than
    /// this function's, since a Windows path is not absolute to a Linux
    /// `Path` and `starts_with` compares whole components.
    #[test]
    fn windows_app_dir_falls_back_to_roaming_under_home() {
        let home = Path::new(r"C:\Users\alex");
        let dir = app_dir_in(Platform::Windows, home, None, None);
        assert_eq!(dir, home.join("AppData").join("Roaming").join("Notetaker"));
        assert!(
            dir.starts_with(home),
            "the fallback must stay under the home directory, got {dir:?}"
        );
    }

    #[test]
    fn macos_app_dir_is_application_support() {
        let dir = app_dir_in(Platform::MacOs, Path::new("/Users/alex"), None, None);
        assert_eq!(
            dir,
            Path::new("/Users/alex/Library/Application Support/Notetaker")
        );
    }

    /// macOS has no `%APPDATA%` or `$XDG_DATA_HOME` concept, and must ignore
    /// them even if something in the environment has set them.
    #[test]
    fn macos_app_dir_ignores_appdata_and_xdg() {
        let dir = app_dir_in(
            Platform::MacOs,
            Path::new("/Users/alex"),
            Some(Path::new("/nonsense/appdata")),
            Some(Path::new("/nonsense/xdg")),
        );
        assert_eq!(
            dir,
            Path::new("/Users/alex/Library/Application Support/Notetaker")
        );
    }

    #[test]
    fn xdg_app_dir_uses_xdg_data_home_when_set() {
        let dir = app_dir_in(
            Platform::Xdg,
            Path::new("/home/alex"),
            None,
            Some(Path::new("/home/alex/.local/share")),
        );
        assert_eq!(dir, Path::new("/home/alex/.local/share/Notetaker"));
    }

    #[test]
    fn xdg_app_dir_falls_back_to_dot_local_share() {
        let dir = app_dir_in(Platform::Xdg, Path::new("/home/alex"), None, None);
        assert_eq!(dir, Path::new("/home/alex/.local/share/Notetaker"));
    }

    /// The app dir must never land inside the user's recording library —
    /// a SQLite index sitting in `Notetaker/` would be scanned as if it were a
    /// recording folder, and `storage` treats unknown directories as data.
    #[test]
    fn app_dir_is_never_inside_the_storage_root() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Xdg] {
            let home = Path::new("/home/alex");
            let app = app_dir_in(platform, home, None, None);
            let library = storage_root_in(home);
            assert!(
                !app.starts_with(&library),
                "{platform:?} puts the app dir inside the library: {app:?}"
            );
        }
    }

    // --- the env layer --------------------------------------------------

    #[test]
    fn empty_env_var_reads_as_absent() {
        // Uses a name nothing else will touch, and restores it immediately.
        let key = "NOTETAKER_PATHS_TEST_EMPTY";
        std::env::set_var(key, "   ");
        let got = non_empty_env(key);
        std::env::remove_var(key);
        assert_eq!(got, None);
    }

    #[test]
    fn current_platform_matches_the_build_target() {
        let expected = if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Xdg
        };
        assert_eq!(current_platform(), expected);
    }
}
