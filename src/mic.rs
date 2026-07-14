//! Detect whether the microphone is currently in use by any application.
//!
//! Windows records microphone access under the *Capability Access Manager*
//! consent store in the registry. For every app that has used the mic there is
//! a subkey containing two `FILETIME` values:
//!
//! * `LastUsedTimeStart` – when the app started using the mic;
//! * `LastUsedTimeStop`  – when it stopped, **or `0` while it is still active**.
//!
//! So "is the mic in use right now?" reduces to "does any app under the store
//! have `LastUsedTimeStop == 0`?". Packaged (Store) apps sit directly under the
//! `microphone` key; classic desktop apps sit under its `NonPackaged` subkey.
//! We also check `HKEY_LOCAL_MACHINE` for apps that record system-wide.
//!
//! This is read-only registry polling – no elevated rights, no audio device is
//! opened, and it works for every app the OS knows about.

/// Registry path (relative to the hive root) of the microphone consent store.
#[cfg(windows)]
const MIC_STORE: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// Returns `true` if any application currently holds the microphone.
#[cfg(windows)]
pub fn microphone_in_use() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let root = RegKey::predef(hive);
        if let Ok(store) = root.open_subkey(MIC_STORE) {
            if store_has_active_app(&store) {
                return true;
            }
        }
    }
    false
}

/// Non-Windows stub so the crate still builds/tests on other platforms.
#[cfg(not(windows))]
pub fn microphone_in_use() -> bool {
    false
}

/// Scan every app subkey (and the `NonPackaged` group) for an active session.
#[cfg(windows)]
fn store_has_active_app(store: &winreg::RegKey) -> bool {
    for name in store.enum_keys().flatten() {
        let sub = match store.open_subkey(&name) {
            Ok(k) => k,
            Err(_) => continue,
        };

        if name.eq_ignore_ascii_case("NonPackaged") {
            // Classic desktop apps live one level deeper.
            for np in sub.enum_keys().flatten() {
                if let Ok(app) = sub.open_subkey(&np) {
                    if app_is_active(&app) {
                        return true;
                    }
                }
            }
        } else if app_is_active(&sub) {
            return true;
        }
    }
    false
}

/// An app is actively using the mic when it has a real start time and its stop
/// time is still `0` (Windows zeroes `LastUsedTimeStop` for the duration of use).
#[cfg(windows)]
fn app_is_active(app: &winreg::RegKey) -> bool {
    let start = app.get_value::<u64, _>("LastUsedTimeStart").unwrap_or(0);
    let stop = app.get_value::<u64, _>("LastUsedTimeStop").unwrap_or(1);
    start > 0 && stop == 0
}
