# Portable Notification Reader

Reads your **Windows 11 notifications aloud** using natural, online neural voices
(with an offline fallback), controlled entirely from a small **system-tray icon**.

It is a ground-up **Rust** rewrite of the original C# app, redesigned to be
**100% portable**:

- A **single `.exe`** — no installer, no MSIX package, no admin rights.
- All settings live in **`config.json` next to the executable**. Nothing is
  written to the registry or to AppData. Copy the folder to a USB stick and it
  runs anywhere.
- Notifications are read by **polling the Windows notification database directly**
  (`%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db`), so the app
  needs **no package identity** and no `UserNotificationListener` registration —
  which is exactly what makes a portable, unpackaged build possible.

---

## Features

- **Speaks new toast notifications** as they arrive (polls once per second by
  default; configurable).
- **Full online neural voice catalogue** (Microsoft Edge voices) fetched live,
  including the **South-African voices**: `en-ZA` (Leah, Luke),
  `af-ZA` (Adri, Willem) and `zu-ZA` (Thando, Themba).
- **Voices filtered by your Windows display language** by default (e.g. an
  English-South-Africa install shows the English voices first), with a
  **“Show all languages”** toggle to reveal the entire catalogue.
- **Offline SAPI voices** are always available and are used automatically as a
  fallback when there is no internet connection.
- **Volume with loudness boost** — 0–200%. Values above 100% amplify the audio
  *louder than the system default* (online voices only).
- **Speaking speed** control.
- **Per-app mute** — any app that has sent a notification appears in the
  *Disable apps* menu and can be muted/unmuted with a click.
- **Filter rules** — block or allow notifications by substring or regex.
- **Text filtering and replacement** — remove or replace words/phrases before
  speaking (plain text or regex). Leave the replacement blank to simply delete
  the matched text.
- **Emoji handling** — toggle **Speak emojis** to have emoji meanings spoken
  (e.g. 🎉 → "party popper"), or leave it off to strip them silently. Full
  Unicode CLDR coverage including multi-part sequences (skin tones, ZWJ families).
- **Shorthand expansion** — common chat abbreviations (lol, brb, omg, etc.) are
  automatically expanded to full words before speaking.
- **Smart speech shaping** (matches the original app):
  - the **app name is never read aloud**;
  - for a **WhatsApp group**, the group name is skipped and only the
    “Sender: message” body is read;
  - for a one-on-one chat the contact/title is kept;
  - **URLs are replaced** with friendly "domain link" text (e.g. "instagram link").
- Left-click **or** right-click the tray icon to open the menu.

---

## Download / Build

This is a Windows GUI app and is built by **GitHub Actions** on a Windows
runner (the authoritative build), because it uses Win32 / WinRT APIs that can’t
be compiled on Linux/macOS.

1. Push to `main` (or use the **Actions → Build Windows executable → Run
   workflow** button). The workflow compiles a release build and uploads
   `PortableNotificationReader.exe` (and a zip) as a **build artifact**.
2. To cut a versioned release, push a tag like `v1.0.0`; the same binary is
   attached to a GitHub Release automatically.

