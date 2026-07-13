//! Online neural TTS via Microsoft Edge's free "read aloud" WebSocket endpoint.
//!
//! Protocol (same one Edge uses):
//!   1. open a WSS connection with the trusted-client token + Sec-MS-GEC auth,
//!   2. send a `speech.config` text frame choosing the MP3 output format,
//!   3. send an `ssml` text frame with the voice + text,
//!   4. read back binary frames (2-byte big-endian header length, header, audio)
//!      until a `Path:turn.end` text frame arrives.
//!
//! Returns 24 kHz 48 kbit mono MP3 bytes.

use crate::drm;
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const WSS_BASE: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
const OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn now_string() -> String {
    // Format expected by the service, e.g. "Mon Jul 13 2026 12:00:00 GMT+0000 (Coordinated Universal Time)".
    chrono::Utc::now()
        .format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
        .to_string()
}

/// Synthesise `text` with the given Edge voice short-name (e.g. `en-ZA-LeahNeural`).
/// `rate_percent` is a relative speed offset (-100..100). Returns MP3 bytes.
pub async fn synthesize(voice_short_name: &str, text: &str, rate_percent: i32) -> Result<Vec<u8>> {
    let connection_id = Uuid::new_v4().simple().to_string();
    let sec = drm::generate_sec_ms_gec();
    let url = format!(
        "{WSS_BASE}?TrustedClientToken={}&Sec-MS-GEC={}&Sec-MS-GEC-Version={}&ConnectionId={}",
        drm::TRUSTED_CLIENT_TOKEN,
        sec,
        drm::SEC_MS_GEC_VERSION,
        connection_id
    );

    let mut request = url.into_client_request()?;
    {
        let headers = request.headers_mut();
        headers.insert("Origin", HeaderValue::from_static(drm::ORIGIN));
        headers.insert("User-Agent", HeaderValue::from_static(drm::USER_AGENT));
        headers.insert("Pragma", HeaderValue::from_static("no-cache"));
        headers.insert("Cache-Control", HeaderValue::from_static("no-cache"));
        headers.insert("Accept-Encoding", HeaderValue::from_static("gzip, deflate, br"));
        headers.insert("Accept-Language", HeaderValue::from_static("en-US,en;q=0.9"));
        // The MUID cookie is now required by the endpoint (403 without it).
        if let Ok(cookie) = HeaderValue::from_str(&drm::muid_cookie()) {
            headers.insert("Cookie", cookie);
        }
    }

    let (mut ws, _resp) = connect_async(request).await?;

    // 1) speech.config
    let config = format!(
        "X-Timestamp:{ts}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"{fmt}\"}}}}}}}}",
        ts = now_string(),
        fmt = OUTPUT_FORMAT
    );
    ws.send(Message::Text(config.into())).await?;

    // 2) ssml
    let rate = if rate_percent >= 0 {
        format!("+{rate_percent}%")
    } else {
        format!("{rate_percent}%")
    };
    let request_id = Uuid::new_v4().simple().to_string();
    let ssml = format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{voice}'><prosody pitch='+0Hz' rate='{rate}' volume='+0%'>{text}</prosody></voice></speak>",
        voice = voice_short_name,
        rate = rate,
        text = xml_escape(text)
    );
    let ssml_msg = format!(
        "X-RequestId:{rid}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{ts}Z\r\nPath:ssml\r\n\r\n{ssml}",
        rid = request_id,
        ts = now_string(),
        ssml = ssml
    );
    ws.send(Message::Text(ssml_msg.into())).await?;

    // 3) read audio until turn.end
    let mut audio: Vec<u8> = Vec::new();
    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Binary(data) => {
                if data.len() < 2 {
                    continue;
                }
                let header_len = ((data[0] as usize) << 8) | (data[1] as usize);
                let start = 2 + header_len;
                if start <= data.len() {
                    audio.extend_from_slice(&data[start..]);
                }
            }
            Message::Text(t) => {
                if t.contains("Path:turn.end") {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let _ = ws.close(None).await;

    if audio.is_empty() {
        return Err(anyhow!("no audio returned from Edge TTS"));
    }
    Ok(audio)
}
