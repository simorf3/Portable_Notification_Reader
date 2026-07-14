//! System-tray user interface (Windows-only), built with `native-windows-gui`.
//!
//! * The tray icon's pop-up menu is **rebuilt from the current configuration**
//!   each time it opens, so it always reflects live state (voices discovered,
//!   apps seen, current volume/speed, …).
//! * Hovering a voice in the menu **plays a short preview** in that voice.
//! * Filters are edited in a **dedicated GUI window** (no hand-editing JSON),
//!   with an explicit blacklist / whitelist choice.
//! * Both a left-click and a right-click on the tray icon open the menu.

use crate::config::{Config, FilterRule, ReplaceRule};
use crate::worker::{PreviewSlot, SayQueue, SharedCatalog, SharedConfig};
use crate::{locale, voices};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

// Extended window style for the transient startup hint window:
//   WS_EX_TOOLWINDOW (0x0080) – keep it out of the taskbar / Alt-Tab list.
//
// The hint is a *normal titled* window (WindowFlags::WINDOW) rather than a
// borderless WS_POPUP. A plain popup has several ways to end up invisible
// (no repaint, wrong z-order, never activated); a titled top-most window is
// guaranteed to render and sit where we place it. It is also marked top-most
// via the builder so it floats above other windows.
const ARROW_EX_FLAGS: u32 = 0x0080;

/// How long the startup "running in the tray" arrow hint stays on screen.
const ARROW_HINT_SECS: u64 = 6;

/// Icon shown in the tray (compiled into the executable → nothing to ship).
const ICON_BYTES: &[u8] = include_bytes!("../assets/app.ico");

/// Volume presets offered in the menu (percent). Values >100 amplify.
const VOLUME_PRESETS: &[u32] = &[50, 80, 100, 130, 150, 180, 200];

/// Speaking-speed presets on our -10..=10 scale, with friendly labels.
const SPEED_PRESETS: &[(&str, i32)] = &[
    ("Very slow", -8),
    ("Slow", -4),
    ("Normal", 0),
    ("Fast", 4),
    ("Very fast", 8),
];

/// Phrase spoken when previewing / testing a voice.
const SAMPLE_TEXT: &str = "Hello, this is a preview of the selected notification voice.";

/// An action attached to a menu item and dispatched when it is clicked.
#[derive(Clone)]
enum Action {
    ToggleEnabled,
    ToggleSpeakEmojis,
    TogglePauseOnMic,
    SelectVoice(String),
    ToggleShowAll,
    SetVolume(u32),
    SetRate(i32),
    ToggleMuteApp(String),
    TestVoice,
    ManageFilters,
    ManageReplacements,
    OpenFolder,
    About,
    Exit,
}

pub struct App {
    cfg: SharedConfig,
    catalog: SharedCatalog,
    say_queue: SayQueue,
    preview: PreviewSlot,

    window: nwg::MessageWindow,
    #[allow(dead_code)]
    icon: nwg::Icon,
    tray: nwg::TrayNotification,

    // ---- Filters window (built once, shown on demand) ----
    // Some controls are only read by the window itself after creation; they are
    // retained as fields so nwg keeps them alive for the window's lifetime.
    fwindow: nwg::Window,
    #[allow(dead_code)]
    fhint: nwg::Label,
    fmode: nwg::Label,
    flist: nwg::ListBox<String>,
    #[allow(dead_code)]
    fpattern_label: nwg::Label,
    fpattern: nwg::TextInput,
    fregex: nwg::CheckBox,
    fblock: nwg::RadioButton,
    fallow: nwg::RadioButton,
    fadd: nwg::Button,
    fremove: nwg::Button,
    fclose: nwg::Button,

    // ---- "Text filtering & replacement rules" window ----
    // A single rule with a blank replacement acts as a plain filter (removes
    // the match); with a replacement it substitutes text before speaking.
    rwindow: nwg::Window,
    #[allow(dead_code)]
    rhint: nwg::Label,
    rlist: nwg::ListBox<String>,
    #[allow(dead_code)]
    rfind_label: nwg::Label,
    rfind: nwg::TextInput,
    #[allow(dead_code)]
    rwith_label: nwg::Label,
    rwith: nwg::TextInput,
    rregex: nwg::CheckBox,
    radd: nwg::Button,
    rremove: nwg::Button,
    rclose: nwg::Button,

