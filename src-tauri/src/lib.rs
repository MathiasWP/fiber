mod http;
mod store;

use std::path::PathBuf;

use http::{HttpError, HttpState, RequestSpec, ResponseData};
use store::{Section, StoreError};
use tauri::{Manager, State};

/// Resolved once at startup so every command agrees on where collections live.
struct Paths {
    sections: PathBuf,
}

#[tauri::command]
async fn send_request(
    state: State<'_, HttpState>,
    spec: RequestSpec,
) -> Result<ResponseData, HttpError> {
    http::send(state.inner(), spec).await
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
            sections_path
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
