//! System-tray user interface (Windows-only), built with `native-windows-gui`.
//!
//! The whole UI is a single tray icon whose pop-up menu is **rebuilt from the
//! current configuration every time it is opened**. This keeps the code simple
//! (no long-lived widget state to keep in sync) and lets us show live data such
//! as the discovered voice catalogue and the list of apps seen so far.
//!
//! Both a left-click and a right-click on the tray icon open the menu, matching
//! the behaviour of the original application.

use crate::config::Config;
use crate::worker::{SayQueue, SharedCatalog, SharedConfig};
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

/// An action attached to a menu item and dispatched when it is clicked.
#[derive(Clone)]
enum Action {
    ToggleEnabled,
    SelectVoice(String),
    ToggleShowAll,
    SetVolume(u32),
    SetRate(i32),
    ToggleMuteApp(String),
    RemoveFilter(usize),
    ClearFilters,
    TestVoice,
    OpenConfig,
    OpenFolder,
    About,
    Exit,
}

pub struct App {
    cfg: SharedConfig,
    catalog: SharedCatalog,
    say_queue: SayQueue,

    window: nwg::MessageWindow,
    #[allow(dead_code)]
    icon: nwg::Icon,
    tray: nwg::TrayNotification,

    // Dynamic menu controls are stored so their native handles stay alive while
    // the menu is on screen. They are replaced wholesale on every rebuild.
    menus: RefCell<Vec<nwg::Menu>>,
    items: RefCell<Vec<nwg::MenuItem>>,
    seps: RefCell<Vec<nwg::MenuSeparator>>,
    actions: RefCell<Vec<(nwg::ControlHandle, Action)>>,
}

impl App {
    /// Build the tray UI and bind the event handler. The returned handler must be
    /// kept alive for as long as the program runs.
    pub fn build(
        cfg: SharedConfig,
        catalog: SharedCatalog,
        say_queue: SayQueue,
    ) -> Result<(Rc<App>, nwg::EventHandler), nwg::NwgError> {
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

        let app = Rc::new(App {
            cfg,
            catalog,
            say_queue,
            window,
            icon,
            tray,
            menus: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
            seps: RefCell::new(Vec::new()),
            actions: RefCell::new(Vec::new()),
        });

        let handler_app = app.clone();
        let handler = nwg::full_bind_event_handler(&app.window.handle, move |evt, _data, handle| {
            use nwg::Event as E;
            match evt {
                // Right-click on the tray icon.
                E::OnContextMenu => {
                    if handle == handler_app.tray.handle {
                        handler_app.show_menu();
                    }
                }
                // Left-click (button released) on the tray icon.
                E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp) => {
                    if handle == handler_app.tray.handle {
                        handler_app.show_menu();
                    }
                }
                E::OnMenuItemSelected => handler_app.on_menu_select(handle),
                _ => {}
            }
        });

        Ok((app, handler))
    }

    /// Rebuild the menu from current state and pop it up at the cursor.
    fn show_menu(&self) {
        self.rebuild_menu();
        let (x, y) = nwg::GlobalCursor::position();
        if let Some(root) = self.menus.borrow().first() {
            root.popup(x, y);
        }
    }

    fn on_menu_select(&self, handle: nwg::ControlHandle) {
        let action = self
            .actions
            .borrow()
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, a)| a.clone());
        if let Some(a) = action {
            self.apply(a);
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
            Action::RemoveFilter(i) => self.with_cfg(|c| {
                if i < c.filters.len() {
                    c.filters.remove(i);
                }
            }),
            Action::ClearFilters => self.with_cfg(|c| c.filters.clear()),
            Action::TestVoice => {
                if let Ok(mut q) = self.say_queue.lock() {
                    q.push("This is a test of the selected notification voice.".to_string());
                }
            }
            Action::OpenConfig => {
                // Make sure the file exists before we open it for editing.
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
                nwg::stop_thread_dispatch();
            }
        }
    }

    // ---- menu construction -------------------------------------------------

    fn rebuild_menu(&self) {
        let mut menus: Vec<nwg::Menu> = Vec::new();
        let mut items: Vec<nwg::MenuItem> = Vec::new();
        let mut seps: Vec<nwg::MenuSeparator> = Vec::new();
        let mut actions: Vec<(nwg::ControlHandle, Action)> = Vec::new();

        // Snapshot the state we need so we don't hold the lock while building.
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
        let all_voices = self
            .catalog
            .lock()
            .map(|c| c.all())
            .unwrap_or_default();

        // ---- root popup menu ----
        let mut root = nwg::Menu::default();
        nwg::Menu::builder()
            .popup(true)
            .parent(&self.window)
            .build(&mut root)
            .unwrap();
        let root_h = root.handle;
        menus.push(root);

        // Title (disabled)
        add_item(&mut items, &mut actions, root_h, "Portable Notification Reader", None, false);
        add_sep(&mut seps, root_h);

        // Enable / disable
        add_item(
            &mut items,
            &mut actions,
            root_h,
            if enabled { "\u{2714} Reading notifications: ON" } else { "\u{2716} Reading notifications: OFF" },
            Some(Action::ToggleEnabled),
            true,
        );
        add_sep(&mut seps, root_h);

        // ---- Voice submenu ----
        let voice_menu = add_submenu(&mut menus, root_h, "Voice");
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
            // Group by the friendly locale label taken from the voice display.
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
            add_item(
                &mut items,
                &mut actions,
                vol_menu,
                &format!("{mark}{p}%{extra}"),
                Some(Action::SetVolume(p)),
                true,
            );
        }

        // ---- Speed submenu ----
        let speed_menu = add_submenu(&mut menus, root_h, &format!("Speed ({rate:+})"));
        for &(name, r) in SPEED_PRESETS {
            let mark = if r == rate { "\u{25CF} " } else { "    " };
            add_item(
                &mut items,
                &mut actions,
                speed_menu,
                &format!("{mark}{name}"),
                Some(Action::SetRate(r)),
                true,
            );
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
                add_item(
                    &mut items,
                    &mut actions,
                    apps_menu,
                    &format!("{mark}{app}"),
                    Some(Action::ToggleMuteApp(app.clone())),
                    true,
                );
            }
        }

        // ---- Filters submenu ----
        let filters_menu = add_submenu(&mut menus, root_h, &format!("Filters ({})", filters.len()));
        if filters.is_empty() {
            add_item(&mut items, &mut actions, filters_menu, "No filter rules", None, false);
        } else {
            add_item(&mut items, &mut actions, filters_menu, "Click a rule to remove it", None, false);
            add_sep(&mut seps, filters_menu);
            for (i, f) in filters.iter().enumerate() {
                let kind = if f.block { "Block" } else { "Allow" };
                let re = if f.is_regex { " [regex]" } else { "" };
                add_item(
                    &mut items,
                    &mut actions,
                    filters_menu,
                    &format!("\u{2716} {kind}: {}{re}", f.pattern),
                    Some(Action::RemoveFilter(i)),
                    true,
                );
            }
            add_sep(&mut seps, filters_menu);
            add_item(&mut items, &mut actions, filters_menu, "Clear all rules", Some(Action::ClearFilters), true);
        }
        add_item(&mut items, &mut actions, filters_menu, "Add / edit rules (opens config.json)", Some(Action::OpenConfig), true);

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
