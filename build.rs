//! Build script: on Windows, compile `resources.rc` into the binary so the
//! application manifest (Common Controls v6) and icon are embedded.
//!
//! The manifest is what fixes the startup error
//!   "The procedure entry point GetWindowSubclass could not be located in the
//!    dynamic link library ...":
//! it makes Windows load comctl32.dll v6, which exports the subclassing APIs
//! that native-windows-gui depends on.

fn main() {
    #[cfg(windows)]
    {
        // Rebuild if the manifest or resource script changes.
        println!("cargo:rerun-if-changed=resources.rc");
        println!("cargo:rerun-if-changed=PortableNotificationReader.manifest");
        println!("cargo:rerun-if-changed=assets/app.ico");
        embed_resource::compile("resources.rc", embed_resource::NONE);
    }
}
