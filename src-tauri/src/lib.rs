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
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&dir)?;
            let store = store::Store::open(&dir.join("doicheck.sqlite3")).expect("open store");
            app.manage(AppState {
                store: Mutex::new(store),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_documents,
            commands::get_email,
            commands::set_email,
            commands::get_reports_dir,
            commands::set_reports_dir,
            commands::open_document,
            commands::latest_check,
            commands::check_document,
            commands::recheck_failures,
            commands::export_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
