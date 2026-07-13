//! System-tray user interface (Windows-only), built with `native-windows-gui`.
//!
//! * The tray icon's pop-up menu is **rebuilt from the current configuration**
//!   each time it opens, so it always reflects live state (voices discovered,
//!   apps seen, current volume/speed, …).
//! * Hovering a voice in the menu **plays a short preview** in that voice.
//! * Filters are edited in a **dedicated GUI window** (no hand-editing JSON),
//!   with an explicit blacklist / whitelist choice.
//! * Both a left-click and a right-click on the tray icon open the menu.

use crate::config::{Config, FilterRule};
use crate::worker::{PreviewSlot, SayQueue, SharedCatalog, SharedConfig};
use crate::{locale, voices};
use std::cell::RefCell;
use std::rc::Rc;

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
    SelectVoice(String),
    ToggleShowAll,
    SetVolume(u32),
    SetRate(i32),
    ToggleMuteApp(String),
    TestVoice,
    ManageFilters,
    OpenConfig,
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
            .tip(Some("Portable Notification Reader"))
            .build(&mut tray)?;

        // ---- Filters window ----
        let mut fwindow = nwg::Window::default();
        nwg::Window::builder()
            .title("Notification Filters")
            .flags(nwg::WindowFlags::WINDOW) // title + close, non-resizable, hidden
            .size((480, 430))
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
            .position((12, 76))
            .size((456, 20))
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
            menus: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
            seps: RefCell::new(Vec::new()),
            actions: RefCell::new(Vec::new()),
            last_preview: RefCell::new(String::new()),
        });

        // One handler for the tray/message window, one for the filters window.
        let a1 = app.clone();
        let h1 = nwg::full_bind_event_handler(&app.window.handle, move |evt, data, handle| {
            a1.dispatch(evt, &data, handle);
        });
        let a2 = app.clone();
        let h2 = nwg::full_bind_event_handler(&app.fwindow.handle, move |evt, data, handle| {
            a2.dispatch(evt, &data, handle);
        });

        Ok((app, vec![h1, h2]))
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
            E::OnWindowClose if handle == self.fwindow.handle => {
                // Hide instead of destroying, so the window can be reopened.
                if let nwg::EventData::OnWindowClose(close) = data {
                    close.close(false);
                }
                self.fwindow.set_visible(false);
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
            Action::OpenConfig => {
                if let Ok(c) = self.cfg.lock() {
                    let _ = c.save();
                }
                let _ = std::process::Command::new("notepad.exe")
                    .arg(Config::config_path())
                    .spawn();
            }
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
                     (with an offline fallback). Fully portable: all settings live in \
                     config.json next to the executable.\n\n\
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

    // ---- menu construction -----------------------------------------------

    fn rebuild_menu(&self) {
        let mut menus: Vec<nwg::Menu> = Vec::new();
        let mut items: Vec<nwg::MenuItem> = Vec::new();
        let mut seps: Vec<nwg::MenuSeparator> = Vec::new();
        let mut actions: Vec<(nwg::ControlHandle, Action)> = Vec::new();

        let (enabled, volume, rate, show_all, selected, known_apps, muted_apps, filters) = {
            let c = self.cfg.lock().unwrap();
            (
                c.enabled,
                c.volume,
                c.rate,
                c.show_all_languages,
                c.selected_voice_id.clone(),
                c.known_apps.clone(),
                c.muted_apps.clone(),
                c.filters.clone(),
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
            if enabled { "\u{2714} Reading notifications: ON" } else { "\u{2716} Reading notifications: OFF" },
            Some(Action::ToggleEnabled),
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
        let apps_menu = add_submenu(&mut menus, root_h, "Apps");
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

        // ---- Filters (opens the GUI window) ----
        add_item(
            &mut items,
            &mut actions,
            root_h,
            &format!("Filters\u{2026} ({} rule{})", filters.len(), if filters.len() == 1 { "" } else { "s" }),
            Some(Action::ManageFilters),
            true,
        );

        // ---- Footer ----
        add_sep(&mut seps, root_h);
        add_item(&mut items, &mut actions, root_h, "Open config file", Some(Action::OpenConfig), true);
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
