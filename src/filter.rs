//! Turns a raw notification into the exact words to speak, and decides whether it
//! should be spoken at all (per-app mute + user filter rules).

use crate::config::{FilterRule, ReplaceRule};
use once_cell::sync::Lazy;
use regex::{escape, Regex};

/// Matches a "Sender: message" prefix used by WhatsApp *group* toasts, where the
/// body already names who spoke (e.g. "Alice: are we still on?").
static SENDER_PREFIX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[^:\r\n]{1,40}:\s").unwrap());

/// Matches URLs (http/https) to replace with friendlier spoken text.
static URL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"https?://[^\s]+").unwrap());

fn looks_like_group_message(body: &str) -> bool {
    SENDER_PREFIX.is_match(body)
}

/// Extract a friendly domain name from a URL for speech.
/// Examples:
///   https://www.instagram.com/reel/... → "instagram"
///   https://youtu.be/xAIeA3ewRbo       → "youtube"
///   https://github.com/user/repo       → "github"
fn extract_domain(url: &str) -> String {
    // Strip protocol
    let after_protocol = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    // Take the part before the first '/' or '?' (the hostname)
    let hostname = after_protocol
        .split(&['/', '?'][..])
        .next()
        .unwrap_or(after_protocol);

    // Strip "www." prefix
    let without_www = hostname.strip_prefix("www.").unwrap_or(hostname);

    // Normalize common short domains
    let normalized = match without_www {
        "youtu.be" => "youtube",
        "goo.gl" => "google",
        "bit.ly" => "bitly",
        "t.co" => "twitter",
        _ => without_www,
    };

    // Take just the main domain (drop TLD and subdomains for simplicity)
    normalized
        .split('.')
        .next()
        .unwrap_or(normalized)
        .to_string()
}

/// Replace URLs in text with "domain link" for friendlier speech.
/// Example: "Check this https://youtu.be/abc123" → "Check this youtube link"
fn clean_urls(text: &str) -> String {
    URL_PATTERN
        .replace_all(text, |caps: &regex::Captures| {
            let url = &caps[0];
            let domain = extract_domain(url);
            format!("{} link", domain)
        })
        .to_string()
}

/// Build the spoken string from a notification's text parts.
///
/// Rules (matching the original app's behaviour):
///  * The app name is **never** spoken – the message content makes the source obvious.
///  * For a WhatsApp **group** (the body starts with "Someone: ..."), the first text
///    part is the group name and is skipped; we speak only the sender-prefixed body.
///  * For a 1-on-1 chat / generic toast, the first part (contact/title) is kept.
///  * URLs are replaced with "domain link" (e.g. "https://youtu.be/abc" → "youtube link").
pub fn build_spoken_text(parts: &[String]) -> String {
    let raw = match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        _ => {
            let title = &parts[0];
            let bodies = &parts[1..];
            if looks_like_group_message(&bodies[0]) {
                // Group: drop the group-name title, speak the sender-prefixed body.
                bodies.join(". ")
            } else {
                let mut s = title.clone();
                for b in bodies {
                    s.push_str(". ");
                    s.push_str(b);
                }
                s
            }
        }
    };
    clean_urls(&raw)
}

/// Apply the user's text filtering & replacement rules to `text`, in order.
/// Plain patterns match case-insensitively; an empty replacement deletes the
/// match (so a rule with a blank replacement acts as a plain filter).
pub fn apply_replacements(text: &str, rules: &[ReplaceRule]) -> String {
    let mut out = text.to_string();
    for r in rules {
        if r.pattern.trim().is_empty() {
            continue;
        }
        let re = if r.is_regex {
            Regex::new(&r.pattern)
        } else {
            Regex::new(&format!("(?i){}", escape(&r.pattern)))
        };
        if let Ok(re) = re {
            // `$` is special in the replacement; callers of the plain (non-regex)
            // path expect a literal replacement, so escape it there.
            let replacement = if r.is_regex {
                r.replacement.clone()
            } else {
                r.replacement.replace('$', "$$")
            };
            out = re.replace_all(&out, replacement.as_str()).to_string();
        }
    }
    out
}

fn rule_matches(rule: &FilterRule, haystack: &str) -> bool {
    if rule.pattern.trim().is_empty() {
        return false;
    }
    if rule.is_regex {
        match Regex::new(&rule.pattern) {
            Ok(re) => re.is_match(haystack),
            Err(_) => false, // invalid regex never matches
        }
    } else {
        haystack
            .to_lowercase()
            .contains(&rule.pattern.to_lowercase())
    }
}

