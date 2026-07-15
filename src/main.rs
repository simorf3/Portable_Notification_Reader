//! Portable Notification Reader – entry point.
//!
//! On Windows this starts the tray UI plus the background notification-polling
//! worker. On other platforms it is a stub (the app depends on Windows APIs).

// No console window on Windows (GUI/tray app).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use portable_notification_reader::{app, config::Config, logging, startup, worker};
    use std::sync::{Arc, Mutex};

    // Start a fresh diagnostic log next to the executable.
    logging::init(&Config::app_dir());

    // First run = no config.json has been written yet.
    let first_run = !Config::config_path().exists();

    // Shared state between the UI thread and the worker thread.
    let cfg = Arc::new(Mutex::new(Config::load()));

    if first_run {
        // Persist defaults so the next launch is not treated as a first run.
        if let Ok(c) = cfg.lock() {
            let _ = c.save();
        }
        // Offer to start automatically at sign-in (only asked once; skipped
        // entirely if a startup shortcut already exists).
        startup::prompt_on_first_run();
    }
    let catalog = Arc::new(Mutex::new(worker::VoiceCatalog::default()));
    let say_queue: worker::SayQueue = Arc::new(Mutex::new(Vec::new()));
    // Slot the UI writes into when the user hovers a voice (latest wins).
    let preview: worker::PreviewSlot = Arc::new(Mutex::new(None));

    // Kick off the online voice-list download and the polling worker.
    worker::spawn_voice_fetch(catalog.clone());
    let _worker = worker::spawn(cfg.clone(), catalog.clone(), say_queue.clone(), preview.clone());

    nwg::init().expect("failed to initialise native Windows GUI");
    let _ui = app::App::build(cfg, catalog, say_queue, preview).expect("failed to build tray UI");

    // Runs until the tray "Exit" action calls stop_thread_dispatch().
    nwg::dispatch_thread_events();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Portable Notification Reader is a Windows-only application.");
    std::process::exit(1);
}