To build locally on a Windows machine with the Rust toolchain installed:

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --release
# -> target\release\PortableNotificationReader.exe
```

---

## ⚠️ "Windows protected your PC" (SmartScreen)

When you first run the downloaded `.exe`, Windows may show a blue
**"Windows protected your PC"** dialog from *Microsoft Defender SmartScreen*.

**This is expected and the app is safe to run.** SmartScreen flags **any**
program that isn't signed with a paid *code-signing certificate*
(these cost US$200–500 **per year**, which isn't worth it for a free hobby
app). The warning is about the **missing signature, not about anything the app
does** — the full source code is in this repository for anyone to inspect.

### How to run it anyway

1. On the blue dialog, click **More info**.
2. Click the **Run anyway** button that appears.

You only have to do this **once per download** — Windows remembers your choice
for that copy of the file.

### Verify your download first (recommended)

Every release publishes the **SHA-256 hash** of the exe (in the release notes
and as a `PortableNotificationReader.exe.sha256` file). Confirm your copy
matches before running it:

```powershell
Get-FileHash .\PortableNotificationReader.exe -Algorithm SHA256
```

Compare the output to the hash on the [Releases](../../releases) page — if they
match, the file is exactly what the CI built.

### Prefer no warning at all? Two free options

- **Build it yourself** from source (see above). Locally-compiled executables
  are not flagged by SmartScreen.
- **Add a trusted publisher.** Releases can be **self-signed** (see
  [`scripts/self_sign.ps1`](scripts/self_sign.ps1)). A self-signed certificate
  does *not* silence the first warning, but once you install its certificate
  into **Trusted Publishers**, future versions signed with the same certificate
  run without a prompt. Right-click the exe → **Properties → Digital Signatures
  → Details → View Certificate → Install Certificate → Trusted Publishers**.

> Signing is fully optional in CI: if the repository has the `CODESIGN_PFX_BASE64`
> and `CODESIGN_PASSWORD` secrets set, the build signs the exe automatically;
> otherwise it ships unsigned. See [`scripts/self_sign.ps1`](scripts/self_sign.ps1)
> for how to generate a free self-signed certificate and the base64 secret.

---

## Usage

1. Put `PortableNotificationReader.exe` in any folder and run it. A tray icon
   appears; `config.json` is created next to it on first save.
2. Click the tray icon and pick a **Voice**, set **Volume/Speed**, and toggle
   **Read notifications** and **Speak emojis** on/off.
3. Use **Disable apps** to mute specific applications and **Filters** to
   block/allow notifications or filter/replace text before speaking.

> **Tip:** Windows only records a toast in the database when notifications for
> that app are enabled in *Settings → System → Notifications*. If nothing is
> being read, check that the sending app is allowed to show notifications.

### Autostart (optional)

Because the app is portable it doesn’t register itself. To start it with
Windows, drop a shortcut to the `.exe` into your Startup folder
(`Win`+`R` → `shell:startup`).

---

## `config.json`

Created next to the executable. Example:

```json
{
  "enabled": true,
  "selected_voice_id": "online:en-ZA-LeahNeural",
  "rate": 0,
  "volume": 130,
  "show_all_languages": false,
  "speak_emojis": false,
  "muted_apps": ["Microsoft Teams"],
  "known_apps": ["WhatsApp", "Microsoft Teams", "Mail"],
  "filters": [
    { "pattern": "verification code", "is_regex": false, "block": true },
    { "pattern": "^Reminder:", "is_regex": true, "block": true }
  ],
  "replacements": [
    { "pattern": "AI", "replacement": "Artificial Intelligence", "is_regex": false },
    { "pattern": "\\[ERROR\\]", "replacement": "", "is_regex": true }
  ],
  "poll_interval_ms": 1000
}
```

Field notes:

| Field | Meaning |
|-------|---------|
| `selected_voice_id` | `online:{ShortName}` for a neural voice, or `sapi:{name}` for an offline voice. |
| `rate` | Speaking speed, `-10`..`10` (0 = normal). |
| `volume` | `0`..`200`. `100` = normal; above `100` amplifies (louder than system). |
| `show_all_languages` | `false` = only voices matching the Windows display language. |
| `speak_emojis` | `false` (default) strips emojis; `true` speaks their meanings (e.g. 🎉 → "party popper"). |
| `poll_interval_ms` | How often the notification DB is polled (minimum 250 ms; 1000 ms recommended). |
| `filters[].block` | `true` blocks matching notifications; `false` switches to allow-list mode. |
| `replacements[]` | Text filtering & replacement rules. Empty `replacement` removes the match. |

Filter rules are matched against the app name **and** the notification text.

Replacements are applied **after** filters, in order, before speaking. Chat
shorthand (lol, brb, etc.) is automatically expanded and then emojis are handled
per the `speak_emojis` toggle.

---

## How it works

```
                +-----------------------------+
   Windows      |  wpndatabase.db (SQLite)    |
   toast   -->  |  Notification + Handler      |
                +--------------+--------------+
                               | poll every ~1s (read-only)
                               v
   +---------------------------+---------------------------+
   |  worker thread                                        |
   |  parse toast XML -> text parts                        |
   |  build spoken text (skip app name / group name)       |
   |  apply per-app mute + filter rules                    |
   +---------------------------+---------------------------+
                               v
   +---------------------------+---------------------------+
   |  speech engine                                        |
   |  online: Edge neural TTS (MP3) -> rodio (gain boost)  |
   |  offline fallback: Windows SAPI voice                 |
   +-------------------------------------------------------+

   tray UI (native-windows-gui) <-> config.json (portable)
```

The app reads the database **read-only** and only reacts to notifications that
arrive after it starts, so it never re-reads your notification backlog.

---

## Project layout

| Path | Purpose |
|------|---------|
| `src/config.rs` | Portable `config.json` load/save. |
| `src/notifications.rs` | Reads/polls the Windows notification SQLite DB. |
| `src/filter.rs` | Block/allow rules + text filtering/replacement + URL cleaning. |
| `src/text_shaping.rs` | Shorthand expansion + emoji handling (speak or strip). |
| `src/voices.rs` | Full online voice catalogue + language filtering. |
| `src/drm.rs` / `src/edge_tts.rs` | Edge neural TTS auth + WebSocket synthesis. |
| `src/speech.rs` | Audio playback (online) + SAPI offline fallback. |
| `src/locale.rs` | Detects the Windows display language. |
| `src/worker.rs` | Background polling/speaking thread. |
| `src/app.rs` | System-tray UI and menu. |
| `.github/workflows/build.yml` | Windows CI build + release. |

---

## License

See [LICENSE](LICENSE).
