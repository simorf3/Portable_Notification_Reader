//! Background worker: polls the notification database on an interval and speaks
//! new notifications. Shares state with the UI thread via `Arc<Mutex<..>>`.

use crate::config::Config;
use crate::filter;
use crate::notifications::NotificationReader;
use crate::speech::SpeechEngine;
use crate::voices::{self, Voice};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Configuration shared between the UI and the worker.
pub type SharedConfig = Arc<Mutex<Config>>;

/// The merged voice catalogue, filled in asynchronously at startup.
#[derive(Default)]
pub struct VoiceCatalog {
    pub online: Vec<Voice>,
    pub offline: Vec<Voice>,
    pub online_ready: bool,
    pub offline_ready: bool,
}

impl VoiceCatalog {
    /// All voices merged (online first, then offline).
    pub fn all(&self) -> Vec<Voice> {
        let mut v = self.online.clone();
        v.extend(self.offline.clone());
        v
    }
}

pub type SharedCatalog = Arc<Mutex<VoiceCatalog>>;

/// Ad-hoc phrases the UI wants spoken now (e.g. the "Test voice" action).
pub type SayQueue = Arc<Mutex<Vec<String>>>;

/// Latest voice id the UI wants to preview (from hovering the voice menu).
/// "Latest wins": a new hover replaces any pending preview.
pub type PreviewSlot = Arc<Mutex<Option<String>>>;

/// Short phrase used when previewing a voice on hover.
const PREVIEW_TEXT: &str = "Hello, this is a preview of this voice.";

/// Fetch the full online voice list on a background thread (network, ~1s).
pub fn spawn_voice_fetch(catalog: SharedCatalog) {
    std::thread::spawn(move || {
        let mut online = voices::fetch_online_voices();
        if online.is_empty() {
            log::warn!("online voice fetch failed; using seeded fallback list");
            online = voices::fallback_online_voices();
        }
        if let Ok(mut c) = catalog.lock() {
            c.online = online;
            c.online_ready = true;
        }
    });
}

/// Spawn the notification-polling / speaking worker thread.
pub fn spawn(
    cfg: SharedConfig,
    catalog: SharedCatalog,
    say_queue: SayQueue,
    preview: PreviewSlot,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        run(cfg, catalog, say_queue, preview);
    })
}

fn run(cfg: SharedConfig, catalog: SharedCatalog, say_queue: SayQueue, preview: PreviewSlot) {
    // Audio + SAPI must be created on this thread (rodio/tts are not Send).
    let (gain, rate) = {
        let c = cfg.lock().unwrap();
        (c.gain(), c.rate)
    };
    let mut engine = SpeechEngine::new(gain, rate);

    // Publish offline SAPI voices into the shared catalogue for the UI menu.
    let offline = engine.list_offline_voices();
    if let Ok(mut c) = catalog.lock() {
        c.offline = offline;
        c.offline_ready = true;
    }

    let db_path = match crate::notifications::default_db_path() {
        Some(p) => p,
        None => {
            log::error!("could not resolve notification database path");
            return;
        }
    };
    let mut reader = NotificationReader::new(db_path);
    if let Err(e) = reader.initialize_baseline() {
        log::warn!("baseline init failed (will retry as we poll): {e:#}");
    }

    // Seed the "Apps" menu with every app that has ever sent a notification, so
    // the list is populated on first launch (not just as new items arrive).
    if let Ok(apps) = reader.query_all_app_ids() {
        let mut changed = false;
        {
            let mut c = cfg.lock().unwrap();
            for app in apps {
                if c.remember_app(&app) {
                    changed = true;
                }
            }
        }
        if changed {
            if let Ok(c) = cfg.lock() {
                let _ = c.save(); // non-critical; don't fail if save errors
            }
        }
    }

    // Tick frequently so voice previews and the "Test voice" action feel
    // responsive, but only hit the database every `poll_interval_ms`.
    let tick = Duration::from_millis(120);
    let mut last_poll = std::time::Instant::now()
        .checked_sub(Duration::from_secs(3600))
        .unwrap_or_else(std::time::Instant::now);

    loop {
        // 1) Voice preview (hovering a voice in the menu). Latest wins.
        let preview_id = preview.lock().ok().and_then(|mut s| s.take());
        if let Some(voice_id) = preview_id {
            let (gain, rate) = {
                let c = cfg.lock().unwrap();
                (c.gain(), c.rate)
            };
            engine.set_gain(gain);
            engine.set_rate(rate);
            engine.speak(&voice_id, PREVIEW_TEXT);
        }

        // 2) Ad-hoc "say this now" requests from the UI (Test voice).
        let pending: Vec<String> = {
            let mut q = say_queue.lock().unwrap();
            std::mem::take(&mut *q)
        };
        for phrase in pending {
            let (voice_id, gain, rate) = {
                let c = cfg.lock().unwrap();
                (c.selected_voice_id.clone(), c.gain(), c.rate)
            };
            engine.set_gain(gain);
            engine.set_rate(rate);
            engine.speak(&voice_id, &phrase);
        }

        // 3) Poll the notification DB on the configured interval.
        let interval = {
            let c = cfg.lock().unwrap();
            c.poll_interval_ms
        };
        if last_poll.elapsed() >= Duration::from_millis(interval) {
            last_poll = std::time::Instant::now();
            match reader.poll_new() {
                Ok(items) => {
                    for n in items {
                        handle_notification(&cfg, &mut engine, &n);
                    }
                }
                Err(e) => {
                    // DB may be transiently locked by Windows; retry next poll.
                    log::debug!("poll error: {e:#}");
                }
            }
        }

        std::thread::sleep(tick);
    }
}

fn handle_notification(
    cfg: &SharedConfig,
    engine: &mut SpeechEngine,
    n: &crate::notifications::RawNotification,
) {
    // Remember every app that sends notifications (for the per-app menu).
    let mut changed = false;
    {
        let mut c = cfg.lock().unwrap();
        if c.remember_app(&n.app_display) {
            changed = true;
        }
    }
    if changed {
        let c = cfg.lock().unwrap();
        let _ = c.save();
    }

    // Snapshot the settings we need.
    let (enabled, voice_id, gain, rate, muted, filters, text_filters, replacements, speak_emojis) = {
        let c = cfg.lock().unwrap();
        (
            c.enabled,
            c.selected_voice_id.clone(),
            c.gain(),
            c.rate,
            c.is_app_muted(&n.app_display),
            c.filters.clone(),
            c.text_filters.clone(),
            c.replacements.clone(),
            c.speak_emojis,
        )
    };

    if !enabled || muted {
        return;
    }
    if n.text_parts.is_empty() {
        return;
    }

    let spoken = filter::build_spoken_text(&n.text_parts);
    // Filter against app name + the full text so existing rules keep working.
    let full_text = n.text_parts.join(" ");
    if !filter::passes_filters(&n.app_display, &format!("{spoken}\n{full_text}"), &filters) {
        return;
    }

    // Shape the text before speaking:
    //   1) remove user "Filter text" phrases
    //   2) apply user "Replace text" rules
    //   3) expand chat shorthand + handle emojis (strip or speak meaning)
    let spoken = filter::apply_text_filters(&spoken, &text_filters);
    let spoken = filter::apply_replacements(&spoken, &replacements);
    let spoken = crate::text_shaping::shape(&spoken, speak_emojis);

    // Nothing left to say after shaping (e.g. an emoji-only message with
    // emojis turned off) – skip speaking silence.
    if spoken.trim().is_empty() {
        return;
    }

    engine.set_gain(gain);
    engine.set_rate(rate);
    engine.speak(&voice_id, &spoken);
}
