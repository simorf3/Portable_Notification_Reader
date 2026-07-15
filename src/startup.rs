//! "Start automatically when I sign in" support.
//!
//! On first run the app offers to create a shortcut in the current user's
//! Windows **Startup** folder so it launches every time you log in. We never
//! ask again once a shortcut exists (or once the first-run prompt has been
//! shown), so the user is only prompted a single time.

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

    /// File name of the shortcut we create in the Startup folder.
    const LNK_NAME: &str = "Portable Notification Reader.lnk";

    /// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`.
    fn startup_dir() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|appdata| {
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Startup")
        })
    }

    /// Full path to the shortcut we manage.
    pub fn shortcut_path() -> Option<PathBuf> {
        startup_dir().map(|d| d.join(LNK_NAME))
    }

    /// True if a startup shortcut for this app already exists.
    pub fn shortcut_exists() -> bool {
        shortcut_path().map(|p| p.exists()).unwrap_or(false)
    }

    /// Create the Startup-folder shortcut pointing at the running executable.
    pub fn create_shortcut() -> anyhow::Result<()> {
        let exe = std::env::current_exe()?;
        let target = shortcut_path()
            .ok_or_else(|| anyhow::anyhow!("could not locate the Startup folder (APPDATA unset)"))?;
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let link = mslnk::ShellLink::new(&exe)
            .map_err(|e| anyhow::anyhow!("building shortcut for {}: {e}", exe.display()))?;
        link.create_lnk(&target)
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", target.display()))?;
        log::info!("created startup shortcut at {}", target.display());
        Ok(())
    }

    /// Show a Yes/No message box. Returns true if the user chose "Yes".
    fn ask_yes_no(title: &str, text: &str) -> bool {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_ICONQUESTION, MB_YESNO,
        };
        let res = unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(text),
                &HSTRING::from(title),
                MB_YESNO | MB_ICONQUESTION,
            )
        };
        res == IDYES
    }

    /// Show a simple information / error message box.
    fn info_box(title: &str, text: &str, is_error: bool) {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
        };
        let style = MB_OK | if is_error { MB_ICONERROR } else { MB_ICONINFORMATION };
        unsafe {
            MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(title), style);
        }
    }

    /// Called once on first run. If no startup shortcut exists yet, ask the user
    /// whether the app should launch automatically at sign-in, and create the
    /// shortcut if they agree. Does nothing (and never prompts) if a shortcut is
    /// already present.
    pub fn prompt_on_first_run() {
        if shortcut_exists() {
            return; // already set up – never nag.
        }
        let wants_it = ask_yes_no(
            "Portable Notification Reader",
            "Start Portable Notification Reader automatically when you sign in to Windows?\n\n\
             (You can change this later by deleting the shortcut from your Startup folder.)",
        );
        if !wants_it {
            return;
        }
        match create_shortcut() {
            Ok(()) => info_box(
                "Portable Notification Reader",
                "Done! The app will now start automatically when you sign in.",
                false,
            ),
            Err(e) => info_box(
                "Portable Notification Reader",
                &format!("Sorry, the startup shortcut could not be created:\n\n{e}"),
                true,
            ),
        }
    }
}

#[cfg(windows)]
pub use imp::{create_shortcut, prompt_on_first_run, shortcut_exists, shortcut_path};

// ---- Non-Windows stub so the crate still builds on Linux for checks ----
#[cfg(not(windows))]
pub fn prompt_on_first_run() {}

#[cfg(not(windows))]
pub fn shortcut_exists() -> bool {
    false
}
