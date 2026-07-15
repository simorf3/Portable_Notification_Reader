//! Portable configuration: a single `config.json` stored next to the executable.
//!
//! Being portable means we never touch the registry or per-user AppData for our
//! own settings – everything lives beside the `.exe`, so the whole app can be
//! copied on a USB stick and run anywhere.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single filter rule. Rules are matched against `"{app}\n{full text}"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRule {
    /// Text or regex to match.
    pub pattern: String,
    /// If true `pattern` is a regular expression, otherwise a case-insensitive substring.
    #[serde(default)]
    pub is_regex: bool,
    /// `true`  = block: matching notifications are silenced.
    /// `false` = allow: enables "allow-list mode" (see [`crate::filter`]).
    #[serde(default = "default_true")]
    pub block: bool,
}

fn default_true() -> bool {
    true
}

/// A rule that rewrites part of a message before it is spoken.
/// (Menu: Filters → Text filtering & replacement rules.)
/// An empty `replacement` simply removes the match (i.e. acts as a filter).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceRule {
    /// Text or regex to find.
    pub pattern: String,
    /// What to replace it with (may be empty to simply remove it).
    #[serde(default)]
    pub replacement: String,
    /// If true `pattern` is a regular expression, otherwise a plain substring.
    #[serde(default)]
    pub is_regex: bool,
}

fn default_volume() -> u32 {
    130
}
fn default_poll() -> u64 {
    1000
}
fn default_voice() -> String {
    // South-African English neural voice, our regional default.
    "online:en-ZA-LeahNeural".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Master on/off switch for speaking notifications.
    pub enabled: bool,
    /// `"online:{ShortName}"` for Edge neural voices or `"sapi:{name}"` for offline voices.
    pub selected_voice_id: String,
    /// Speaking rate. Percent offset for online voices and a -10..10 scale for offline.
    pub rate: i32,
    /// Playback volume 0..=200. 100 = normal, >100 amplifies (louder than system default).
    pub volume: u32,
    /// When false, only voices whose language matches the Windows UI language are shown.
    pub show_all_languages: bool,
    /// Apps the user chose NOT to read aloud (display names, case-insensitive).
    pub muted_apps: Vec<String>,
    /// Apps that have ever sent a notification (populated automatically for the UI).
    pub known_apps: Vec<String>,
    /// User filter rules.
    pub filters: Vec<FilterRule>,
    /// Text filtering & replacement rules applied to a message before it is
    /// spoken. An empty `replacement` removes the match.
    #[serde(default)]
    pub replacements: Vec<ReplaceRule>,
    /// When true, emojis are spoken by their meaning (e.g. "smiling face").
    /// When false (default), emojis are removed from the message before speaking.
    #[serde(default)]
    pub speak_emojis: bool,
    /// When true, notifications are NOT spoken while the microphone is in use by
    /// another app (e.g. a call or recording), so the app stays quiet during
    /// meetings. Default on.
    #[serde(default = "default_true")]
    pub pause_on_mic: bool,
    /// When true, other apps (video/music) are briefly turned down while a
    /// notification is read, so the app never talks over them. Default on.
    #[serde(default = "default_true")]
    pub duck_while_speaking: bool,
    /// How often to poll the notification database, in milliseconds.
    pub poll_interval_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            selected_voice_id: default_voice(),
            rate: 0,
            volume: default_volume(),
            show_all_languages: false,
            muted_apps: Vec::new(),
            known_apps: Vec::new(),
            filters: Vec::new(),
            replacements: Vec::new(),
            speak_emojis: false,
            pause_on_mic: true,
            duck_while_speaking: true,
            poll_interval_ms: default_poll(),
        }
    }
}

impl Config {
    /// Directory that contains the running executable (portable root).
    pub fn app_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn config_path() -> PathBuf {
        Self::app_dir().join("config.json")
    }

    /// Load configuration from disk, falling back to defaults on any error.
    pub fn load() -> Self {
        let path = Self::config_path();
        match Self::load_from(&path) {
            Ok(c) => c,
            Err(_) => Config::default(),
        }
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config = serde_json::from_str(&data).context("parsing config.json")?;
        Ok(cfg.sanitized())
    }

    /// Clamp values into valid ranges so a hand-edited file can never break us.
    pub fn sanitized(mut self) -> Self {
        if self.volume > 200 {
            self.volume = 200;
        }
        if self.rate < -10 {
            self.rate = -10;
        }
        if self.rate > 10 {
            self.rate = 10;
        }
        if self.poll_interval_ms < 250 {
            self.poll_interval_ms = 250; // guard against pathological values
        }
        if self.selected_voice_id.trim().is_empty() {
            self.selected_voice_id = default_voice();
        }
        self
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        let data = serde_json::to_string_pretty(self).context("serialising config")?;
        std::fs::write(&path, data).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Gain multiplier for audio playback (1.0 == 100%).
    pub fn gain(&self) -> f32 {
        (self.volume as f32) / 100.0
    }

    pub fn is_app_muted(&self, app: &str) -> bool {
        let a = app.trim();
        self.muted_apps
            .iter()
            .any(|m| m.eq_ignore_ascii_case(a))
    }

    /// Record an app that sent a notification. Returns true if it was newly added.
    pub fn remember_app(&mut self, app: &str) -> bool {
        let a = app.trim();
        if a.is_empty() {
            return false;
        }
        if self.known_apps.iter().any(|k| k.eq_ignore_ascii_case(a)) {
            return false;
        }
        self.known_apps.push(a.to_string());
        self.known_apps.sort_by_key(|s| s.to_lowercase());
        true
    }
}
