//! Detect whether the microphone or webcam is currently in use — i.e. whether
//! the user is on a call / in a meeting.
//!
//! Windows records microphone **and** camera access under the *Capability
//! Access Manager* consent store in the registry. For every app that has used
//! the device there is a subkey containing two `FILETIME` values:
//!
//! * `LastUsedTimeStart` – when the app started using the device;
//! * `LastUsedTimeStop`  – when it stopped, **or `0` while it is still active**.
//!
//! So "is the mic/camera in use right now?" reduces to "does any app under the
//! store have `LastUsedTimeStop == 0`?". Packaged (Store) apps sit directly
//! under the device key; classic desktop apps sit under its `NonPackaged`
//! subkey. We also check `HKEY_LOCAL_MACHINE` for apps that record system-wide.
//!
//! This is the most reliable *general* way to tell someone is in a meeting: in
//! a Teams / Slack / Zoom / Meet / Discord call the app holds the microphone
//! open (even while you are muted) and usually the camera too. It needs no
//! per-app hacks, no elevated rights, and never opens an audio/video device.

/// Registry path (relative to the hive root) of the microphone consent store.
#[cfg(windows)]
const MIC_STORE: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// Registry path (relative to the hive root) of the webcam consent store.
#[cfg(windows)]
const CAM_STORE: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam";

/// Returns `true` if any application currently holds the microphone.
#[cfg(windows)]
pub fn microphone_in_use() -> bool {
    store_is_active(MIC_STORE)
}

/// Returns `true` if any application currently holds the webcam/camera.
#[cfg(windows)]
pub fn camera_in_use() -> bool {
    store_is_active(CAM_STORE)
}

/// Returns `true` if the user appears to be on a call / in a meeting, i.e. the
/// microphone **or** the camera is currently in use by any app.
#[cfg(windows)]
pub fn in_meeting() -> bool {
    microphone_in_use() || camera_in_use()
}

/// Non-Windows stubs so the crate still builds/tests on other platforms.
#[cfg(not(windows))]
pub fn microphone_in_use() -> bool {
    false
}
#[cfg(not(windows))]
pub fn camera_in_use() -> bool {
    false
}
#[cfg(not(windows))]
pub fn in_meeting() -> bool {
    false
}

/// Open the given consent store under both HKCU and HKLM and report whether any
/// app inside has an active (in-use) session.
#[cfg(windows)]
fn store_is_active(store_path: &str) -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let root = RegKey::predef(hive);
        if let Ok(store) = root.open_subkey(store_path) {
            if store_has_active_app(&store) {
                return true;
            }
        }
    }
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

/// An app is actively using the device when it has a real start time and its
/// stop time is still `0` (Windows zeroes `LastUsedTimeStop` for the duration
/// of use).
#[cfg(windows)]
fn app_is_active(app: &winreg::RegKey) -> bool {
    let start = app.get_value::<u64, _>("LastUsedTimeStart").unwrap_or(0);
    let stop = app.get_value::<u64, _>("LastUsedTimeStop").unwrap_or(1);
    start > 0 && stop == 0
}
