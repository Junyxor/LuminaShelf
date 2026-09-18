use super::AppState;
use lumina_core::{library::now_unix_ms, Bookmark, LibraryItem};
use tauri::State;

#[tauri::command]
pub(super) fn bookmarks(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<Vec<Bookmark>, String> {
    state
        .state_store
        .list_bookmarks(&library_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) fn add_bookmark(
    state: State<'_, AppState>,
    library_id: String,
    label: String,
    locator: String,
    fraction: f64,
) -> Result<Bookmark, String> {
    if locator.len() > 8192 || !fraction.is_finite() {
        return Err("无效的阅读位置".into());
    }
    let bookmark = Bookmark {
        id: uuid::Uuid::new_v4().to_string(),
        library_id,
        label: label.chars().take(160).collect(),
        locator,
        fraction: fraction.clamp(0.0, 1.0),
        created_at_unix_ms: now_unix_ms(),
    };
    state
        .state_store
        .upsert_bookmark(&bookmark)
        .map_err(|error| error.to_string())?;
    Ok(bookmark)
}

#[tauri::command]
pub(super) fn remove_bookmark(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state
        .state_store
        .remove_bookmark(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) fn edit_library_item(
    state: State<'_, AppState>,
    id: String,
    title: String,
    authors: Vec<String>,
) -> Result<LibraryItem, String> {
    if title.trim().is_empty() {
        return Err("书名不能为空".into());
    }
    let mut item = state
        .state_store
        .library_item(&id)
        .map_err(|error| error.to_string())?
        .ok_or("书籍不存在")?;
    item.title = title.trim().chars().take(300).collect();
    item.authors = authors
        .into_iter()
        .map(|author| author.trim().chars().take(150).collect::<String>())
        .filter(|author| !author.is_empty())
        .take(20)
        .collect();
    state
        .state_store
        .upsert_library_item(&item)
        .map_err(|error| error.to_string())?;
    Ok(item)
}

#[tauri::command]
pub(super) fn remove_library_item(
    state: State<'_, AppState>,
    id: String,
    delete_file: bool,
) -> Result<bool, String> {
    let item = state
        .state_store
        .library_item(&id)
        .map_err(|error| error.to_string())?
        .ok_or("书籍不存在")?;
    let mut moved = None;
    if delete_file && item.path.exists() {
        let root =
            std::fs::canonicalize(&state.managed_library_dir).map_err(|error| error.to_string())?;
        let path = std::fs::canonicalize(&item.path).map_err(|error| error.to_string())?;
        if !path.starts_with(&root) {
            return Err("原文件位于应用书库之外，只能移除记录，不会删除原文件。".into());
        }
        let trash = root.join(".trash");
        std::fs::create_dir_all(&trash).map_err(|error| error.to_string())?;
        let target = trash.join(format!(
            "{}-{}",
            uuid::Uuid::new_v4(),
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        std::fs::rename(&path, &target).map_err(|error| error.to_string())?;
        moved = Some((path, target));
    }
    match state.state_store.remove_library_item(&id) {
        Ok(removed) => {
            if let Some((_, target)) = moved {
                std::fs::remove_file(&target).map_err(|error| {
                    format!(
                        "记录已移除，但文件清理失败，副本保留在 {}：{error}",
                        target.display()
                    )
                })?;
            }
            Ok(removed)
        }
        Err(error) => {
            if let Some((path, target)) = moved {
                let _ = std::fs::rename(target, path);
            }
            Err(error.to_string())
        }
    }
}