    // ---- Startup "running in the tray" arrow hint ----
    // A borderless, click-through, top-most window shown once at launch. It sits
    // just above the notification area and points at it with a big arrow, then
    // closes itself after `ARROW_HINT_SECS` via `arrow_timer`.
    awindow: nwg::Window,
    #[allow(dead_code)]
    atext: nwg::Label,
    #[allow(dead_code)]
    aarrow: nwg::Label,
    arrow_timer: nwg::AnimationTimer,

    // Dynamic menu controls are stored so their native handles stay alive while
    // the menu is on screen. They are replaced wholesale on every rebuild.
    menus: RefCell<Vec<nwg::Menu>>,
    items: RefCell<Vec<nwg::MenuItem>>,
    seps: RefCell<Vec<nwg::MenuSeparator>>,
    actions: RefCell<Vec<(nwg::ControlHandle, Action)>>,
    /// Last voice id we previewed, to avoid replaying while the cursor sits still.
    last_preview: RefCell<String>,
}

impl App {
    /// Build the tray UI + filters window and bind the event handlers. The
    /// returned handlers must be kept alive for as long as the program runs.
    pub fn build(
        cfg: SharedConfig,
        catalog: SharedCatalog,
        say_queue: SayQueue,
        preview: PreviewSlot,
    ) -> Result<(Rc<App>, Vec<nwg::EventHandler>), nwg::NwgError> {
        // Use the modern Windows UI font for every control. Without this, nwg
        // falls back to the ancient "System" bitmap font, which renders far too
        // small and looks cramped/ugly. Setting a global default makes every
        // control built afterwards inherit Segoe UI at a sensible size.
        let mut ui_font = nwg::Font::default();
        nwg::Font::builder()
            .family("Segoe UI")
            .size(18)
            .build(&mut ui_font)?;
        nwg::Font::set_global_default(Some(ui_font));

        // A bold copy for the "mode" line so the whitelist/blacklist state stands out.
        let mut bold_font = nwg::Font::default();
        nwg::Font::builder()
            .family("Segoe UI")
            .size(18)
            .weight(700)
            .build(&mut bold_font)?;

        let mut window = nwg::MessageWindow::default();
        nwg::MessageWindow::builder().build(&mut window)?;

        let mut icon = nwg::Icon::default();
        nwg::Icon::builder()
            .source_bin(Some(ICON_BYTES))
            .build(&mut icon)?;

        let mut tray = nwg::TrayNotification::default();
        nwg::TrayNotification::builder()
            .parent(&window)
            .icon(Some(&icon))
            .tip(Some("Portable Notification Reader \u{2013} running (click for menu)"))
            .build(&mut tray)?;

        // ---- Filters window ----
        let mut fwindow = nwg::Window::default();
        nwg::Window::builder()
            .title("Notification Filters")
            .flags(nwg::WindowFlags::WINDOW) // title + close, non-resizable, hidden
            .size((490, 470))
            .center(true)
            .build(&mut fwindow)?;

        let mut fhint = nwg::Label::default();
        nwg::Label::builder()
            .text(
                "Add rules to control which notifications are read aloud.\n\
                 \u{2022} BLOCK (blacklist): read everything EXCEPT matches.\n\
                 \u{2022} ALLOW (whitelist): read ONLY matches (block rules still apply).",
            )
            .parent(&fwindow)
            .position((12, 10))
            .size((456, 62))
            .build(&mut fhint)?;

        let mut fmode = nwg::Label::default();
        nwg::Label::builder()
            .text("Mode: no rules \u{2013} every notification is read.")
            .parent(&fwindow)
            .font(Some(&bold_font))
            .position((12, 76))
            .size((456, 22))
            .build(&mut fmode)?;

        let mut flist = nwg::ListBox::default();
        nwg::ListBox::builder()
            .parent(&fwindow)
            .position((12, 100))
            .size((456, 150))
            .build(&mut flist)?;

        let mut fpattern_label = nwg::Label::default();
        nwg::Label::builder()
            .text("Text / pattern:")
            .parent(&fwindow)
            .position((12, 262))
            .size((90, 22))
            .build(&mut fpattern_label)?;

        let mut fpattern = nwg::TextInput::default();
        nwg::TextInput::builder()
            .parent(&fwindow)
            .position((104, 260))
            .size((364, 24))
            .build(&mut fpattern)?;

        let mut fregex = nwg::CheckBox::default();
        nwg::CheckBox::builder()
            .text("Treat as a regular expression")
            .parent(&fwindow)
            .position((12, 292))
            .size((300, 22))
            .build(&mut fregex)?;

        let mut fblock = nwg::RadioButton::default();
        nwg::RadioButton::builder()
            .text("Block (blacklist) \u{2013} silence matching notifications")
            .parent(&fwindow)
            .position((12, 316))
            .size((440, 22))
            .build(&mut fblock)?;
        fblock.set_check_state(nwg::RadioButtonState::Checked);

        let mut fallow = nwg::RadioButton::default();
        nwg::RadioButton::builder()
            .text("Allow (whitelist) \u{2013} read ONLY matching notifications")
            .parent(&fwindow)
            .position((12, 340))
            .size((440, 22))
            .build(&mut fallow)?;

        let mut fadd = nwg::Button::default();
        nwg::Button::builder()
            .text("Add rule")
            .parent(&fwindow)
            .position((12, 372))
            .size((110, 30))
            .build(&mut fadd)?;

        let mut fremove = nwg::Button::default();
        nwg::Button::builder()
            .text("Remove selected")
            .parent(&fwindow)
            .position((132, 372))
            .size((150, 30))
            .build(&mut fremove)?;

        let mut fclose = nwg::Button::default();
        nwg::Button::builder()
            .text("Close")
            .parent(&fwindow)
            .position((378, 372))
            .size((90, 30))
            .build(&mut fclose)?;

        // ---- "Text filtering & replacement rules" window ----
        let mut rwindow = nwg::Window::default();
        nwg::Window::builder()
            .title("Text filtering and replacement rules")
            .flags(nwg::WindowFlags::WINDOW)
            .size((520, 430))
            .center(true)
            .build(&mut rwindow)?;

        let mut rhint = nwg::Label::default();
        nwg::Label::builder()
            .text(
                "Define rules to process messages before they are spoken. You can either \
                 replace specific text or remove (filter) it entirely by leaving the \
                 replacement empty.",
            )
            .parent(&rwindow)
            .position((12, 10))
            .size((496, 48))
            .build(&mut rhint)?;

        let mut rlist = nwg::ListBox::default();
        nwg::ListBox::builder()
            .parent(&rwindow)
            .position((12, 66))
            .size((496, 170))
            .build(&mut rlist)?;

        let mut rfind_label = nwg::Label::default();
        nwg::Label::builder()
            .text("Text / pattern:")
            .parent(&rwindow)
            .position((12, 250))
            .size((110, 22))
            .build(&mut rfind_label)?;

        let mut rfind = nwg::TextInput::default();
        nwg::TextInput::builder()
            .parent(&rwindow)
            .position((124, 248))
            .size((384, 24))
            .build(&mut rfind)?;

        let mut rwith_label = nwg::Label::default();
        nwg::Label::builder()
            .text("Replace with:")
            .parent(&rwindow)
            .position((12, 282))
            .size((110, 22))
            .build(&mut rwith_label)?;

        let mut rwith = nwg::TextInput::default();
        nwg::TextInput::builder()
            .parent(&rwindow)
            .position((124, 280))
            .size((384, 24))
            .build(&mut rwith)?;

        let mut rregex = nwg::CheckBox::default();
        nwg::CheckBox::builder()
            .text("Treat \u{2018}Text / pattern\u{2019} as a regular expression")
            .parent(&rwindow)
            .position((12, 312))
            .size((400, 22))
            .build(&mut rregex)?;

        let mut radd = nwg::Button::default();
        nwg::Button::builder()
            .text("Add rule")
            .parent(&rwindow)
            .position((12, 344))
            .size((110, 30))
            .build(&mut radd)?;

        let mut rremove = nwg::Button::default();
        nwg::Button::builder()
            .text("Remove selected")
            .parent(&rwindow)
            .position((132, 344))
            .size((150, 30))
            .build(&mut rremove)?;

        let mut rclose = nwg::Button::default();
        nwg::Button::builder()
            .text("Close")
            .parent(&rwindow)
            .position((418, 344))
            .size((90, 30))
            .build(&mut rclose)?;

        // ---- Startup arrow hint (borderless, click-through, top-most) ----
        // Sit just above the notification area in the bottom-right corner of the
        // primary monitor and point at it with a big arrow.
        let aw: i32 = 340;
        let ah: i32 = 120;
        let screen_w = nwg::Monitor::width();
        let screen_h = nwg::Monitor::height();
        // Sit higher up and further left of the far-right corner so the arrow
        // points at the general tray-icon area (which lives to the LEFT of the
        // clock/date) rather than at the clock itself. Clamp so we never go
        // off-screen on small displays.
        let ax = (screen_w - aw - 220).max(0);
        let ay = (screen_h - ah - 150).max(0);

        let mut awindow = nwg::Window::default();
        nwg::Window::builder()
            .title("Portable Notification Reader")
            .flags(nwg::WindowFlags::WINDOW) // caption + close, non-resizable
            .ex_flags(ARROW_EX_FLAGS)
            .topmost(true)
            .icon(Some(&icon))
            .size((aw, ah))
            .position((ax, ay))
            .build(&mut awindow)?;

        let mut atext = nwg::Label::default();
        nwg::Label::builder()
            .text(
                "Portable Notification Reader is running.\n\
                 It lives down here in the notification tray \u{2013} \
                 click the icon any time for the menu.",
            )
            .parent(&awindow)
            .position((14, 12))
            .size((312, 66))
            .build(&mut atext)?;

        // Big arrow pointing down-right toward the tray / overflow area.
        let mut arrow_font = nwg::Font::default();
        nwg::Font::builder()
            .family("Segoe UI Symbol")
            .size(46)
            .weight(700)
            .build(&mut arrow_font)?;

        let mut aarrow = nwg::Label::default();
        nwg::Label::builder()
            .text("\u{2198}") // ↘ down-right arrow
            .font(Some(&arrow_font))
            .parent(&awindow)
            .position((250, 60))
            .size((76, 56))
            .build(&mut aarrow)?;

        // Fires once after ARROW_HINT_SECS to close the hint automatically.
        // Parented to the message window so its tick is delivered by the main
        // event handler (`h1`).
        let mut arrow_timer = nwg::AnimationTimer::default();
        nwg::AnimationTimer::builder()
            .parent(&window)
            .interval(Duration::from_secs(ARROW_HINT_SECS))
            .max_tick(Some(1))
            .active(false)
            .build(&mut arrow_timer)?;

        let app = Rc::new(App {
            cfg,
            catalog,
            say_queue,
            preview,
            window,
            icon,
            tray,
            fwindow,
            fhint,
            fmode,
            flist,
            fpattern_label,
            fpattern,
            fregex,
            fblock,
            fallow,
            fadd,
            fremove,
            fclose,
            rwindow,
            rhint,
            rlist,
            rfind_label,
            rfind,
            rwith_label,
            rwith,
            rregex,
            radd,
            rremove,
            rclose,
            awindow,
            atext,
            aarrow,
            arrow_timer,
            menus: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
            seps: RefCell::new(Vec::new()),
            actions: RefCell::new(Vec::new()),
            last_preview: RefCell::new(String::new()),
        });

        // One handler per top-level window (tray/message + the two editors).
        let a1 = app.clone();
        let h1 = nwg::full_bind_event_handler(&app.window.handle, move |evt, data, handle| {
            a1.dispatch(evt, &data, handle);
        });
        let a2 = app.clone();
        let h2 = nwg::full_bind_event_handler(&app.fwindow.handle, move |evt, data, handle| {
            a2.dispatch(evt, &data, handle);
        });
        let a3 = app.clone();
        let h3 = nwg::full_bind_event_handler(&app.rwindow.handle, move |evt, data, handle| {
            a3.dispatch(evt, &data, handle);
        });
        let a4 = app.clone();
        let h4 = nwg::full_bind_event_handler(&app.awindow.handle, move |evt, data, handle| {
            a4.dispatch(evt, &data, handle);
        });

        // Announce that the app is running with a tray balloon that fades on its
        // own. The balloon visually points at our tray icon, so the user knows
        // where to find the app afterwards.
        app.tray.show(
            "Running in the notification tray. Left- or right-click the icon here for the menu.",
            Some("Portable Notification Reader"),
            Some(nwg::TrayNotificationFlags::USER_ICON | nwg::TrayNotificationFlags::LARGE_ICON),
            Some(&app.icon),
        );

        // Pop the arrow hint above the tray and start the countdown that closes
        // it after a few seconds.
        app.awindow.set_visible(true);
        app.arrow_timer.start();

        Ok((app, vec![h1, h2, h3, h4]))
    }

