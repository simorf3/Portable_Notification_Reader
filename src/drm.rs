//! Auth-token generation for Microsoft's free online Edge "read aloud" neural TTS.
//!
//! The public endpoint is gated by a `Sec-MS-GEC` token: the uppercase SHA-256 of
//! `"{windows_ticks}{TRUSTED_CLIENT_TOKEN}"`, where `windows_ticks` is the current
//! time in Windows-epoch 100-nanosecond units rounded down to the last 5 minutes.
//! This matches the well-known algorithm used by Microsoft Edge itself.

use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// Public "trusted client" token shipped inside Microsoft Edge.
pub const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";

/// Edge/Chromium version string reported alongside the token.
pub const SEC_MS_GEC_VERSION: &str = "1-143.0.3650.75";

/// Origin header value Edge uses for the read-aloud endpoints.
pub const ORIGIN: &str = "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold";

/// User-Agent string matching the reported Edge version.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";

/// Seconds between the Windows epoch (1601-01-01) and the Unix epoch (1970-01-01).
const WIN_EPOCH_OFFSET_SECS: u64 = 11_644_473_600;

/// Generate the `Sec-MS-GEC` auth token for the current time.
pub fn generate_sec_ms_gec() -> String {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Move to the Windows epoch, then round down to the previous 5-minute boundary.
    let mut ticks_secs = unix_secs + WIN_EPOCH_OFFSET_SECS;
    ticks_secs -= ticks_secs % 300;

    // Convert seconds -> 100-nanosecond units (1 s = 10_000_000 * 100 ns).
    let ticks_100ns: u128 = (ticks_secs as u128) * 10_000_000;

    let to_hash = format!("{}{}", ticks_100ns, TRUSTED_CLIENT_TOKEN);

    let mut hasher = Sha256::new();
    hasher.update(to_hash.as_bytes());
    let digest = hasher.finalize();
    hex::encode_upper(digest)
}

/// Generate a random MUID (32 uppercase hex chars) for the `Cookie: muid=...` header,
/// which the endpoints now require.
pub fn generate_muid() -> String {
    let bytes: [u8; 16] = uuid::Uuid::new_v4().into_bytes();
    hex::encode_upper(bytes)
}

/// The `Cookie` header value carrying the MUID.
pub fn muid_cookie() -> String {
    format!("muid={};", generate_muid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex_uppercase() {
        let t = generate_sec_ms_gec();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
    }
}
