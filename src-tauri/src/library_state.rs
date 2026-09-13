use lumina_core::{library::scan_folder, LibraryFormat, LibraryItem, LibraryStore};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tauri::{AppHandle, Manager};

#[cfg(target_os = "android")]
use tauri_plugin_android_fs::AndroidFsExt;

static LIBRARY_STORE: OnceLock<LibraryStore> = OnceLock::new();

fn store(app: &AppHandle) -> Result<&'static LibraryStore, String> {
    if let Some(store) = LIBRARY_STORE.get() {
        return Ok(store);
    }
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let store =
        LibraryStore::open(config_dir.join("state.sqlite3")).map_err(|error| error.to_string())?;
    let _ = LIBRARY_STORE.set(store);
    LIBRARY_STORE
        .get()
        .ok_or_else(|| "failed to initialize library store".to_string())
}

pub(super) fn managed_library_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("library");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
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
            let item =
                LibraryItem::inspect(path, None, None, None).map_err(|error| error.to_string())?;
            store.upsert(item).map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(target_os = "android")]
#[tauri::command]
pub(super) async fn import_library_files(app: AppHandle) -> Result<Vec<LibraryItem>, String> {
    let api = app.android_fs_async();
    let selected = api
        .picker()
        .pick_files(None, &["*/*"], false)
        .await
        .map_err(|error| error.to_string())?;
    if selected.is_empty() {
        return store(&app)?
            .list_existing()
            .map_err(|error| error.to_string());
    }

    let destination_dir = managed_library_dir(&app)?;
    let mut imported = 0usize;
    for uri in selected {
        let name = api
            .get_name(&uri)
            .await
            .map_err(|error| error.to_string())?;
        let safe_name = sanitize_import_name(&name);
        if !LibraryFormat::is_supported_path(Path::new(&safe_name)) {
            continue;
        }
        let source = api
            .open_file_readable(&uri)
            .await
            .map_err(|error| error.to_string())?;
        let destination = unique_import_destination(&destination_dir, &safe_name);
        let copy_destination = destination.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let mut source = source;
            let mut target = std::fs::File::create(copy_destination)?;
            std::io::copy(&mut source, &mut target)?;
            target.sync_all()?;
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;

        let item = LibraryItem::inspect(&destination, None, None, None)
            .map_err(|error| error.to_string())?;
        store(&app)?
            .upsert(item)
            .map_err(|error| error.to_string())?;
        imported += 1;
    }

    if imported == 0 {
        return Err("没有选择支持的电子书文件".to_string());
    }
    store(&app)?
        .list_existing()
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub(super) async fn import_library_files(_app: AppHandle) -> Result<Vec<LibraryItem>, String> {
    Err("原生文件导入目前仅用于 Android".to_string())
}

fn sanitize_import_name(name: &str) -> String {
    let file_name = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("book.epub");
    let cleaned = file_name
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>();
    let cleaned = cleaned.trim_matches([' ', '.']);
    if cleaned.is_empty() {
        "book.epub".to_string()
    } else {
        cleaned.chars().take(180).collect()
    }
}

fn unique_import_destination(directory: &Path, file_name: &str) -> PathBuf {
    let requested = Path::new(file_name);
    let stem = requested
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("book");
    let extension = requested
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin");
    let first = directory.join(format!("{stem}.{extension}"));
    if !first.exists() {
        return first;
    }
    for suffix in 2..10_000 {
        let candidate = directory.join(format!("{stem} ({suffix}).{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem}-{}.{}", std::process::id(), extension))
}
