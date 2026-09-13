use lumina_core::{library::scan_folder, LibraryItem, LibraryStore};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

static LIBRARY_STORE: OnceLock<LibraryStore> = OnceLock::new();

fn store(app: &AppHandle) -> Result<&'static LibraryStore, String> {
    if let Some(store) = LIBRARY_STORE.get() {
        return Ok(store);
    }
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let store = LibraryStore::open(config_dir.join("state.sqlite3"))
        .map_err(|error| error.to_string())?;
    let _ = LIBRARY_STORE.set(store);
    LIBRARY_STORE
        .get()
        .ok_or_else(|| "failed to initialize library store".to_string())
}

pub(super) fn persist_library_item(
    app: &AppHandle,
    item: LibraryItem,
) -> Result<LibraryItem, String> {
    store(app)?.upsert(item).map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) fn list_library(app: AppHandle) -> Result<Vec<LibraryItem>, String> {
    store(&app)?
        .list_existing()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) fn scan_library_persisted(
    app: AppHandle,
    path: String,
    recursive: Option<bool>,
    max_items: Option<usize>,
) -> Result<Vec<LibraryItem>, String> {
    let paths = scan_folder(
        path,
        recursive.unwrap_or(true),
        max_items.unwrap_or(500).clamp(1, 5000),
    )
    .map_err(|error| error.to_string())?;
    let store = store(&app)?;
    paths
        .into_iter()
        .map(|path| {
            let item = LibraryItem::inspect(path, None, None, None)
                .map_err(|error| error.to_string())?;
            store.upsert(item).map_err(|error| error.to_string())
        })
        .collect()
}
