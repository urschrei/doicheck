//! Tauri command handlers bridging the UI to the pipeline and store.

use crate::cache::StoreCache;
use crate::crossref::CrossrefClient;
use crate::datacite::DataCiteClient;
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
    let store = state.store.lock().map_err(map_err)?;
    store.list_documents().map_err(map_err)
}

#[tauri::command]
pub fn get_email(state: State<'_, AppState>) -> Result<String, String> {
    let store = state.store.lock().map_err(map_err)?;
    Ok(store
        .get_setting("crossref_email")
        .map_err(map_err)?
        .unwrap_or_else(|| DEFAULT_EMAIL.to_string()))
}

#[tauri::command]
pub fn set_email(state: State<'_, AppState>, email: String) -> Result<(), String> {
    let store = state.store.lock().map_err(map_err)?;
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
    let store = state.store.lock().map_err(map_err)?;
    store.latest_result(&ingested.fingerprint).map_err(map_err)
}

/// The most recent structured result for a document, by fingerprint (sidebar).
#[tauri::command]
pub fn latest_check(
    state: State<'_, AppState>,
    fingerprint: String,
) -> Result<Option<crate::model::CheckResult>, String> {
    let store = state.store.lock().map_err(map_err)?;
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
    run_full_check(&app, &state.store, &path).await
}

/// Re-check an already-known document by fingerprint, re-reading its stored file
/// path. Errors if no path was recorded or the file no longer exists, so the
/// caller can fall back to asking the user to locate it.
#[tauri::command]
pub async fn recheck_document(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    fingerprint: String,
) -> Result<crate::model::CheckResult, String> {
    let path = {
        let store = state.store.lock().map_err(map_err)?;
        store.path_for(&fingerprint).map_err(map_err)?
    };
    let path = path.ok_or_else(|| "no stored file path for this document".to_string())?;
    if !std::path::Path::new(&path).exists() {
        return Err(format!("file no longer exists: {path}"));
    }
    run_full_check(&app, &state.store, &path).await
}

/// Full check of a document at `path`: extract, run the pipeline, persist (and
/// record the path for later re-checks), and return the annotated result.
async fn run_full_check(
    app: &tauri::AppHandle,
    store: &Mutex<Store>,
    path: &str,
) -> Result<crate::model::CheckResult, String> {
    let ingested = crate::ingest::ingest(&PathBuf::from(path)).map_err(map_err)?;
    let text = crate::extract::extract_text(&ingested.bytes, ingested.kind).map_err(map_err)?;
    if !crate::extract::has_usable_text(&text) {
        return Err("no extractable text (image-only PDF?)".to_string());
    }

    let (email, concurrency) = {
        let store = store.lock().map_err(map_err)?;
        let email = store
            .get_setting("crossref_email")
            .map_err(map_err)?
            .unwrap_or_else(|| DEFAULT_EMAIL.to_string());
        let concurrency = store.concurrency();
        (email, concurrency)
    };
    let client = CrossrefClient::new(&email);
    let datacite = DataCiteClient::new(&email);
    let run_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let cache = StoreCache { store };
    let app_for_progress = app.clone();
    let result = crate::pipeline::run(
        ingested.filename.clone(),
        ingested.fingerprint.clone(),
        run_at,
        &text,
        &client,
        &datacite,
        &cache,
        concurrency,
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
        let mut store = store.lock().map_err(map_err)?;
        store
            .save_check(&result, kind, &report_text)
            .map_err(map_err)?;
        store
            .set_document_path(&ingested.fingerprint, path)
            .map_err(map_err)?;
    }
    {
        let store = store.lock().map_err(map_err)?;
        if let Some(annotated) = store
            .latest_result(&ingested.fingerprint)
            .map_err(map_err)?
        {
            return Ok(annotated);
        }
    }
    Ok(result)
}

