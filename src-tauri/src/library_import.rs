use super::AppState;
use lumina_core::{library::import_reader, LibraryItem};
use serde::Serialize;
use tauri::{AppHandle, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ImportReport {
    items: Vec<LibraryItem>,
    imported: usize,
    errors: Vec<String>,
}

// The picker grants access to precisely the files chosen by the user. Copies live
// in app-managed storage so a moved original or expired SAF grant cannot break reading.
#[tauri::command]
pub(super) async fn import_library_files(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ImportReport, String> {
    let directory = state.managed_library_dir.clone();
    let store = state.state_store.clone();
    let mut report = ImportReport {
        items: Vec::new(),
        imported: 0,
        errors: Vec::new(),
    };
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_dialog::DialogExt;
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .add_filter(
                "电子书",
                &["epub", "pdf", "txt", "mobi", "azw3", "cbz", "djvu"],
            )
            .pick_files(move |files| {
                let _ = tx.send(files);
            });
        let selected = rx
            .await
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        for selected in selected {
            let result = selected.into_path().map_err(|error| error.to_string());
            let path = match result {
                Ok(path) => path,
                Err(error) => {
                    report.errors.push(error);
                    continue;
                }
            };
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("book")
                .to_string();
            let destination = directory.clone();
            let task_store = store.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let source = std::fs::File::open(&path).map_err(|error| error.to_string())?;
                let item = import_reader(source, &destination, &name)
                    .map_err(|error| error.to_string())?;
                task_store
                    .upsert_library_item(&item)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())?;
            match result {
                Ok(()) => report.imported += 1,
                Err(error) => report.errors.push(error),
            }
        }
    }
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_android_fs::AndroidFsExt;
        let api = app.android_fs_async();
        let selected = api
            .picker()
            .pick_files(None, &["*/*"], false)
            .await
            .map_err(|error| error.to_string())?;
        for uri in selected {
            let result = async {
                let name = api
                    .get_name(&uri)
                    .await
                    .map_err(|error| error.to_string())?;
                let source = api
                    .open_file_readable(&uri)
                    .await
                    .map_err(|error| error.to_string())?;
                let destination = directory.clone();
                let task_store = store.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let item = import_reader(source, &destination, &name)
                        .map_err(|error| error.to_string())?;
                    task_store
                        .upsert_library_item(&item)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| error.to_string())?
            }
            .await;
            match result {
                Ok(()) => report.imported += 1,
                Err(error) => report.errors.push(error),
            }
        }
    }
    report.items = store
        .list_library_items()
        .map_err(|error| error.to_string())?;
    Ok(report)
}

#[tauri::command]
pub(super) async fn choose_library_folder(app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_dialog::DialogExt;
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog().file().pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
        rx.await
            .map_err(|error| error.to_string())?
            .map(|folder| {
                folder
                    .into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .transpose()
    }
    #[cfg(target_os = "android")]
    {
        let _ = app;
        Err("Android 请使用「导入电子书」从系统文件选择器导入。".into())
    }
}
