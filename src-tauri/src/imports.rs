use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_fs::{FilePath, FsExt};

fn import_file(app: &AppHandle, source: FilePath, directory: &Path) -> Result<LibraryItem, String> {
    #[cfg(target_os = "android")]
    let name = app
        .path()
        .file_name(&source.to_string())
        .ok_or("无法读取所选文件名")?;
    #[cfg(not(target_os = "android"))]
    let name = source
        .clone()
        .into_path()
        .map_err(|error| error.to_string())?
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or("无法读取所选文件名")?;
    let extension = Path::new(&name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "epub" | "pdf" | "txt") {
        return Err(format!("暂不支持导入 {extension}，请选择 EPUB、PDF 或 TXT"));
    }
    let title = Path::new(&name)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let temporary = directory.join(format!(".import-{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = tauri_plugin_fs::OpenOptions::new();
        options.read(true);
        let mut input = app
            .fs()
            .open(source, options)
            .map_err(|error| error.to_string())?;
        let mut output = fs::File::create(&temporary).map_err(|error| error.to_string())?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut size = 0_u64;
        loop {
            let read = input.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            size += read as u64;
            if size > 512 * 1024 * 1024 {
                return Err("单本书籍不能超过 512 MB".into());
            }
            digest.update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
        }
        drop(output);
        let hash = format!("{:x}", digest.finalize());
        let destination = directory.join(format!(
            "{} [{}].{extension}",
            sanitize_filename(&title),
            &hash[..12]
        ));
        if destination.exists() {
            fs::remove_file(&temporary).map_err(|error| error.to_string())?;
        } else {
            fs::rename(&temporary, &destination).map_err(|error| error.to_string())?;
        }
        LibraryItem::inspect(destination, Some(&title), None, None)
            .map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

async fn import_selection(
    app: AppHandle,
    sources: Vec<FilePath>,
) -> Result<Vec<LibraryItem>, String> {
    if sources.len() > 100 {
        return Err("每次最多导入 100 本书".into());
    }
    let directory = app.state::<AppState>().book_dir.clone();
    let worker_app = app.clone();
    let items = tauri::async_runtime::spawn_blocking(move || {
        sources
            .into_iter()
            .map(|source| import_file(&worker_app, source, &directory))
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|error| error.to_string())??;
    app.state::<AppState>()
        .state_store
        .upsert_library_items(&items)
        .map_err(|error| error.to_string())?;
    Ok(items)
}

#[tauri::command]
pub async fn import_local_books(app: AppHandle) -> Result<Vec<LibraryItem>, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("电子书", &["epub", "pdf", "txt"])
        .pick_files(move |files| {
            let _ = send.send(files.unwrap_or_default());
        });
    let sources = receive.await.map_err(|_| "文件选择已取消")?;
    import_selection(app, sources).await
}

#[tauri::command]
pub async fn import_paths(app: AppHandle, paths: Vec<String>) -> Result<Vec<LibraryItem>, String> {
    import_selection(
        app,
        paths
            .into_iter()
            .map(|path| FilePath::Path(PathBuf::from(path)))
            .collect(),
    )
    .await
}

#[tauri::command]
pub fn remove_library_item(state: State<'_, AppState>, library_id: String) -> Result<(), String> {
    state
        .state_store
        .remove_library_item(&library_id)
        .map_err(|error| error.to_string())
}