/// Decide whether a notification should be spoken given the filter rules.
///
/// Semantics:
///  * Any matching **block** rule silences the notification.
///  * If at least one **allow** rule exists, the app runs in allow-list mode:
///    only notifications matching an allow rule are spoken.
///  * With no allow rules, everything not blocked is spoken.
pub fn passes_filters(app: &str, spoken_or_full_text: &str, rules: &[FilterRule]) -> bool {
    let haystack = format!("{app}\n{spoken_or_full_text}");

    // Block rules win outright.
    for r in rules.iter().filter(|r| r.block) {
        if rule_matches(r, &haystack) {
            return false;
        }
    }

    let allow_rules: Vec<&FilterRule> = rules.iter().filter(|r| !r.block).collect();
    if allow_rules.is_empty() {
        return true;
    }
    allow_rules.iter().any(|r| rule_matches(r, &haystack))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_message_skips_group_name() {
        let parts = vec![
            "Family Group".to_string(),
            "Mom: dinner at 7?".to_string(),
        ];
        assert_eq!(build_spoken_text(&parts), "Mom: dinner at 7?");
    }

    #[test]
    fn one_on_one_keeps_contact() {
        let parts = vec!["Alice".to_string(), "see you soon".to_string()];
        assert_eq!(build_spoken_text(&parts), "Alice. see you soon");
    }

    #[test]
    fn single_part_spoken_as_is() {
        let parts = vec!["Battery low".to_string()];
        assert_eq!(build_spoken_text(&parts), "Battery low");
    }

    #[test]
    fn block_rule_silences() {
        let rules = vec![FilterRule {
            pattern: "spam".into(),
            is_regex: false,
            block: true,
        }];
        assert!(!passes_filters("MailApp", "you won spam prize", &rules));
        assert!(passes_filters("MailApp", "hello", &rules));
    }

    #[test]
    fn allow_list_mode() {
        let rules = vec![FilterRule {
            pattern: "WhatsApp".into(),
            is_regex: false,
            block: false,
        }];
        assert!(passes_filters("WhatsApp", "hi", &rules));
        assert!(!passes_filters("Email", "hi", &rules));
    }

    #[test]
    fn url_instagram_replaced_with_domain() {
        let parts = vec!["Check this out https://www.instagram.com/reel/DauPDIGFJVX/?igsh=MXE3Mjk4em55Nzg4cQ==".to_string()];
        assert_eq!(build_spoken_text(&parts), "Check this out instagram link");
    }

    #[test]
    fn url_youtube_short_domain() {
        let parts = vec!["Watch https://youtu.be/xAIeA3ewRbo".to_string()];
        assert_eq!(build_spoken_text(&parts), "Watch youtube link");
    }

    #[test]
    fn multiple_urls_in_message() {
        let parts = vec!["See https://github.com/user/repo and https://google.com/search?q=test".to_string()];
        assert_eq!(build_spoken_text(&parts), "See github link and google link");
    }

    #[test]
    fn url_with_query_params() {
        let parts = vec!["Link: https://www.instagram.com/reel/abc/?igsh=xyz".to_string()];
        assert_eq!(build_spoken_text(&parts), "Link: instagram link");
    }

    #[test]
    fn replacement_empty_deletes_case_insensitive() {
        // A rule with a blank replacement acts as a plain "filter".
        let rules = vec![ReplaceRule { pattern: "URGENT:".into(), replacement: "".into(), is_regex: false }];
        assert_eq!(apply_replacements("urgent: call me", &rules), " call me");
    }

    #[test]
    fn replacement_regex_delete() {
        let rules = vec![ReplaceRule { pattern: r"\[.*?\]".into(), replacement: "".into(), is_regex: true }];
        assert_eq!(apply_replacements("[work] hello [tag]", &rules), " hello ");
    }

    #[test]
    fn replacement_substitutes_word() {
        let rules = vec![ReplaceRule { pattern: "u".into(), replacement: "you".into(), is_regex: false }];
        // case-insensitive literal: both "U" and "u" replaced
        assert_eq!(apply_replacements("cU soon", &rules), "cyou soon");
    }

    #[test]
    fn replacement_empty_deletes() {
        let rules = vec![ReplaceRule { pattern: "spam".into(), replacement: "".into(), is_regex: false }];
        assert_eq!(apply_replacements("no spam here", &rules), "no  here");
    }

    #[test]
    fn replacement_regex_with_groups() {
        let rules = vec![ReplaceRule { pattern: r"(\d+)%".into(), replacement: "$1 percent".into(), is_regex: true }];
        assert_eq!(apply_replacements("battery 80%", &rules), "battery 80 percent");
    }

    #[test]
    fn invalid_regex_is_skipped() {
        let rules = vec![ReplaceRule { pattern: "(".into(), replacement: "".into(), is_regex: true }];
        // bad regex must not panic and must leave text untouched
        assert_eq!(apply_replacements("hello (", &rules), "hello (");
    }
}
