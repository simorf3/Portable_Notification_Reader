//! Detects the Windows UI language so we can show voices for the user's language
//! by default (e.g. an English-South-Africa install shows the `en-*` voices first).

/// Returns `(locale, language)` such as `("en-ZA", "en")`. Falls back to `en-US`.
pub fn windows_ui_locale() -> (String, String) {
    let loc = detect();
    let lang = loc.split('-').next().unwrap_or("en").to_lowercase();
    (loc, lang)
}

#[cfg(windows)]
fn detect() -> String {
    use windows::core::PWSTR;
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    // LOCALE_NAME_MAX_LENGTH is 85.
    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if len > 1 {
        // len includes the trailing null terminator.
        let s = String::from_utf16_lossy(&buf[..(len as usize - 1)]);
        if !s.trim().is_empty() {
            return s;
        }
    }
    // Silence unused import warning path.
    let _ = PWSTR::null();
    "en-US".to_string()
}

#[cfg(not(windows))]
fn detect() -> String {
    // On non-Windows (dev/test), approximate from environment.
    std::env::var("LANG")
        .ok()
        .and_then(|l| l.split('.').next().map(|s| s.replace('_', "-")))
        .filter(|s| !s.is_empty() && s != "C" && s != "POSIX")
        .unwrap_or_else(|| "en-US".to_string())
}
