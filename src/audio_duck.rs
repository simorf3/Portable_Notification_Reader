//! Temporarily "duck" (lower) the volume of *other* apps while a notification
//! is read aloud, so the app never talks over a YouTube video, a song or a
//! game. This is our take on Windows' communications-style ducking.
//!
//! Why this approach: the audio library we use (`rodio`) does not let us tag our
//! own speech stream as a "communications" stream, which is what would make
//! Windows duck everything else automatically. Instead we do the ducking
//! ourselves through the Core Audio session APIs: right before speaking we walk
//! every *other* app's audio session, remember its current volume and turn it
//! down; a [`DuckGuard`] restores every volume the instant it is dropped (i.e.
//! as soon as the notification finishes). Our own process is never touched, so
//! the spoken notification stays at full volume.
//!
//! This needs no admin rights and only changes volumes for the moment we speak.

/// Volume (0.0..=1.0) other apps are lowered to while we speak. 0.2 = 20 %.
pub const DUCK_LEVEL: f32 = 0.2;

#[cfg(windows)]
mod imp {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioSessionControl2, IAudioSessionEnumerator, IAudioSessionManager2,
        IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Threading::GetCurrentProcessId;

    /// Restores every ducked app's volume when it goes out of scope.
    pub struct DuckGuard {
        saved: Vec<(ISimpleAudioVolume, f32)>,
    }

    impl Drop for DuckGuard {
        fn drop(&mut self) {
            for (vol, level) in &self.saved {
                unsafe {
                    let _ = vol.SetMasterVolume(*level, std::ptr::null());
                }
            }
        }
    }

    /// Lower every other app's audio session to `level` (0.0..=1.0) and return a
    /// guard that restores them when dropped. Returns `None` if nothing could be
    /// ducked (no other audio, or the APIs were unavailable) – speaking then
    /// simply proceeds without ducking.
    pub fn duck_other_apps(level: f32) -> Option<DuckGuard> {
        unsafe {
            // The worker thread may not have COM initialised yet; safe to call
            // repeatedly (returns S_FALSE when already initialised).
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
            let sessions: IAudioSessionEnumerator = manager.GetSessionEnumerator().ok()?;
            let count = sessions.GetCount().ok()?;
            let our_pid = GetCurrentProcessId();

            let mut saved: Vec<(ISimpleAudioVolume, f32)> = Vec::new();
            for i in 0..count {
                let ctrl = match sessions.GetSession(i) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let ctrl2: IAudioSessionControl2 = match ctrl.cast() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                // Skip the system-sounds session (pid 0) and our own speech.
                let pid = ctrl2.GetProcessId().unwrap_or(0);
                if pid == 0 || pid == our_pid {
                    continue;
                }
                let vol: ISimpleAudioVolume = match ctrl2.cast() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let current = vol.GetMasterVolume().unwrap_or(1.0);
                // Only duck apps that are actually audible right now.
                if current <= level {
                    continue;
                }
                if vol.SetMasterVolume(level, std::ptr::null()).is_ok() {
                    saved.push((vol, current));
                }
            }

            if saved.is_empty() {
                None
            } else {
                Some(DuckGuard { saved })
            }
        }
    }
}

/// Non-Windows stub so the crate still builds/tests on other platforms.
#[cfg(not(windows))]
mod imp {
    pub struct DuckGuard;
    pub fn duck_other_apps(_level: f32) -> Option<DuckGuard> {
        None
    }
}

pub use imp::{duck_other_apps, DuckGuard};