    fn dispatch(&self, evt: nwg::Event, data: &nwg::EventData, handle: nwg::ControlHandle) {
        use nwg::Event as E;
        match evt {
            E::OnContextMenu if handle == self.tray.handle => self.show_menu(),
            E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp)
                if handle == self.tray.handle =>
            {
                self.show_menu()
            }
            E::OnMenuItemSelected => self.on_menu_select(handle),
            E::OnMenuHover => self.on_menu_hover(handle),
            E::OnButtonClick => self.on_button(handle),
            // The one-shot timer elapsed: close the startup arrow hint.
            E::OnTimerTick | E::OnTimerStop if handle == self.arrow_timer.handle => {
                self.arrow_timer.stop();
                self.awindow.set_visible(false);
            }
            E::OnWindowClose => {
                // Hide instead of destroying, so the window can be shown again
                // and closing it never quits the whole app.
                if handle == self.fwindow.handle
                    || handle == self.rwindow.handle
                    || handle == self.awindow.handle
                {
                    if let nwg::EventData::OnWindowClose(close) = data {
                        close.close(false);
                    }
                    if handle == self.fwindow.handle {
                        self.fwindow.set_visible(false);
                    } else if handle == self.rwindow.handle {
                        self.rwindow.set_visible(false);
                    } else {
                        // User dismissed the startup hint early — stop its timer.
                        self.arrow_timer.stop();
                        self.awindow.set_visible(false);
                    }
                }
            }
            _ => {}
        }
    }

    /// Rebuild the menu from current state and pop it up at the cursor.
    fn show_menu(&self) {
        self.rebuild_menu();
        let (x, y) = nwg::GlobalCursor::position();
        if let Some(root) = self.menus.borrow().first() {
            root.popup(x, y);
        }
    }

    fn find_action(&self, handle: nwg::ControlHandle) -> Option<Action> {
        self.actions
            .borrow()
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, a)| a.clone())
    }

    fn on_menu_select(&self, handle: nwg::ControlHandle) {
        if let Some(a) = self.find_action(handle) {
            self.apply(a);
        }
    }

    /// Hovering a voice item previews it (once per distinct voice).
    fn on_menu_hover(&self, handle: nwg::ControlHandle) {
        if let Some(Action::SelectVoice(id)) = self.find_action(handle) {
            if *self.last_preview.borrow() != id {
                *self.last_preview.borrow_mut() = id.clone();
                if let Ok(mut slot) = self.preview.lock() {
                    *slot = Some(id); // latest-wins; worker plays it promptly
                }
            }
        }
    }

    fn on_button(&self, handle: nwg::ControlHandle) {
        // ---- Filter messages window ----
        if handle == self.fadd.handle {
            self.add_filter();
        } else if handle == self.fremove.handle {
            self.remove_filter();
        } else if handle == self.fclose.handle {
            self.fwindow.set_visible(false);
        } else if handle == self.fblock.handle {
            self.fallow.set_check_state(nwg::RadioButtonState::Unchecked);
            self.fblock.set_check_state(nwg::RadioButtonState::Checked);
        } else if handle == self.fallow.handle {
            self.fblock.set_check_state(nwg::RadioButtonState::Unchecked);
            self.fallow.set_check_state(nwg::RadioButtonState::Checked);
        // ---- Text filtering & replacement rules window ----
        } else if handle == self.radd.handle {
            self.add_replacement();
        } else if handle == self.rremove.handle {
            self.remove_replacement();
        } else if handle == self.rclose.handle {
            self.rwindow.set_visible(false);
        }
    }

    /// Lock the config, mutate it and persist the result.
    fn with_cfg(&self, f: impl FnOnce(&mut Config)) {
        if let Ok(mut c) = self.cfg.lock() {
            f(&mut c);
            let _ = c.save();
        }
    }

    fn apply(&self, action: Action) {
        match action {
            Action::ToggleEnabled => self.with_cfg(|c| c.enabled = !c.enabled),
            Action::ToggleSpeakEmojis => self.with_cfg(|c| c.speak_emojis = !c.speak_emojis),
            Action::TogglePauseOnMic => self.with_cfg(|c| c.pause_on_mic = !c.pause_on_mic),
            Action::SelectVoice(id) => self.with_cfg(|c| c.selected_voice_id = id),
            Action::ToggleShowAll => self.with_cfg(|c| c.show_all_languages = !c.show_all_languages),
            Action::SetVolume(v) => self.with_cfg(|c| c.volume = v),
            Action::SetRate(r) => self.with_cfg(|c| c.rate = r),
            Action::ToggleMuteApp(app) => self.with_cfg(|c| {
                if let Some(pos) = c
                    .muted_apps
                    .iter()
                    .position(|m| m.eq_ignore_ascii_case(&app))
                {
                    c.muted_apps.remove(pos);
                } else {
                    c.muted_apps.push(app.clone());
                }
            }),
            Action::TestVoice => {
                if let Ok(mut q) = self.say_queue.lock() {
                    q.push(SAMPLE_TEXT.to_string());
                }
            }
            Action::ManageFilters => self.show_filters(),
            Action::ManageReplacements => self.show_replacements(),
            Action::OpenFolder => {
                let _ = std::process::Command::new("explorer.exe")
                    .arg(Config::app_dir())
                    .spawn();
            }
            Action::About => {
                nwg::modal_info_message(
                    &self.window.handle,
                    "About Portable Notification Reader",
                    "Portable Notification Reader\n\n\
                     Reads your Windows notifications aloud using online neural voices \
                     (with an offline fallback).\n\n\
                     Left- or right-click the tray icon for the menu.",
                );
            }
            Action::Exit => {
                // Remove the tray icon and terminate immediately so the process
                // fully releases its folder/exe (no lingering background threads).
                self.tray.set_visibility(false);
                nwg::stop_thread_dispatch();
                std::process::exit(0);
            }
        }
    }

    // ---- filters window --------------------------------------------------

    fn show_filters(&self) {
        self.refresh_filters();
        self.fwindow.set_visible(true);
        self.fpattern.set_focus();
    }

    fn refresh_filters(&self) {
        let filters = self
            .cfg
            .lock()
            .map(|c| c.filters.clone())
            .unwrap_or_default();

        let rows: Vec<String> = filters
            .iter()
            .map(|f| {
                let kind = if f.block { "\u{1F507} BLOCK" } else { "\u{2705} ALLOW" };
                let re = if f.is_regex { "  [regex]" } else { "" };
                format!("{kind}   {}{re}", f.pattern)
            })
            .collect();
        self.flist.set_collection(rows);

        let any_allow = filters.iter().any(|f| !f.block);
        let any_block = filters.iter().any(|f| f.block);
        let mode = if any_allow {
            "Mode: WHITELIST \u{2013} only notifications matching an ALLOW rule are read (BLOCK rules still silence)."
        } else if any_block {
            "Mode: BLACKLIST \u{2013} everything is read except BLOCK matches."
        } else {
            "Mode: no rules \u{2013} every notification is read."
        };
        self.fmode.set_text(mode);
    }

    fn add_filter(&self) {
        let pattern = self.fpattern.text();
        let pattern = pattern.trim().to_string();
        if pattern.is_empty() {
            nwg::modal_info_message(
                &self.fwindow.handle,
                "Add rule",
                "Please type some text (or a regular expression) to match.",
            );
            return;
        }
        let is_regex = self.fregex.check_state() == nwg::CheckBoxState::Checked;
        let block = self.fblock.check_state() == nwg::RadioButtonState::Checked;

        self.with_cfg(|c| {
            c.filters.push(FilterRule {
                pattern: pattern.clone(),
                is_regex,
                block,
            });
        });
        self.fpattern.set_text("");
        self.refresh_filters();
    }

    fn remove_filter(&self) {
        if let Some(idx) = self.flist.selection() {
            self.with_cfg(|c| {
                if idx < c.filters.len() {
                    c.filters.remove(idx);
                }
            });
            self.refresh_filters();
        } else {
            nwg::modal_info_message(
                &self.fwindow.handle,
                "Remove rule",
                "Select a rule in the list first, then click Remove selected.",
            );
        }
    }

    // ---- Text filtering & replacement rules window -----------------------

    fn show_replacements(&self) {
        self.refresh_replacements();
        self.rwindow.set_visible(true);
        self.rfind.set_focus();
    }

    fn refresh_replacements(&self) {
        let rules = self
            .cfg
            .lock()
            .map(|c| c.replacements.clone())
            .unwrap_or_default();
        let rows: Vec<String> = rules
            .iter()
            .map(|r| {
                let re = if r.is_regex { " (RegEx)" } else { "" };
                if r.replacement.is_empty() {
                    // Blank replacement = a plain filter (removes the match).
                    format!("Filter{re}: \u{201C}{}\u{201D}", r.pattern)
                } else {
                    format!(
                        "Replace{re}: \u{201C}{}\u{201D} with \u{201C}{}\u{201D}",
                        r.pattern, r.replacement
                    )
                }
            })
            .collect();
        self.rlist.set_collection(rows);
    }

    fn add_replacement(&self) {
        let pattern = self.rfind.text().trim().to_string();
        if pattern.is_empty() {
            nwg::modal_info_message(
                &self.rwindow.handle,
                "Text filtering and replacement rules",
                "Please type the text (or a regular expression) to match.",
            );
            return;
        }
        // Replacement may be empty (that just deletes the match); keep it as typed.
        let replacement = self.rwith.text();
        let is_regex = self.rregex.check_state() == nwg::CheckBoxState::Checked;
        self.with_cfg(|c| {
            c.replacements.push(ReplaceRule {
                pattern: pattern.clone(),
                replacement: replacement.clone(),
                is_regex,
            });
        });
        self.rfind.set_text("");
        self.rwith.set_text("");
        self.refresh_replacements();
    }

    fn remove_replacement(&self) {
        if let Some(idx) = self.rlist.selection() {
            self.with_cfg(|c| {
                if idx < c.replacements.len() {
                    c.replacements.remove(idx);
                }
            });
            self.refresh_replacements();
        } else {
            nwg::modal_info_message(
                &self.rwindow.handle,
                "Text filtering and replacement rules",
                "Select a rule in the list first, then click Remove selected.",
            );
        }
    }

    // ---- menu construction -----------------------------------------------

    fn rebuild_menu(&self) {
        let mut menus: Vec<nwg::Menu> = Vec::new();
        let mut items: Vec<nwg::MenuItem> = Vec::new();
        let mut seps: Vec<nwg::MenuSeparator> = Vec::new();
        let mut actions: Vec<(nwg::ControlHandle, Action)> = Vec::new();

        let (enabled, speak_emojis, pause_on_mic, volume, rate, show_all, selected, known_apps, muted_apps, filters, n_replacements) = {
            let c = self.cfg.lock().unwrap();
            (
                c.enabled,
                c.speak_emojis,
                c.pause_on_mic,
                c.volume,
                c.rate,
                c.show_all_languages,
                c.selected_voice_id.clone(),
                c.known_apps.clone(),
                c.muted_apps.clone(),
                c.filters.clone(),
                c.replacements.len(),
            )
        };
        let all_voices = self.catalog.lock().map(|c| c.all()).unwrap_or_default();

        // ---- root popup menu ----
        let mut root = nwg::Menu::default();
        nwg::Menu::builder()
            .popup(true)
            .parent(&self.window)
            .build(&mut root)
            .unwrap();
        let root_h = root.handle;
        menus.push(root);

        add_item(&mut items, &mut actions, root_h, "Portable Notification Reader", None, false);
        add_sep(&mut seps, root_h);

        add_item(
            &mut items,
            &mut actions,
            root_h,
            if enabled { "\u{2714} Read notifications" } else { "\u{2610} Read notifications" },
            Some(Action::ToggleEnabled),
            true,
        );
        add_item(
            &mut items,
            &mut actions,
            root_h,
            if speak_emojis { "\u{2714} Speak emojis" } else { "\u{2610} Speak emojis" },
            Some(Action::ToggleSpeakEmojis),
            true,
        );
        add_item(
            &mut items,
            &mut actions,
            root_h,
            if pause_on_mic {
                "\u{2714} Pause during calls/meetings (mic or camera)"
            } else {
                "\u{2610} Pause during calls/meetings (mic or camera)"
            },
            Some(Action::TogglePauseOnMic),
            true,
        );
        add_sep(&mut seps, root_h);

        // ---- Voice submenu (hover an entry to hear it) ----
        let voice_menu = add_submenu(&mut menus, root_h, "Voice  (hover to preview)");
        add_item(
            &mut items,
            &mut actions,
            voice_menu,
            if show_all { "\u{2714} Show all languages" } else { "\u{2610} Show all languages" },
            Some(Action::ToggleShowAll),
            true,
        );
        add_item(&mut items, &mut actions, voice_menu, "\u{25B6} Test current voice", Some(Action::TestVoice), true);
        add_sep(&mut seps, voice_menu);

        let (ui_locale, ui_lang) = locale::windows_ui_locale();
        let list = voices::filter_and_sort(all_voices, &ui_lang, &ui_locale, show_all);
        if list.is_empty() {
            add_item(&mut items, &mut actions, voice_menu, "Loading voices\u{2026}", None, false);
        } else {
            let mut groups: Vec<(String, Vec<voices::Voice>)> = Vec::new();
            for v in list {
                let label = v
                    .display
                    .split("\u{2014} ")
                    .nth(1)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| if v.online { v.locale.clone() } else { "Offline (SAPI)".to_string() });
                match groups.iter_mut().find(|(l, _)| *l == label) {
                    Some(g) => g.1.push(v),
                    None => groups.push((label, vec![v])),
                }
            }
            for (label, vs) in groups {
                let gm = add_submenu(&mut menus, voice_menu, &label);
                for v in vs {
                    let short = v
                        .display
                        .split("\u{2014}")
                        .next()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| v.display.clone());
                    let mark = if v.id == selected { "\u{25CF} " } else { "    " };
                    add_item(
                        &mut items,
                        &mut actions,
                        gm,
                        &format!("{mark}{short}"),
                        Some(Action::SelectVoice(v.id.clone())),
                        true,
                    );
                }
            }
        }

        // ---- Volume submenu ----
        let vol_menu = add_submenu(&mut menus, root_h, &format!("Volume ({volume}%)"));
        for &p in VOLUME_PRESETS {
            let mark = if p == volume { "\u{25CF} " } else { "    " };
            let extra = if p > 100 { " (boosted)" } else if p == 100 { " (normal)" } else { "" };
            add_item(&mut items, &mut actions, vol_menu, &format!("{mark}{p}%{extra}"), Some(Action::SetVolume(p)), true);
        }

        // ---- Speed submenu ----
        let speed_menu = add_submenu(&mut menus, root_h, &format!("Speed ({rate:+})"));
        for &(name, r) in SPEED_PRESETS {
            let mark = if r == rate { "\u{25CF} " } else { "    " };
            add_item(&mut items, &mut actions, speed_menu, &format!("{mark}{name}"), Some(Action::SetRate(r)), true);
        }

        // ---- Apps submenu (per-app mute) ----
        let apps_menu = add_submenu(&mut menus, root_h, "Filter apps");
        if known_apps.is_empty() {
            add_item(&mut items, &mut actions, apps_menu, "No apps seen yet", None, false);
        } else {
            add_item(&mut items, &mut actions, apps_menu, "Click an app to mute / unmute it", None, false);
            add_sep(&mut seps, apps_menu);
            for app in &known_apps {
                let muted = muted_apps.iter().any(|m| m.eq_ignore_ascii_case(app));
                let mark = if muted { "\u{1F507} " } else { "\u{1F508} " };
                add_item(&mut items, &mut actions, apps_menu, &format!("{mark}{app}"), Some(Action::ToggleMuteApp(app.clone())), true);
            }
        }

        // ---- Filter editors (top-level, each opens its own editor window) ----
        add_item(
            &mut items,
            &mut actions,
            root_h,
            &format!("Filter messages\u{2026} ({} rule{})", filters.len(), plural(filters.len())),
            Some(Action::ManageFilters),
            true,
        );
        add_item(
            &mut items,
            &mut actions,
            root_h,
            &format!("Filter and replace text\u{2026} ({} rule{})", n_replacements, plural(n_replacements)),
            Some(Action::ManageReplacements),
            true,
        );

        // ---- Footer ----
        add_sep(&mut seps, root_h);
        add_item(&mut items, &mut actions, root_h, "Open app folder", Some(Action::OpenFolder), true);
        add_item(&mut items, &mut actions, root_h, "About", Some(Action::About), true);
        add_sep(&mut seps, root_h);
        add_item(&mut items, &mut actions, root_h, "Exit", Some(Action::Exit), true);

        *self.menus.borrow_mut() = menus;
        *self.items.borrow_mut() = items;
        *self.seps.borrow_mut() = seps;
        *self.actions.borrow_mut() = actions;
    }
}

