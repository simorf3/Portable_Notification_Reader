//! Tiny file logger. Writes a fresh `notification-reader.log` next to the
//! executable each time the app starts, so you can see exactly what the app
//! decided to do with every notification (spoken, muted, filtered, skipped…).
//! This is invaluable for diagnosing "app X pops up but isn't read" problems –
//! open the log from the tray menu and look for that app's line.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};

/// File name of the log, stored in the portable app folder.
pub const LOG_FILE: &str = "notification-reader.log";

/// Full path to the log file (next to the executable).
pub fn log_path(app_dir: &Path) -> PathBuf {
    app_dir.join(LOG_FILE)
}

struct FileLogger {
    file: Mutex<std::fs::File>,
}

impl Log for FileLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if let Ok(mut f) = self.file.lock() {
            let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(f, "{ts} [{:<5}] {}", record.level(), record.args());
            let _ = f.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

/// Initialise file logging. Truncates any previous log so each run starts fresh.
/// Silently does nothing if the file can't be opened (e.g. read-only folder).
pub fn init(app_dir: &Path) {
    let path = log_path(app_dir);
    if let Ok(file) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
    {
        let logger = Box::new(FileLogger {
            file: Mutex::new(file),
        });
        if log::set_boxed_logger(logger).is_ok() {
            log::set_max_level(LevelFilter::Debug);
            log::info!("Portable Notification Reader v{} started", env!("CARGO_PKG_VERSION"));
        }
    }
}
