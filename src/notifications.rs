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

/// Turn an AUMID / handler id into something human-readable for the app list.
pub fn pretty_app_name(primary_id: &str) -> String {
    let mut s = primary_id;

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
                    app_display: pretty_app_name(&primary_id),
                    app_primary_id: primary_id,
                    arrival_time: arrival,
                    text_parts,
                });
                continue;
            }
            out.push(RawNotification {
                app_display: pretty_app_name(&primary_id),
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
        assert_eq!(
            pretty_app_name("5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App"),
            "Whats App Desktop"
        );
        assert_eq!(
            pretty_app_name(r"C:\Program Files\Slack\slack.exe"),
            "slack"
        );
    }
}