// ---- small builder helpers -------------------------------------------------

/// "" for a count of 1, "s" otherwise (for "1 rule" / "3 rules").
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn add_item(
    items: &mut Vec<nwg::MenuItem>,
    actions: &mut Vec<(nwg::ControlHandle, Action)>,
    parent: nwg::ControlHandle,
    text: &str,
    action: Option<Action>,
    enabled: bool,
) -> nwg::ControlHandle {
    let mut it = nwg::MenuItem::default();
    nwg::MenuItem::builder()
        .text(text)
        .parent(parent)
        .build(&mut it)
        .unwrap();
    if !enabled {
        it.set_enabled(false);
    }
    let h = it.handle;
    if let Some(a) = action {
        actions.push((h, a));
    }
    items.push(it);
    h
}

fn add_sep(seps: &mut Vec<nwg::MenuSeparator>, parent: nwg::ControlHandle) {
    let mut s = nwg::MenuSeparator::default();
    nwg::MenuSeparator::builder()
        .parent(parent)
        .build(&mut s)
        .unwrap();
    seps.push(s);
}

fn add_submenu(
    menus: &mut Vec<nwg::Menu>,
    parent: nwg::ControlHandle,
    text: &str,
) -> nwg::ControlHandle {
    let mut m = nwg::Menu::default();
    nwg::Menu::builder()
        .text(text)
        .parent(parent)
        .build(&mut m)
        .unwrap();
    let h = m.handle;
    menus.push(m);
    h
}
