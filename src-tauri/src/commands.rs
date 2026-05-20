//! Tauri command handlers bridging the UI to the pipeline and store.

use crate::crossref::CrossrefClient;
use crate::model::Progress;
use crate::store::{DocumentSummary, Store};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, State};

const DEFAULT_EMAIL: &str = "urschrei@gmail.com";

pub struct AppState {
    pub store: Mutex<Store>,
}

fn map_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn list_documents(state: State<'_, AppState>) -> Result<Vec<DocumentSummary>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.list_documents().map_err(map_err)
}

#[tauri::command]
pub fn get_email(state: State<'_, AppState>) -> Result<String, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    Ok(store
        .get_setting("crossref_email")
        .map_err(map_err)?
        .unwrap_or_else(|| DEFAULT_EMAIL.to_string()))
}

#[tauri::command]
pub fn set_email(state: State<'_, AppState>, email: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.set_setting("crossref_email", &email).map_err(map_err)
}

/// Look up an already-seen document by file path; return the latest structured
/// result if present.
#[tauri::command]
pub fn open_document(
    state: State<'_, AppState>,
    path: String,
) -> Result<Option<crate::model::CheckResult>, String> {
    let ingested = crate::ingest::ingest(&PathBuf::from(&path)).map_err(map_err)?;
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.latest_result(&ingested.fingerprint).map_err(map_err)
}

/// The most recent structured result for a document, by fingerprint (sidebar).
#[tauri::command]
pub fn latest_check(
    state: State<'_, AppState>,
    fingerprint: String,
) -> Result<Option<crate::model::CheckResult>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.latest_result(&fingerprint).map_err(map_err)
}

/// Run a full check, persist it, and return the rendered report. Emits
/// `progress` events as `Progress { done, total }`.
#[tauri::command]
pub async fn check_document(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<crate::model::CheckResult, String> {
    let ingested = crate::ingest::ingest(&PathBuf::from(&path)).map_err(map_err)?;
    let text = crate::extract::extract_text(&ingested.bytes, ingested.kind).map_err(map_err)?;
    if !crate::extract::has_usable_text(&text) {
        return Err("no extractable text (image-only PDF?)".to_string());
    }

    let email = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_setting("crossref_email")
            .map_err(map_err)?
            .unwrap_or_else(|| DEFAULT_EMAIL.to_string())
    };
    let client = CrossrefClient::new(&email);
    let run_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let app_for_progress = app.clone();
    let result = crate::pipeline::run(
        ingested.filename.clone(),
        ingested.fingerprint.clone(),
        run_at,
        &text,
        &client,
        move |p: Progress| {
            let _ = app_for_progress.emit("progress", p);
        },
    )
    .await;

    let report_text = crate::report::render(&result);
    let kind = match ingested.kind {
        crate::model::FileKind::Pdf => "pdf",
        crate::model::FileKind::Docx => "docx",
    };
    {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .save_check(&result, kind, &report_text)
            .map_err(map_err)?;
    }
    Ok(result)
}

/// Write a stored report to `path` in the given format ("txt", "json", "csv").
#[tauri::command]
pub fn export_report(
    state: State<'_, AppState>,
    path: String,
    fingerprint: String,
    format: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let content = match format.as_str() {
        "txt" => store
            .latest_report(&fingerprint)
            .map_err(map_err)?
            .ok_or_else(|| "no report stored for this document".to_string())?,
        "json" => {
            let r = store
                .latest_result(&fingerprint)
                .map_err(map_err)?
                .ok_or_else(|| "no result stored for this document".to_string())?;
            crate::export::to_json(&r)
        }
        "csv" => {
            let r = store
                .latest_result(&fingerprint)
                .map_err(map_err)?
                .ok_or_else(|| "no result stored for this document".to_string())?;
            crate::export::to_csv(&r)
        }
        other => return Err(format!("unknown export format: {other}")),
    };
    std::fs::write(&path, content).map_err(map_err)
}

#[tauri::command]
pub fn get_reports_dir(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.get_setting("reports_dir").map_err(map_err)
}

#[tauri::command]
pub fn set_reports_dir(state: State<'_, AppState>, dir: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.set_setting("reports_dir", &dir).map_err(map_err)
}
