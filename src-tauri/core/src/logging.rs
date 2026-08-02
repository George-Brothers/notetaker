//! Where this app's log goes, and how it gets there.
//!
//! This is core infrastructure rather than a Tauri plugin so the served app
//! and desktop app leave the same useful evidence behind.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

struct FileLogger {
    file: Mutex<Option<File>>,
}

impl FileLogger {
    const fn new() -> Self {
        Self {
            file: Mutex::new(None),
        }
    }

    fn set_file(&self, file: File) {
        if let Ok(mut current) = self.file.lock() {
            *current = Some(file);
        }
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Ok(mut current) = self.file.lock() {
            if let Some(file) = current.as_mut() {
                let _ = writeln!(
                    file,
                    "{} {} {}: {}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                    record.level(),
                    record.target(),
                    record.args(),
                );
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut current) = self.file.lock() {
            if let Some(file) = current.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

static LOGGER: FileLogger = FileLogger::new();
static LOGGER_INSTALLED: OnceLock<bool> = OnceLock::new();

/// The directory logs are written to.
pub fn log_dir(app_data: &Path) -> PathBuf {
    app_data.join("logs")
}

/// The current log file.
pub fn log_file(app_data: &Path) -> PathBuf {
    log_dir(app_data).join("notetaker.log")
}

/// Wires the global logger and returns the file it writes to.
///
/// Logging is diagnostic scaffolding, so this never prevents the app opening:
/// if the directory or file cannot be made, it simply returns the intended
/// location without a file logger.
pub fn install(app_data: &Path) -> PathBuf {
    let path = log_file(app_data);
    let Some(parent) = path.parent() else {
        return path;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return path;
    }
    let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return path;
    };

    let installed = LOGGER_INSTALLED.get_or_init(|| {
        let installed = log::set_logger(&LOGGER).is_ok();
        if installed {
            log::set_max_level(log::LevelFilter::Info);
        }
        installed
    });
    if *installed {
        LOGGER.set_file(file);
    }
    path
}

/// Flushes buffered records, used before shutdown and by the regression test.
pub fn flush() {
    log::logger().flush();
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use super::*;

    static TEST_LOG_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn logs_land_in_a_logs_directory_under_the_app_data_dir() {
        let base = Path::new("/tmp/appdata");
        assert_eq!(log_dir(base), base.join("logs"));
        assert_eq!(log_file(base), base.join("logs").join("notetaker.log"));
    }

    #[test]
    fn installing_twice_is_not_an_error() {
        let _guard = TEST_LOG_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let first = install(dir.path());
        let second = install(dir.path());
        assert_eq!(first, second);
    }

    #[test]
    fn a_log_directory_that_cannot_be_created_does_not_panic() {
        let _guard = TEST_LOG_LOCK.lock().unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        let _ = install(file.path());
    }

    #[test]
    fn something_logged_after_install_reaches_the_file() {
        let _guard = TEST_LOG_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = install(dir.path());
        log::warn!("canary");
        flush();
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            body.contains("canary"),
            "a warning logged after install must be in {}, got: {body}",
            path.display()
        );
    }
}
