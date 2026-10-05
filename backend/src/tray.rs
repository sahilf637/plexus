//! The system tray icon. Plexus keeps running there when its windows are closed, so the global
//! hotkey keeps working.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::AppHandle;

use crate::error::Result;
use crate::windows;

pub fn create(app: &AppHandle) -> Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Plexus", true, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", "Add something…", true, None::<&str>)?;
    let save_clipboard = MenuItem::with_id(app, "save-clipboard", "Save clipboard", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Plexus", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &capture, &save_clipboard, &separator, &quit])?;

    let mut tray = TrayIconBuilder::with_id("plexus")
        .tooltip("Plexus")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => windows::show_main(app),
            "capture" => windows::show_capture(app),
            "save-clipboard" => {
                let app = app.clone();
                std::thread::spawn(move || crate::capture::save_clipboard(&app));
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}