/// Re-resolve only the entries that previously failed transiently, merge the
/// result, persist, and return it. Works from stored state (no file needed).
#[tauri::command]
pub async fn recheck_failures(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    fingerprint: String,
) -> Result<crate::model::CheckResult, String> {
    let (result, kind, email, concurrency) = {
        let store = state.store.lock().map_err(map_err)?;
        let result = store
            .latest_result(&fingerprint)
            .map_err(map_err)?
            .ok_or_else(|| "no prior check for this document".to_string())?;
        let kind = store
            .kind_for(&fingerprint)
            .map_err(map_err)?
            .unwrap_or_else(|| "pdf".to_string());
        let email = store
            .get_setting("crossref_email")
            .map_err(map_err)?
            .unwrap_or_else(|| DEFAULT_EMAIL.to_string());
        let concurrency = store.concurrency();
        (result, kind, email, concurrency)
    };

    let client = CrossrefClient::new(&email);
    let datacite = DataCiteClient::new(&email);
    let app_for_progress = app.clone();
    let updated = crate::pipeline::recheck_failures(
        result,
        &client,
        &datacite,
        &crate::cache::StoreCache {
            store: &state.store,
        },
        concurrency,
        move |p: crate::model::Progress| {
            let _ = app_for_progress.emit("progress", p);
        },
    )
    .await;

    let report_text = crate::report::render(&updated);
    {
        let mut store = state.store.lock().map_err(map_err)?;
        store
            .save_check(&updated, &kind, &report_text)
            .map_err(map_err)?;
    }
    Ok(updated)
}

enum ExportFormat {
    Txt,
    Json,
    Csv,
}

impl std::str::FromStr for ExportFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "txt" => Ok(Self::Txt),
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            other => Err(format!("unknown export format: {other}")),
        }
    }
}

/// Write a stored report to `path` in the given format ("txt", "json", "csv").
#[tauri::command]
pub fn export_report(
    state: State<'_, AppState>,
    path: String,
    fingerprint: String,
    format: String,
) -> Result<(), String> {
    let format: ExportFormat = format.parse()?;
    let store = state.store.lock().map_err(map_err)?;
    let r = store
        .latest_result(&fingerprint)
        .map_err(map_err)?
        .ok_or_else(|| "no result stored for this document".to_string())?;
    let content = match format {
        ExportFormat::Txt => crate::report::render(&r),
        ExportFormat::Json => crate::export::to_json(&r).map_err(map_err)?,
        ExportFormat::Csv => crate::export::to_csv(&r),
    };
    std::fs::write(&path, content).map_err(map_err)
}

#[tauri::command]
pub fn dismiss_discrepancy(
    state: State<'_, AppState>,
    fingerprint: String,
    doi: String,
    field: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(map_err)?;
    store
        .add_dismissal(&fingerprint, &doi, &field)
        .map_err(map_err)
}

#[tauri::command]
pub fn undismiss_discrepancy(
    state: State<'_, AppState>,
    fingerprint: String,
    doi: String,
    field: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(map_err)?;
    store
        .remove_dismissal(&fingerprint, &doi, &field)
        .map_err(map_err)
}

/// Remove a document and its checks from the database. The shared DOI cache is
/// left intact.
#[tauri::command]
pub fn delete_document(state: State<'_, AppState>, fingerprint: String) -> Result<(), String> {
    let mut store = state.store.lock().map_err(map_err)?;
    store.delete_document(&fingerprint).map_err(map_err)
}

#[tauri::command]
pub fn get_reports_dir(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let store = state.store.lock().map_err(map_err)?;
    store.get_setting("reports_dir").map_err(map_err)
}

#[tauri::command]
pub fn set_reports_dir(state: State<'_, AppState>, dir: String) -> Result<(), String> {
    let store = state.store.lock().map_err(map_err)?;
    store.set_setting("reports_dir", &dir).map_err(map_err)
}

#[tauri::command]
pub fn get_concurrency(state: State<'_, AppState>) -> Result<u32, String> {
    let store = state.store.lock().map_err(map_err)?;
    Ok(store.concurrency() as u32)
}

#[tauri::command]
pub fn set_concurrency(state: State<'_, AppState>, value: u32) -> Result<(), String> {
    let clamped = value.clamp(1, 20) as usize;
    let store = state.store.lock().map_err(map_err)?;
    store.set_concurrency(clamped).map_err(map_err)
}
