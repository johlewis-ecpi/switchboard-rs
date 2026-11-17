use muda::{
    Menu, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
pub struct NativeMenu {
    pub menu: Menu,
    pub focused_items: Vec<MenuItem>,
}
pub fn build() -> NativeMenu {
    let menu = Menu::new();
    let app = Submenu::new("Switchboard", true);
    let file = Submenu::new("File", true);
    let edit = Submenu::new("Edit", true);
    let view = Submenu::new("View", true);
    let mut focused_items = vec![];
    let item = |id: &str, title: &str, key: Option<Code>, shift: bool| {
        MenuItem::with_id(
            id,
            title,
            true,
            key.map(|k| {
                Accelerator::new(
                    Some(
                        Modifiers::SUPER
                            | if shift {
                                Modifiers::SHIFT
                            } else {
                                Modifiers::empty()
                            },
                    ),
                    k,
                )
            }),
        )
    };
    app.append_items(&[
        &item("ui:settings", "Settings…", Some(Code::Comma), false),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(None),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &item("quit", "Quit Switchboard", Some(Code::KeyQ), false),
    ])
    .unwrap();
    file.append(&item(
        "newWindow",
        "New Library Window",
        Some(Code::KeyN),
        false,
    ))
    .unwrap();
    file.append(&item("launcher", "Quick Launcher", None, false))
        .unwrap();
    for (id, title, key) in [
        ("addFiles", "Add Files and Folders…", Some(Code::KeyO)),
        ("ui:addUrl", "Add URL…", None),
        ("ui:health", "Library Health…", None),
        ("ui:transfer", "Import, Export and Backups…", None),
        ("ui:collections", "Manage Collections…", None),
        ("ui:tags", "Manage Tags…", None),
    ] {
        let i = item(id, title, key, false);
        file.append(&i).unwrap();
        focused_items.push(i)
    }
    // Native text controls keep their responder-chain undo and edit commands.
    edit.append_items(&[
        &item("ui:undoRoute", "Undo", Some(Code::KeyZ), false),
        &item("ui:redoRoute", "Redo", Some(Code::KeyZ), true),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
        &PredefinedMenuItem::separator(),
    ])
    .unwrap();
    for (id, title, key, shift) in [
        ("undo", "Undo Library Action", None, false),
        ("redo", "Redo Library Action", None, false),
        ("ui:focusSearch", "Search Library", Some(Code::KeyF), false),
        ("ui:clearSearch", "Clear Search", None, false),
    ] {
        let i = item(id, title, key, shift);
        edit.append(&i).unwrap();
        focused_items.push(i)
    }
    for (id, title) in [
        ("ui:sidebar", "Toggle Sidebar"),
        ("ui:inspector", "Toggle Details"),
        ("ui:minimal", "Toggle Minimal"),
        ("ui:config", "View Configuration…"),
        ("ui:recent", "Recently Opened"),
        ("ui:frequent", "Frequently Opened"),
        ("ui:showAll", "All Entries"),
        ("ui:smart", "Smart Collections…"),
        ("ui:rules", "Organization Rules…"),
        ("ui:bulk", "Bulk Organization…"),
    ] {
        let i = item(id, title, None, false);
        view.append(&i).unwrap();
        focused_items.push(i)
    }
    menu.append_items(&[&app, &file, &edit, &view]).unwrap();
    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();
    NativeMenu {
        menu,
        focused_items,
    }
}

/// Per-window menu IDs keep selections tied to the window that opened the menu.
pub struct EntryMenu {
    menu: Menu,
    actions: std::collections::HashMap<String, String>,
}

fn entry_actions(
    kind: switchboard::model::Kind,
    favorite: bool,
    reorder: bool,
) -> Vec<(&'static str, &'static str)> {
    use switchboard::model::Kind;
    let mut actions = vec![("open", "Open")];
    if kind == Kind::File {
        actions.push(("viewDocument", "View in App"));
    }
    if kind != Kind::Url {
        actions.push(("reveal", "Show in Finder"));
    }
    actions.extend([
        ("copy", "Copy Location"),
        (
            "favorite",
            if favorite {
                "Remove Favorite"
            } else {
                "Favorite"
            },
        ),
        ("organize", "Move / tags…"),
        ("clearCollection", "Remove from Collection"),
    ]);
    if reorder {
        actions.extend([("earlier", "Move Earlier"), ("later", "Move Later")]);
    }
    actions.extend([
        ("show", "Show in Switchboard"),
        ("remove", "Remove from Library…"),
    ]);
    actions
}

impl EntryMenu {
    pub fn new(entry: &switchboard::model::Entry, reorder: bool) -> Result<Self, String> {
        let menu = Menu::new();
        let token = uuid::Uuid::new_v4();
        let mut actions = std::collections::HashMap::new();
        for (action, title) in entry_actions(entry.kind, entry.favorite, reorder) {
            let id = format!("context:{token}:{action}");
            menu.append(&MenuItem::with_id(&id, title, true, None))
                .map_err(|e| e.to_string())?;
            actions.insert(id, action.to_owned());
        }
        Ok(Self { menu, actions })
    }

    pub fn action(&self, id: &str) -> Option<&str> {
        self.actions.get(id).map(String::as_str)
    }

    pub fn show(&self, window: &tao::window::Window, x: f64, y: f64) {
        use muda::{ContextMenu, dpi::LogicalPosition};
        let position = Some(LogicalPosition::new(x, y).into());
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSWindow;
            use tao::platform::macos::WindowExtMacOS;
            let native = unsafe { &*(window.ns_window() as *const NSWindow) };
            // Wry replaces tao's original view; anchor to the installed content view.
            if let Some(view) = native.contentView() {
                unsafe {
                    self.menu.show_context_menu_for_nsview(
                        (&*view as *const objc2_app_kit::NSView).cast(),
                        position,
                    );
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::WindowExtWindows;
            unsafe {
                self.menu
                    .show_context_menu_for_hwnd(window.hwnd(), position);
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use tao::platform::unix::WindowExtUnix;
            self.menu
                .show_context_menu_for_gtk_window(window.gtk_window(), position);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::entry_actions;
    use switchboard::model::Kind;

    #[test]
    fn minimal_context_menu_keeps_all_actions() {
        let actions = entry_actions(Kind::File, false, true);
        assert_eq!(actions.len(), 11);
        assert!(actions.contains(&("viewDocument", "View in App")));
        assert!(actions.contains(&("earlier", "Move Earlier")));
        assert!(actions.contains(&("later", "Move Later")));
        assert_eq!(actions.last(), Some(&("remove", "Remove from Library…")));
    }

    #[test]
    fn context_menu_matches_kind_favorite_and_ordering() {
        for kind in [Kind::File, Kind::Folder, Kind::Url] {
            let actions = entry_actions(kind, true, false);
            assert_eq!(
                actions.iter().any(|(action, _)| *action == "viewDocument"),
                kind == Kind::File
            );
            assert_eq!(
                actions.iter().any(|(action, _)| *action == "reveal"),
                kind != Kind::Url
            );
            assert!(actions.contains(&("favorite", "Remove Favorite")));
            assert!(
                !actions
                    .iter()
                    .any(|(action, _)| matches!(*action, "earlier" | "later"))
            );
        }
    }
}
