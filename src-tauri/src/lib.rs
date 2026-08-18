mod history;
mod http;
mod store;

use std::path::PathBuf;

use history::{HistoryError, HistoryRecord, HistoryStore};
use http::{HttpError, HttpState, RequestSpec, ResponseData};
use store::{Section, StoreError};
use tauri::{Manager, State};

/// How many history entries the UI gets on startup.
const HISTORY_PAGE: usize = 500;

/// Resolved once at startup so every command agrees on where collections live.
struct Paths {
    sections: PathBuf,
}

/// Sends, and records the outcome. History is written here rather than by the
/// frontend so the body never has to travel back down the IPC bridge, and so an
/// entry exists even if the window dies mid-flight.
#[tauri::command]
async fn send_request(
    state: State<'_, HttpState>,
    log: State<'_, HistoryStore>,
    spec: RequestSpec,
) -> Result<ResponseData, HttpError> {
    let at = history::now_millis();
    let url = spec.url.clone();
    let outcome = http::send(state.inner(), spec.clone()).await;

    // A history failure must not fail the request the user actually made.
    if let Err(err) = log.record(&spec, at, &url, &outcome) {
        ::log::warn!("could not record history: {err}");
    }

    outcome
}

#[tauri::command]
fn history_list(log: State<'_, HistoryStore>) -> Result<Vec<HistoryRecord>, HistoryError> {
    let records = log.list(HISTORY_PAGE)?;
    ::log::info!("loaded {} history entr(ies)", records.len());
    Ok(records)
}

/// Bodies are fetched per entry so listing history stays cheap.
#[tauri::command]
fn history_body(log: State<'_, HistoryStore>, id: String) -> Result<Option<String>, HistoryError> {
    log.body(&id)
}

#[tauri::command]
fn history_delete(log: State<'_, HistoryStore>, id: String) -> Result<(), HistoryError> {
    log.delete(&id)
}

#[tauri::command]
fn history_clear_request(
    log: State<'_, HistoryStore>,
    request_id: String,
) -> Result<(), HistoryError> {
    log.clear_request(&request_id)
}

#[tauri::command]
fn history_clear_all(log: State<'_, HistoryStore>) -> Result<(), HistoryError> {
    log.clear_all()
}

/// Returns whether a request with this id was actually in flight.
#[tauri::command]
fn cancel_request(state: State<'_, HttpState>, id: String) -> bool {
    state.cancel(&id)
}

#[tauri::command]
fn list_sections(paths: State<'_, Paths>) -> Result<Vec<Section>, StoreError> {
    let sections = store::load_all(&paths.sections)?;
    log::info!(
        "loaded {} section(s) from {}",
        sections.len(),
        paths.sections.display()
    );
    Ok(sections)
}

#[tauri::command]
fn save_section(paths: State<'_, Paths>, section: Section) -> Result<(), StoreError> {
    store::save(&paths.sections, &section)
}

#[tauri::command]
fn delete_section(paths: State<'_, Paths>, id: String) -> Result<(), StoreError> {
    store::delete(&paths.sections, &id)
}

/// The UI previews and sends the string this returns, so what you see is
/// exactly what goes out.
#[tauri::command]
fn resolve_url(base: String, path: String) -> String {
    store::join_url(&base, &path)
}

/// Where the collection files are, for the "reveal in Finder" affordance.
#[tauri::command]
fn sections_path(paths: State<'_, Paths>) -> String {
    paths.sections.display().to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(HttpState::default())
        .invoke_handler(tauri::generate_handler![
            send_request,
            cancel_request,
            list_sections,
            save_section,
            delete_section,
            resolve_url,
            sections_path,
            history_list,
            history_body,
            history_delete,
            history_clear_request,
            history_clear_all
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let app_data_dir = app.path().app_data_dir()?;
            app.manage(Paths {
                sections: store::sections_dir(&app_data_dir),
            });
            app.manage(HistoryStore::open(&app_data_dir)?);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
