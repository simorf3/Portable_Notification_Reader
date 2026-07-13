//! Reads Windows toast notifications by polling the push-notification SQLite
//! database directly. This keeps the app fully *portable*: no package identity,
//! no WinRT listener registration, no installer – just read the DB Windows already
//! maintains at `%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db`.

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

/// One toast notification pulled from the database.
#[derive(Debug, Clone)]
pub struct RawNotification {
    /// Raw handler id (AUMID / ProgID) exactly as stored.
    pub app_primary_id: String,
    /// Best-effort friendly app name derived from the primary id.
    pub app_display: String,
    /// Windows FILETIME arrival timestamp (100 ns since 1601). Used for ordering.
    pub arrival_time: i64,
    /// The `<text>` elements from the toast payload, in order (title, body, ...).
    pub text_parts: Vec<String>,
}

/// Default path to the per-user notification database.
pub fn default_db_path() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    Some(
        PathBuf::from(local)
            .join("Microsoft")
            .join("Windows")
            .join("Notifications")
            .join("wpndatabase.db"),
    )
}

/// Extract the ordered `<text>` contents from a toast XML payload.
pub fn parse_text_parts(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut parts: Vec<String> = Vec::new();
    let mut in_text = false;
    let mut current = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.name().as_ref() == b"text" => {
                in_text = true;
                current.clear();
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"text" => {
                in_text = false;
                let t = current.trim();
                if !t.is_empty() {
                    parts.push(t.to_string());
                }
            }
            Ok(Event::Text(e)) if in_text => {
                let raw = String::from_utf8_lossy(e.as_ref());
                match quick_xml::escape::unescape(&raw) {
                    Ok(txt) => current.push_str(&txt),
                    Err(_) => current.push_str(&raw),
                }
            }
            Ok(Event::CData(e)) if in_text => {
                if let Ok(s) = std::str::from_utf8(e.as_ref()) {
                    current.push_str(s);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    parts
}

/// Best-effort friendly app name for a notification handler id.
///
/// Order of preference:
///  1. The `DisplayName` Windows itself stores for the app under the per-user
///     notification settings registry key (exactly what you see in
///     *Settings → Notifications*). This resolves PWAs and packaged apps to a
///     real name instead of a raw AUMID.
///  2. A heuristic clean-up of the AUMID / exe path (see [`pretty_app_name`]).
pub fn app_display_name(primary_id: &str) -> String {
    if let Some(name) = registry_display_name(primary_id) {
        let n = name.trim();
        if !n.is_empty() {
            return n.to_string();
        }
    }
    pretty_app_name(primary_id)
}

/// Look up the friendly `DisplayName` Windows records for this handler id.
#[cfg(windows)]
fn registry_display_name(primary_id: &str) -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = format!(
        r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{primary_id}"
    );
    let key = hkcu.open_subkey(path).ok()?;
    // Windows stores the shown name in "DisplayName"; some builds indirect it.
    let name: String = key.get_value("DisplayName").ok()?;
    let name = name.trim();
    if name.is_empty() || name.starts_with("@{") || name.starts_with('@') {
        // "@{Package?ms-resource...}" indirect strings can't be resolved cheaply.
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(not(windows))]
fn registry_display_name(_primary_id: &str) -> Option<String> {
    None
}

/// Turn an AUMID / handler id into something human-readable for the app list.
pub fn pretty_app_name(primary_id: &str) -> String {
    // A few well-known ids map to names Windows won't spell nicely on its own.
    if let Some(known) = known_app_name(primary_id) {
        return known.to_string();
    }

    // Drop any query-string tail (e.g. Edge PWA ids like "...?clientType=pwa").
    let mut s = primary_id.split('?').next().unwrap_or(primary_id).trim();
    if s.is_empty() {
        return "Web app".to_string();
    }

    // A Win32 exe path / ProgID with a path -> file stem (do this first so the
    // ".exe" extension doesn't get mistaken for an AUMID dotted segment). We split
    // manually on both separators so it behaves the same on any host OS.
    if s.contains('\\') || s.contains('/') || s.to_lowercase().ends_with(".exe") {
        let base = s.rsplit(|c| c == '\\' || c == '/').next().unwrap_or(s);
        let stem = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
        return camel_split(stem);
    }

    // "5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App" -> take before '!'
    if let Some(idx) = s.find('!') {
        s = &s[..idx];
    }
    // Drop the publisher hash: "...Desktop_cv1g..." -> "...Desktop"
    if let Some(idx) = s.find('_') {
        s = &s[..idx];
    }
    // Take the last dotted segment: "5319275A.WhatsAppDesktop" -> "WhatsAppDesktop"
    if let Some(idx) = s.rfind('.') {
        s = &s[idx + 1..];
    }

    let out = camel_split(s);
    if out.is_empty() {
        primary_id.to_string()
    } else {
        out
    }
}

/// Friendly names for common ids whose AUMID doesn't clean up nicely.
fn known_app_name(primary_id: &str) -> Option<&'static str> {
    let p = primary_id.to_lowercase();
    const MAP: &[(&str, &str)] = &[
        ("whatsapp", "WhatsApp"),
        ("screensketch", "Snipping Tool"),
        ("microsoft.windows.snip", "Snipping Tool"),
        ("teams", "Microsoft Teams"),
        ("outlook", "Outlook"),
        ("olk.exe", "Outlook"),
        ("windowslive.mail", "Mail"),
        ("microsoft.outlookforwindows", "Outlook (new)"),
        ("microsoft.skypeapp", "Skype"),
        ("telegram", "Telegram"),
        ("discord", "Discord"),
        ("slack", "Slack"),
        ("spotify", "Spotify"),
        ("microsoft.windowsstore", "Microsoft Store"),
        ("windowssecurity", "Windows Security"),
        ("microsoft.xboxapp", "Xbox"),
    ];
    MAP.iter()
        .find(|(needle, _)| p.contains(needle))
        .map(|(_, name)| *name)
}

/// Split simple CamelCase into space-separated words ("WhatsAppDesktop" -> "Whats App Desktop").
fn camel_split(s: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        out.push(c);
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    out.trim().to_string()
}

/// Polls the notification database for new toasts.
pub struct NotificationReader {
    db_path: PathBuf,
    last_seen: i64,
}

impl NotificationReader {
    pub fn new(db_path: PathBuf) -> Self {
        NotificationReader {
            db_path,
            last_seen: i64::MIN,
        }
    }

    /// Open the DB strictly read-only so we never interfere with Windows' writes.
    fn open(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(
            &self.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("opening {}", self.db_path.display()))?;
        // Never write; make sure we don't try to checkpoint the WAL.
        let _ = conn.busy_timeout(std::time::Duration::from_millis(200));
        Ok(conn)
    }

    fn query_since(conn: &Connection, since: i64) -> Result<Vec<RawNotification>> {
        // Notification.Payload holds the toast XML; join to the handler for the app id.
        let mut stmt = conn.prepare(
            "SELECT h.PrimaryId, n.ArrivalTime, n.Payload \
             FROM Notification n \
             JOIN NotificationHandler h ON n.HandlerId = h.RecordId \
             WHERE n.ArrivalTime > ?1 AND n.\"Type\" = 'toast' \
             ORDER BY n.ArrivalTime ASC",
        )?;

        let rows = stmt.query_map([since], |row| {
            let primary_id: String = row.get(0)?;
            let arrival: i64 = row.get(1)?;
            // Payload may be TEXT or BLOB depending on Windows build.
            let payload: Vec<u8> = match row.get_ref(2)? {
                rusqlite::types::ValueRef::Text(t) => t.to_vec(),
                rusqlite::types::ValueRef::Blob(b) => b.to_vec(),
                _ => Vec::new(),
            };
            Ok((primary_id, arrival, payload))
        })?;

        let mut out = Vec::new();
        for r in rows {
            let (primary_id, arrival, payload) = r?;
            let xml = String::from_utf8_lossy(&payload);
            let text_parts = parse_text_parts(&xml);
            if text_parts.is_empty() {
                // Nothing speakable (e.g. image-only toast) – still advance the cursor.
                out.push(RawNotification {
                    app_display: app_display_name(&primary_id),
                    app_primary_id: primary_id,
                    arrival_time: arrival,
                    text_parts,
                });
                continue;
            }
            out.push(RawNotification {
                app_display: app_display_name(&primary_id),
                app_primary_id: primary_id,
                arrival_time: arrival,
                text_parts,
            });
        }
        Ok(out)
    }

    /// Establish the baseline so we only speak notifications that arrive *after*
    /// the app starts (never a backlog of old toasts).
    pub fn initialize_baseline(&mut self) -> Result<()> {
        let conn = self.open()?;
        let max: Option<i64> = conn
            .query_row("SELECT MAX(ArrivalTime) FROM Notification", [], |r| r.get(0))
            .ok()
            .flatten();
        self.last_seen = max.unwrap_or(0);
        Ok(())
    }

    /// Return notifications that arrived since the last poll, advancing the cursor.
    pub fn poll_new(&mut self) -> Result<Vec<RawNotification>> {
        let conn = self.open()?;
        let since = if self.last_seen == i64::MIN {
            0
        } else {
            self.last_seen
        };
        let items = Self::query_since(&conn, since)?;
        if let Some(last) = items.iter().map(|n| n.arrival_time).max() {
            if last > self.last_seen {
                self.last_seen = last;
            }
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_parts_in_order() {
        let xml = r#"<toast><visual><binding template="ToastGeneric">
            <text>WhatsApp</text><text>Alice: hello there</text>
            </binding></visual></toast>"#;
        let parts = parse_text_parts(xml);
        assert_eq!(parts, vec!["WhatsApp", "Alice: hello there"]);
    }

    #[test]
    fn pretty_names() {
        // Well-known ids resolve through the friendly-name map.
        assert_eq!(
            pretty_app_name("5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App"),
            "WhatsApp"
        );
        assert_eq!(
            pretty_app_name(r"C:\Program Files\Slack\slack.exe"),
            "Slack"
        );
    }

    #[test]
    fn pretty_names_heuristics() {
        // Unknown packaged AUMID -> CamelCase split of the last segment.
        assert_eq!(
            pretty_app_name("1234ABCD.SomeCoolApp_abcdef123456!App"),
            "Some Cool App"
        );
        // Unknown Win32 exe -> file stem.
        assert_eq!(
            pretty_app_name(r"C:\Tools\MyTool\mytool.exe"),
            "mytool"
        );
        // Edge PWA style id with a query tail is stripped.
        assert_eq!(
            pretty_app_name("MSEdge.abc123?clientType=pwa"),
            "abc123"
        );
    }
}
