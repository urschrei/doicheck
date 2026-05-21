// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

pub mod biblio;
pub mod cache;
pub mod commands;
pub mod compare;
pub mod crossref;
pub mod doi;
pub mod export;
pub mod extract;
pub mod ingest;
pub mod model;
pub mod pipeline;
pub mod report;
pub mod store;
pub mod text;

use commands::AppState;
use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&dir)?;
            let store = store::Store::open(&dir.join("doicheck.sqlite3")).expect("open store");
            app.manage(AppState {
                store: Mutex::new(store),
            });

            // Point PDF extraction at the bundled PDFium library in the app's
            // resource directory.
            if let Ok(resources) = app.path().resource_dir() {
                extract::pdf::set_library_dir(resources.join("pdfium"));
            }

            use tauri::menu::{Menu, MenuItem, Submenu};

            let about = MenuItem::with_id(app, "about", "About DOI Checker", true, None::<&str>)?;

            #[cfg(target_os = "macos")]
            {
                use tauri::menu::PredefinedMenuItem;
                let app_menu = Submenu::with_items(
                    app,
                    "DOI Checker",
                    true,
                    &[
                        &about,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::services(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::hide(app, None)?,
                        &PredefinedMenuItem::hide_others(app, None)?,
                        &PredefinedMenuItem::show_all(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::quit(app, None)?,
                    ],
                )?;
                let edit_menu = Submenu::with_items(
                    app,
                    "Edit",
                    true,
                    &[
                        &PredefinedMenuItem::undo(app, None)?,
                        &PredefinedMenuItem::redo(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::cut(app, None)?,
                        &PredefinedMenuItem::copy(app, None)?,
                        &PredefinedMenuItem::paste(app, None)?,
                        &PredefinedMenuItem::select_all(app, None)?,
                    ],
                )?;
                let window_menu = Submenu::with_items(
                    app,
                    "Window",
                    true,
                    &[
                        &PredefinedMenuItem::minimize(app, None)?,
                        &PredefinedMenuItem::maximize(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::close_window(app, None)?,
                    ],
                )?;
                let menu = Menu::with_items(app, &[&app_menu, &edit_menu, &window_menu])?;
                app.set_menu(menu)?;
            }

            #[cfg(not(target_os = "macos"))]
            {
                let help_menu = Submenu::with_items(app, "Help", true, &[&about])?;
                let menu = Menu::with_items(app, &[&help_menu])?;
                app.set_menu(menu)?;
            }

            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "about" {
                use tauri::Emitter;
                let _ = app.emit("open-about", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_documents,
            commands::delete_document,
            commands::get_email,
            commands::set_email,
            commands::get_reports_dir,
            commands::set_reports_dir,
            commands::open_document,
            commands::latest_check,
            commands::check_document,
            commands::recheck_failures,
            commands::export_report,
            commands::dismiss_discrepancy,
            commands::undismiss_discrepancy,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
