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
