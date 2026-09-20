use super::AppState;
use lumina_core::LibraryItem;
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub(super) async fn share_library_book(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    open_with: Option<bool>,
) -> Result<(), String> {
    let item = state
        .state_store
        .library_item(&id)
        .map_err(|error| error.to_string())?
        .ok_or("书籍不存在")?;
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("shared-books");
    tokio::fs::create_dir_all(&cache)
        .await
        .map_err(|error| error.to_string())?;
    // Each share gets its own directory so sharing two books with one title is safe.
    let directory = cache.join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| error.to_string())?;
    let path = directory.join(format!(
        "{}.{}",
        super::sanitize_filename(&item.title),
        item.format.as_str()
    ));
    tokio::fs::copy(&item.path, &path)
        .await
        .map_err(|error| error.to_string())?;
    let mime = match item.format.as_str() {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    };
    super::native_downloads::share(&app, &path, mime, open_with.unwrap_or(false)).await
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RestoreResult {
    imported: usize,
    items: Vec<LibraryItem>,
}

#[cfg(target_os = "android")]
async fn output_file(app: &AppHandle, name: &str, mime: &str) -> Result<Option<File>, String> {
    use tauri_plugin_android_fs::AndroidFsExt;
    let api = app.android_fs_async();
    let Some(uri) = api
        .picker()
        .save_file(None, name, Some(mime), false)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    api.open_file_writable(&uri)
        .await
        .map(Some)
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "android"))]
async fn output_file(app: &AppHandle, name: &str, _mime: &str) -> Result<Option<File>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(name)
        .save_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(file) = rx.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    File::create(file.into_path().map_err(|error| error.to_string())?)
        .map(Some)
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "android")]
async fn input_file(app: &AppHandle) -> Result<Option<File>, String> {
    use tauri_plugin_android_fs::AndroidFsExt;
    let api = app.android_fs_async();
    let Some(uri) = api
        .picker()
        .pick_file(
            None,
            &["application/zip", "application/octet-stream"],
            false,
        )
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    api.open_file_readable(&uri)
        .await
        .map(Some)
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "android"))]
async fn input_file(app: &AppHandle) -> Result<Option<File>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("书库备份", &["zip"])
        .pick_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(file) = rx.await.map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    File::open(file.into_path().map_err(|error| error.to_string())?)
        .map(Some)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn export_library_book(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let item = state
        .state_store
        .library_item(&id)
        .map_err(|error| error.to_string())?
        .ok_or("书籍不存在")?;
    let name = format!(
        "{}.{}",
        super::sanitize_filename(&item.title),
        item.format.as_str()
    );
    let mime = match item.format.as_str() {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    };
    let Some(mut target) = output_file(&app, &name, mime).await? else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
        let mut source = File::open(item.path).map_err(|error| error.to_string())?;
        std::io::copy(&mut source, &mut target).map_err(|error| error.to_string())?;
        target.flush().map_err(|error| error.to_string())?;
        Ok(true)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn export_library_backup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<usize>, String> {
    let store = state.state_store.clone();
    let cache = state
        .managed_library_dir
        .parent()
        .ok_or("无法获取应用目录")?
        .join("backup-cache");
    std::fs::create_dir_all(&cache).map_err(|error| error.to_string())?;
    let temporary = Temporary(cache.join(format!("{}.zip", uuid::Uuid::new_v4())));
    let work_path = temporary.0.clone();
    let count = tauri::async_runtime::spawn_blocking(move || -> Result<usize, String> {
        lumina_core::backup::export_backup(
            &store,
            File::create(work_path).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    let Some(mut target) = output_file(&app, "LuminaShelf-backup.zip", "application/zip").await?
    else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<usize>, String> {
        let mut source = File::open(&temporary.0).map_err(|error| error.to_string())?;
        std::io::copy(&mut source, &mut target).map_err(|error| error.to_string())?;
        target.flush().map_err(|error| error.to_string())?;
        Ok(Some(count))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(super) async fn restore_library_backup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<RestoreResult>, String> {
    let Some(source) = input_file(&app).await? else {
        return Ok(None);
    };
    let store = state.state_store.clone();
    let directory = state.managed_library_dir.clone();
    let cache = directory
        .parent()
        .ok_or("无法获取应用目录")?
        .join("backup-cache");
    std::fs::create_dir_all(&cache).map_err(|error| error.to_string())?;
    let temporary = Temporary(cache.join(format!("{}.zip", uuid::Uuid::new_v4())));
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<RestoreResult>, String> {
        let mut output = File::create(&temporary.0).map_err(|error| error.to_string())?;
        let limit = 2_u64 * 1024 * 1024 * 1024 + 16 * 1024 * 1024;
        let copied = std::io::copy(&mut source.take(limit + 1), &mut output)
            .map_err(|error| error.to_string())?;
        if copied > limit {
            return Err("备份文件超过大小限制".into());
        }
        drop(output);
        let imported = lumina_core::backup::restore_backup(
            &store,
            File::open(&temporary.0).map_err(|error| error.to_string())?,
            &directory,
        )
        .map_err(|error| error.to_string())?;
        Ok(Some(RestoreResult {
            imported,
            items: store
                .list_library_items()
                .map_err(|error| error.to_string())?,
        }))
    })
    .await
    .map_err(|error| error.to_string())?
}
