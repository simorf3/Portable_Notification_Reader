//! Turns a raw notification into the exact words to speak, and decides whether it
//! should be spoken at all (per-app mute + user filter rules).

use crate::config::FilterRule;
use once_cell::sync::Lazy;
use regex::Regex;

/// Matches a "Sender: message" prefix used by WhatsApp *group* toasts, where the
/// body already names who spoke (e.g. "Alice: are we still on?").
static SENDER_PREFIX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[^:\r\n]{1,40}:\s").unwrap());

fn looks_like_group_message(body: &str) -> bool {
    SENDER_PREFIX.is_match(body)
}

/// Build the spoken string from a notification's text parts.
///
/// Rules (matching the original app's behaviour):
///  * The app name is **never** spoken – the message content makes the source obvious.
///  * For a WhatsApp **group** (the body starts with "Someone: ..."), the first text
///    part is the group name and is skipped; we speak only the sender-prefixed body.
///  * For a 1-on-1 chat / generic toast, the first part (contact/title) is kept.
pub fn build_spoken_text(parts: &[String]) -> String {
    match parts.len() {
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
    }
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
}
