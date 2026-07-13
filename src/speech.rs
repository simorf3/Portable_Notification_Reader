//! Speech engine: speaks text using online Edge neural voices (default) and
//! falls back to an offline SAPI voice when the network is unavailable.
//!
//! Online audio is MP3 played through `rodio`, where the sink volume doubles as a
//! gain control so we can play *louder than the system default* (gain > 1.0).
//! This module is Windows-only (audio + SAPI).

use crate::edge_tts;
use std::io::Cursor;
use std::time::Duration;

pub const MAX_VOLUME_PERCENT: u32 = 200;

pub struct SpeechEngine {
    rt: tokio::runtime::Runtime,
    // Keep the output stream alive for the lifetime of the engine.
    _stream: Option<rodio::OutputStream>,
    handle: Option<rodio::OutputStreamHandle>,
    offline: Option<tts::Tts>,
    /// Gain multiplier (1.0 = 100%).
    gain: f32,
    /// Rate on our -10..=10 scale.
    rate: i32,
}

impl SpeechEngine {
    pub fn new(gain: f32, rate: i32) -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        let (stream, handle) = match rodio::OutputStream::try_default() {
            Ok((s, h)) => (Some(s), Some(h)),
            Err(e) => {
                log::warn!("no audio output device: {e}");
                (None, None)
            }
        };

        SpeechEngine {
            rt,
            _stream: stream,
            handle,
            offline: None,
            gain,
            rate,
        }
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    pub fn set_rate(&mut self, rate: i32) {
        self.rate = rate;
    }

    /// Speak `text` using the configured voice id ("online:{short}" or "sapi:{name}").
    /// Returns true if it spoke online, false if it used (or fell back to) offline.
    pub fn speak(&mut self, voice_id: &str, text: &str) -> bool {
        if text.trim().is_empty() {
            return true;
        }

        if let Some(short) = voice_id.strip_prefix("online:") {
            match self.speak_online(short, text) {
                Ok(()) => return true,
                Err(e) => {
                    log::warn!("online TTS failed ({e:#}); falling back to offline voice");
                    self.speak_offline(None, text);
                    return false;
                }
            }
        }

        if let Some(name) = voice_id.strip_prefix("sapi:") {
            self.speak_offline(Some(name), text);
            return false;
        }

        // Unknown id: try online as a best guess, else offline.
        match self.speak_online(voice_id, text) {
            Ok(()) => true,
            Err(_) => {
                self.speak_offline(None, text);
                false
            }
        }
    }

    fn speak_online(&mut self, short_name: &str, text: &str) -> anyhow::Result<()> {
        let rate_percent = (self.rate * 10).clamp(-100, 100);
        let mp3 = self
            .rt
            .block_on(edge_tts::synthesize(short_name, text, rate_percent))?;
        self.play_mp3(mp3)
    }

    fn play_mp3(&self, bytes: Vec<u8>) -> anyhow::Result<()> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no audio output device"))?;
        let sink = rodio::Sink::try_new(handle)?;
        // Sink volume > 1.0 amplifies (louder than normal).
        sink.set_volume(self.gain.max(0.0));
        let source = rodio::Decoder::new(Cursor::new(bytes))?;
        sink.append(source);
        sink.sleep_until_end();
        Ok(())
    }

    fn ensure_offline(&mut self) -> Option<&mut tts::Tts> {
        if self.offline.is_none() {
            match tts::Tts::default() {
                Ok(t) => self.offline = Some(t),
                Err(e) => {
                    log::error!("could not initialise offline TTS: {e}");
                    return None;
                }
            }
        }
        self.offline.as_mut()
    }

    fn speak_offline(&mut self, voice_name: Option<&str>, text: &str) {
        let gain = self.gain;
        let tts = match self.ensure_offline() {
            Some(t) => t,
            None => return,
        };

        // Pick the requested voice if we can find it.
        if let Some(name) = voice_name {
            if let Ok(voices) = tts.voices() {
                if let Some(v) = voices
                    .into_iter()
                    .find(|v| v.name().eq_ignore_ascii_case(name) || v.id() == name)
                {
                    let _ = tts.set_voice(&v);
                }
            }
        }

        // Offline volume is capped at the OS maximum (no >100% amplification here).
        if let Features { volume: true, .. } = tts.supported_features() {
            let max = tts.max_volume();
            let normal = tts.normal_volume();
            let target = (normal * gain).min(max);
            let _ = tts.set_volume(target);
        }

        if tts.speak(text, true).is_ok() {
            // Block until finished so notifications are spoken one at a time.
            for _ in 0..600 {
                match tts.is_speaking() {
                    Ok(true) => std::thread::sleep(Duration::from_millis(50)),
                    _ => break,
                }
            }
        }
    }

    /// List offline SAPI voices available on this machine.
    pub fn list_offline_voices(&mut self) -> Vec<crate::voices::Voice> {
        let tts = match self.ensure_offline() {
            Some(t) => t,
            None => return Vec::new(),
        };
        let voices = match tts.voices() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        voices
            .into_iter()
            .map(|v| {
                let locale = v.language().to_string();
                crate::voices::Voice {
                    id: crate::voices::Voice::sapi_id(&v.name()),
                    token: v.name(),
                    display: format!("{} (offline)", v.name()),
                    locale,
                    gender: String::new(),
                    online: false,
                }
            })
            .collect()
    }
}

use tts::Features;
