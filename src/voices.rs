//! Voice catalogue: fetches the *full* set of Microsoft online neural voices and
//! filters them by the Windows UI language (unless the user opts to see them all).

use crate::drm;
use serde::Deserialize;

const VOICE_LIST_URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list";

/// A voice the user can pick, either an online neural voice or an offline SAPI voice.
#[derive(Debug, Clone)]
pub struct Voice {
    /// Stable id stored in config: `"online:{ShortName}"` or `"sapi:{name}"`.
    pub id: String,
    /// The provider identifier (Edge ShortName, e.g. `en-ZA-LeahNeural`, or SAPI name).
    pub token: String,
    /// Human friendly label shown in the menu.
    pub display: String,
    /// BCP-47 locale, e.g. `en-ZA`.
    pub locale: String,
    /// `"Male"` / `"Female"` where known.
    pub gender: String,
    /// True for online neural voices, false for offline SAPI voices.
    pub online: bool,
}

impl Voice {
    pub fn online_id(short_name: &str) -> String {
        format!("online:{short_name}")
    }
    pub fn sapi_id(name: &str) -> String {
        format!("sapi:{name}")
    }
    /// Language subtag (`en` from `en-ZA`), lower-cased.
    pub fn language(&self) -> String {
        self.locale
            .split('-')
            .next()
            .unwrap_or("")
            .to_lowercase()
    }
}

/// Raw JSON shape returned by the Edge voices/list endpoint.
#[derive(Debug, Deserialize)]
struct RawVoice {
    #[serde(rename = "ShortName")]
    short_name: String,
    #[serde(rename = "Gender")]
    gender: String,
    #[serde(rename = "Locale")]
    locale: String,
    #[serde(rename = "FriendlyName", default)]
    friendly_name: String,
    #[serde(rename = "LocaleName", default)]
    locale_name: String,
}

fn pretty_display(short_name: &str, locale_name: &str, gender: &str) -> String {
    // ShortName looks like "en-ZA-LeahNeural" -> pull out "Leah".
    let person = short_name
        .rsplit('-')
        .next()
        .unwrap_or(short_name)
        .trim_end_matches("Neural")
        .trim_end_matches("Multilingual");
    let loc = if locale_name.is_empty() {
        short_name.split('-').take(2).collect::<Vec<_>>().join("-")
    } else {
        locale_name.to_string()
    };
    let g = match gender {
        "Female" => "♀",
        "Male" => "♂",
        _ => "",
    };
    format!("{person} {g} — {loc}")
}

/// Fetch the complete list of online neural voices from Microsoft.
///
/// Returns an empty vec on any network/parse error; callers should fall back to
/// [`fallback_online_voices`].
pub fn fetch_online_voices() -> Vec<Voice> {
    let url = format!(
        "{VOICE_LIST_URL}?trustedclienttoken={}",
        drm::TRUSTED_CLIENT_TOKEN
    );
    let sec = drm::generate_sec_ms_gec();

    let resp = ureq::get(&url)
        .header("Sec-MS-GEC", &sec)
        .header("Sec-MS-GEC-Version", drm::SEC_MS_GEC_VERSION)
        .header("User-Agent", drm::USER_AGENT)
        .header("Origin", drm::ORIGIN)
        .header("Accept", "*/*")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Cookie", &drm::muid_cookie())
        .call();

    let body = match resp {
        Ok(mut r) => match r.body_mut().read_to_string() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        },
        Err(_) => return Vec::new(),
    };
    let raw: Vec<RawVoice> = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    raw.into_iter()
        .map(|rv| {
            let display = if rv.friendly_name.is_empty() {
                pretty_display(&rv.short_name, &rv.locale_name, &rv.gender)
            } else {
                pretty_display(&rv.short_name, &rv.locale_name, &rv.gender)
            };
            Voice {
                id: Voice::online_id(&rv.short_name),
                token: rv.short_name.clone(),
                display,
                locale: rv.locale,
                gender: rv.gender,
                online: true,
            }
        })
        .collect()
}

/// A curated offline fallback list used only when the live fetch fails.
/// Includes the South-African voices plus a broad multi-locale English set.
pub fn fallback_online_voices() -> Vec<Voice> {
    // (ShortName, LocaleName, Gender)
    const SEED: &[(&str, &str, &str)] = &[
        ("en-ZA-LeahNeural", "English (South Africa)", "Female"),
        ("en-ZA-LukeNeural", "English (South Africa)", "Male"),
        ("af-ZA-AdriNeural", "Afrikaans (South Africa)", "Female"),
        ("af-ZA-WillemNeural", "Afrikaans (South Africa)", "Male"),
        ("zu-ZA-ThandoNeural", "Zulu (South Africa)", "Female"),
        ("zu-ZA-ThembaNeural", "Zulu (South Africa)", "Male"),
        ("en-US-AriaNeural", "English (United States)", "Female"),
        ("en-US-JennyNeural", "English (United States)", "Female"),
        ("en-US-GuyNeural", "English (United States)", "Male"),
        ("en-US-AndrewNeural", "English (United States)", "Male"),
        ("en-US-EmmaNeural", "English (United States)", "Female"),
        ("en-US-BrianNeural", "English (United States)", "Male"),
        ("en-GB-SoniaNeural", "English (United Kingdom)", "Female"),
        ("en-GB-RyanNeural", "English (United Kingdom)", "Male"),
        ("en-GB-LibbyNeural", "English (United Kingdom)", "Female"),
        ("en-AU-NatashaNeural", "English (Australia)", "Female"),
        ("en-AU-WilliamNeural", "English (Australia)", "Male"),
        ("en-IE-EmilyNeural", "English (Ireland)", "Female"),
        ("en-IE-ConnorNeural", "English (Ireland)", "Male"),
        ("en-CA-ClaraNeural", "English (Canada)", "Female"),
        ("en-CA-LiamNeural", "English (Canada)", "Male"),
        ("en-IN-NeerjaNeural", "English (India)", "Female"),
        ("en-IN-PrabhatNeural", "English (India)", "Male"),
    ];
    SEED.iter()
        .map(|(sn, ln, g)| Voice {
            id: Voice::online_id(sn),
            token: (*sn).to_string(),
            display: pretty_display(sn, ln, g),
            locale: sn.split('-').take(2).collect::<Vec<_>>().join("-"),
            gender: (*g).to_string(),
            online: true,
        })
        .collect()
}

/// Order and filter a combined voice list for display.
///
/// * `ui_language` – language subtag from the Windows UI language (e.g. `en`).
/// * `ui_locale`   – full Windows UI locale (e.g. `en-ZA`) for exact-match priority.
/// * `show_all`    – when true, return every language; otherwise keep only voices
///                   whose language matches `ui_language` (offline voices are always kept).
pub fn filter_and_sort(
    mut voices: Vec<Voice>,
    ui_language: &str,
    ui_locale: &str,
    show_all: bool,
) -> Vec<Voice> {
    let lang = ui_language.to_lowercase();
    let loc = ui_locale.to_lowercase();

    if !show_all && !lang.is_empty() {
        voices.retain(|v| !v.online || v.language() == lang);
    }

    voices.sort_by(|a, b| {
        // Rank: exact locale match, then same language, then online-before-offline,
        // then alphabetical by display.
        let rank = |v: &Voice| -> u8 {
            if v.online && v.locale.to_lowercase() == loc {
                0
            } else if v.online && v.language() == lang {
                1
            } else if v.online {
                2
            } else {
                3
            }
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.display.to_lowercase().cmp(&b.display.to_lowercase()))
    });

    voices
}
