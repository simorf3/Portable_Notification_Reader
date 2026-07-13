//! Text shaping applied to a message *after* the core spoken text is built but
//! *before* it is handed to the speech engine.
//!
//! Three transformations live here:
//!  * **Emoji handling** – either strip emojis out entirely (default) or replace
//!    each with a spoken description of its meaning (e.g. 😀 → "grinning face").
//!    Meanings come from the Unicode CLDR names shipped by the `emojis` crate,
//!    so coverage is comprehensive and accurate without hand-maintaining a list.
//!  * **Shorthand expansion** – common chat/text abbreviations are expanded to
//!    the full phrase so they are read naturally (e.g. "brb" → "be right back").
//!  * Tidy-up of the whitespace left behind by the two steps above.

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

/// Common texting / chat shorthand → the words we actually want spoken.
/// Matched case-insensitively on whole words only (see [`expand_shorthand`]).
const SHORTHAND: &[(&str, &str)] = &[
    ("lol", "laughing out loud"),
    ("lmao", "laughing my ass off"),
    ("rofl", "rolling on the floor laughing"),
    ("brb", "be right back"),
    ("btw", "by the way"),
    ("omg", "oh my god"),
    ("omw", "on my way"),
    ("idk", "I don't know"),
    ("idc", "I don't care"),
    ("imo", "in my opinion"),
    ("imho", "in my humble opinion"),
    ("tbh", "to be honest"),
    ("fyi", "for your information"),
    ("asap", "as soon as possible"),
    ("ttyl", "talk to you later"),
    ("np", "no problem"),
    ("nvm", "never mind"),
    ("thx", "thanks"),
    ("ty", "thank you"),
    ("tysm", "thank you so much"),
    ("yw", "you're welcome"),
    ("pls", "please"),
    ("plz", "please"),
    ("gtg", "got to go"),
    ("g2g", "got to go"),
    ("wyd", "what you doing"),
    ("wbu", "what about you"),
    ("hbu", "how about you"),
    ("wby", "what about you"),
    ("smh", "shaking my head"),
    ("ikr", "I know right"),
    ("jk", "just kidding"),
    ("dm", "direct message"),
    ("bff", "best friend forever"),
    ("gg", "good game"),
    ("gn", "good night"),
    ("gm", "good morning"),
    ("hru", "how are you"),
    ("ily", "I love you"),
    ("ilu", "I love you"),
    ("cya", "see you"),
    ("cu", "see you"),
    ("bc", "because"),
    ("cuz", "because"),
    ("b4", "before"),
    ("gr8", "great"),
    ("l8r", "later"),
    ("m8", "mate"),
    ("w8", "wait"),
    ("2day", "today"),
    ("2moro", "tomorrow"),
    ("2nite", "tonight"),
    ("rn", "right now"),
    ("af", "as hell"),
    ("aka", "also known as"),
    ("eta", "estimated time of arrival"),
    ("diy", "do it yourself"),
    ("faq", "frequently asked questions"),
    ("rsvp", "please respond"),
    ("tba", "to be announced"),
    ("tbd", "to be determined"),
    ("tbc", "to be confirmed"),
    ("ppl", "people"),
    ("msg", "message"),
    ("info", "information"),
    ("congrats", "congratulations"),
    ("appt", "appointment"),
    ("mins", "minutes"),
    ("sec", "second"),
    ("wfh", "working from home"),
    ("ootd", "outfit of the day"),
    ("irl", "in real life"),
    ("dw", "don't worry"),
    ("hmu", "hit me up"),
    ("ez", "easy"),
    ("fomo", "fear of missing out"),
    ("ftw", "for the win"),
    ("goat", "greatest of all time"),
    ("idgaf", "I don't give a damn"),
    ("istg", "I swear to god"),
    ("lmk", "let me know"),
    ("nbd", "no big deal"),
    ("otw", "on the way"),
    ("sus", "suspicious"),
    ("tldr", "too long didn't read"),
    ("yolo", "you only live once"),
    ("bday", "birthday"),
];

static SHORTHAND_MAP: Lazy<HashMap<String, &'static str>> = Lazy::new(|| {
    SHORTHAND
        .iter()
        .map(|(k, v)| (k.to_lowercase(), *v))
        .collect()
});

/// Matches "word-ish" tokens: letters/digits/apostrophes. Everything else
/// (spaces, punctuation, emoji) is preserved verbatim between matches.
static WORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[A-Za-z0-9']+").unwrap());

/// Collapses runs of whitespace into a single space and trims the ends.
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s{2,}").unwrap());

/// Expand chat/text shorthand to full words (whole-word, case-insensitive).
pub fn expand_shorthand(text: &str) -> String {
    WORD_RE
        .replace_all(text, |caps: &regex::Captures| {
            let tok = &caps[0];
            match SHORTHAND_MAP.get(&tok.to_lowercase()) {
                Some(full) => (*full).to_string(),
                None => tok.to_string(),
            }
        })
        .to_string()
}

/// Is this grapheme cluster an emoji (or emoji sequence)?
fn is_emoji_cluster(cluster: &str) -> bool {
    emojis::get(cluster).is_some()
}

/// Handle emojis in `text`.
///  * `speak = false` (default): remove emojis entirely.
///  * `speak = true`: replace each emoji with a spoken description of its
///    meaning, e.g. 🎉 → " party popper ".
pub fn handle_emojis(text: &str, speak: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for cluster in text.graphemes(true) {
        if is_emoji_cluster(cluster) {
            if speak {
                if let Some(e) = emojis::get(cluster) {
                    out.push(' ');
                    out.push_str(e.name());
                    out.push(' ');
                }
            }
            // when not speaking, simply drop the emoji
        } else {
            out.push_str(cluster);
        }
    }
    out
}

/// Collapse extra whitespace produced by stripping/expanding and trim ends.
pub fn tidy_whitespace(text: &str) -> String {
    WS_RE.replace_all(text.trim(), " ").to_string()
}

/// Apply shorthand expansion + emoji handling + whitespace tidy-up in order.
pub fn shape(text: &str, speak_emojis: bool) -> String {
    let expanded = expand_shorthand(text);
    let emoji_done = handle_emojis(&expanded, speak_emojis);
    tidy_whitespace(&emoji_done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_whole_word_only() {
        assert_eq!(expand_shorthand("brb dinner"), "be right back dinner");
        // "lol" inside another word must not be expanded
        assert_eq!(expand_shorthand("lollipop"), "lollipop");
    }

    #[test]
    fn shorthand_case_insensitive() {
        assert_eq!(expand_shorthand("OMG really"), "oh my god really");
        assert_eq!(expand_shorthand("Ty so much"), "thank you so much");
    }

    #[test]
    fn emojis_removed_by_default() {
        assert_eq!(tidy_whitespace(&handle_emojis("Hello 😀 world", false)), "Hello world");
        assert_eq!(tidy_whitespace(&handle_emojis("party time 🎉🎉", false)), "party time");
    }

    #[test]
    fn emojis_spoken_when_enabled() {
        let out = tidy_whitespace(&handle_emojis("nice 🎉", true));
        assert_eq!(out, "nice party popper");
    }

    #[test]
    fn emoji_meaning_grinning() {
        let out = tidy_whitespace(&handle_emojis("😀", true));
        assert_eq!(out, "grinning face");
    }

    #[test]
    fn shape_combines_everything() {
        // shorthand expanded, emoji stripped (default off)
        assert_eq!(shape("omw 🚗 brb", false), "on my way be right back");
        // emoji spoken when enabled
        assert_eq!(shape("gg 🎉", true), "good game party popper");
    }
}
