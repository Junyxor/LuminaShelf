use super::*;
use lumina_core::{provider::DiscoveryPage, BookSummary, FavoriteBook};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn open_external_file(
    app: AppHandle,
    state: State<'_, AppState>,
    library_id: String,
) -> Result<(), String> {
    let item = state
        .state_store
        .list_library_items()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|item| item.id == library_id)
        .ok_or("书架中没有此文件")?;
    app.opener()
        .open_path(item.path.to_string_lossy(), None::<String>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_book_website(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
) -> Result<(), String> {
    let url = if provider_id == "gutendex" {
        format!(
            "https://www.gutenberg.org/ebooks/{}",
            book_id.parse::<u64>().map_err(|_| "书籍标识不正确")?
        )
    } else if provider_id == "zlibrary" {
        let (id, hash) = book_id.split_once(':').ok_or("书籍标识不完整")?;
        if !id.chars().all(|c| c.is_ascii_digit())
            || !hash.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Err("书籍标识不正确".into());
        }
        state
            .zlibrary
            .origin()
            .await
            .join(&format!("/book/{id}/{hash}"))
            .map_err(|e| e.to_string())?
            .to_string()
    } else {
        return Err("书源不支持官网入口".into());
    };
    app.opener()
        .open_url(url, None::<String>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn favorite_books(state: State<'_, AppState>) -> Result<Vec<FavoriteBook>, String> {
    state
        .state_store
        .list_favorites()
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn delete_library_download(
    app: AppHandle,
    state: State<'_, AppState>,
    library_id: String,
) -> Result<(), String> {
    let item = state
        .state_store
        .list_library_items()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|item| item.id == library_id)
        .ok_or("书架中没有此文件")?;
    let path = fs::canonicalize(&item.path).map_err(|_| "文件不存在或已被移动")?;
    let roots = [
        Some(state.book_dir.clone()),
        app.path()
            .download_dir()
            .ok()
            .map(|root| root.join("LuminaShelf")),
    ];
    let owned = roots
        .into_iter()
        .flatten()
        .filter_map(|root| fs::canonicalize(root).ok())
        .any(|root| path.starts_with(root));
    if !owned {
        return Err("此文件不在应用管理目录，请使用文件管理器删除原文件".into());
    }
    let temporary = path.with_file_name(format!(".deleting-{}", uuid::Uuid::new_v4()));
    fs::rename(&path, &temporary).map_err(|e| e.to_string())?;
    if let Err(error) = state.state_store.remove_library_item(&library_id) {
        let _ = fs::rename(&temporary, &path);
        return Err(error.to_string());
    }
    fs::remove_file(temporary).map_err(|e| format!("书架记录已移除，文件清理失败：{e}"))
}

#[tauri::command]
pub fn set_book_favorite(
    state: State<'_, AppState>,
    provider_id: String,
    book: BookSummary,
    favorite: bool,
) -> Result<(), String> {
    if favorite {
        state
            .state_store
            .upsert_favorite(&FavoriteBook {
                provider_id,
                book_id: book.id,
                title: book.title,
                authors: book.authors,
                format: book.format.map(|format| format.extension().to_string()),
                cover_url: book.cover_url,
                added_at_unix_ms: now_unix_ms(),
            })
            .map_err(|error| format!("{error:#}"))
    } else {
        state
            .state_store
            .remove_favorite(&provider_id, &book.id)
            .map(|_| ())
            .map_err(|error| format!("{error:#}"))
    }
}

#[tauri::command]
pub async fn discover_books(
    state: State<'_, AppState>,
    provider_id: String,
    refresh: bool,
) -> Result<DiscoveryPage, String> {
    if provider_id == "zlibrary"
        && !state.zlibrary.has_session().await
        && matches!(
            state.zlibrary_config.read().await.origin_mode,
            ZLibraryOriginMode::Automatic
        )
    {
        if let Ok(origin) = select_automatic_zlibrary_origin(&state).await {
            state
                .zlibrary
                .set_origin(origin)
                .await
                .map_err(|e| format!("{e:#}"))?;
        }
    }
    let session = state.zlibrary.current_session().await;
    let scope = if provider_id == "zlibrary" {
        format!(
            "{}:{}",
            state.zlibrary.origin().await,
            session
                .as_ref()
                .map(|value| value.user_id.as_str())
                .unwrap_or("anonymous")
        )
    } else {
        "public".into()
    };
    let key = format!("discovery:{provider_id}:{scope}");
    let cached: Option<DiscoveryPage> = state
        .state_store
        .setting(&key)
        .map_err(|e| e.to_string())?
        .and_then(|raw| serde_json::from_str(&raw).ok());
    if !refresh {
        if let Some(cached) = cached
            .as_ref()
            .filter(|cached| now_unix_ms() - cached.fetched_at_unix_ms < 300_000)
        {
            return Ok(cached.clone());
        }
    }
    match state.providers.discover(&provider_id).await {
        Ok(page) => {
            state
                .state_store
                .set_setting(
                    &key,
                    &serde_json::to_string(&page).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            Ok(page)
        }
        Err(error) => {
            if let Some(mut cached) = cached {
                cached.stale = true;
                cached.error = Some(format!("{error:#}"));
                Ok(cached)
            } else {
                Err(format!("{error:#}"))
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookOptions {
    variants: Vec<BookSummary>,
    website_url: Option<String>,
    site_features: serde_json::Value,
}

#[tauri::command]
pub async fn book_download_options(
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
) -> Result<BookOptions, String> {
    let details = state
        .providers
        .details(&provider_id, &book_id)
        .await
        .map_err(|e| format!("{e:#}"))?;
    let mut variants = vec![details.summary.clone()];
    let mut website_url = None;
    let mut site_features = serde_json::json!({});
    if provider_id == "zlibrary" {
        if let Ok(other) = state.zlibrary.alternate_formats(&book_id).await {
            for book in other {
                if !variants.iter().any(|item| item.id == book.id) {
                    variants.push(book);
                }
            }
        }
        if let Some((id, hash)) = book_id.split_once(':') {
            website_url = state
                .zlibrary
                .origin()
                .await
                .join(&format!("/book/{id}/{hash}"))
                .ok()
                .map(|url| url.to_string());
            if let Ok(features) = state.zlibrary.book_features(&book_id).await {
                site_features = features;
            }
        }
    } else {
        variants = details
            .summary
            .available_formats
            .iter()
            .map(|format| BookSummary {
                format: Some(*format),
                ..details.summary.clone()
            })
            .collect();
    }
    Ok(BookOptions {
        variants,
        website_url,
        site_features,
    })
}

#[tauri::command]
pub async fn validate_library_file(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<(), String> {
    let item = state
        .state_store
        .list_library_items()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|item| item.id == library_id)
        .ok_or("书架中没有此文件")?;
    tauri::async_runtime::spawn_blocking(move || {
        lumina_core::library::validate_readable_file(&item.path).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn convert_library_book(
    state: State<'_, AppState>,
    library_id: String,
    target: String,
) -> Result<LibraryItem, String> {
    if !matches!(target.as_str(), "epub" | "txt") {
        return Err("本地转换支持 TXT ↔ EPUB".into());
    }
    let item = state
        .state_store
        .list_library_items()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|item| item.id == library_id)
        .ok_or("书架中没有此文件")?;
    let directory = state.book_dir.clone();
    let converted = tauri::async_runtime::spawn_blocking(move || {
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join(format!(
            "{} [转换-{}].{target}",
            sanitize_filename(&item.title),
            uuid::Uuid::new_v4()
        ));
        lumina_core::conversion::convert_text_book(&item.path, &path, &item.title)
            .map_err(|e| format!("{e:#}"))?;
        LibraryItem::inspect(
            path,
            Some(&item.title),
            item.source_provider,
            item.source_book_id,
        )
        .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())??;
    state
        .state_store
        .upsert_library_item(&converted)
        .map_err(|e| e.to_string())?;
    Ok(converted)
}

#[tauri::command]
pub async fn export_library_book(
    app: AppHandle,
    state: State<'_, AppState>,
    library_id: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    use tauri_plugin_fs::FsExt;
    let item = state
        .state_store
        .list_library_items()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|item| item.id == library_id)
        .ok_or("书架中没有此文件")?;
    let (send, receive) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!(
            "{}.{}",
            sanitize_filename(&item.title),
            item.format.as_str()
        ))
        .save_file(move |selected| {
            let _ = send.send(selected);
        });
    let Some(destination) = receive.await.map_err(|_| "导出已取消")? else {
        return Ok(None);
    };
    let label = destination.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        copy_export_file(&item.path, |options| app.fs().open(destination, options))
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(Some(label))
}

fn copy_export_file(
    source: &Path,
    open_destination: impl FnOnce(tauri_plugin_fs::OpenOptions) -> std::io::Result<fs::File>,
) -> Result<(), String> {
    let input = fs::File::open(source).map_err(|e| e.to_string())?;
    let mut options = tauri_plugin_fs::OpenOptions::new();
    // Android's "rw" mode also avoids truncating a content URI before identity checks.
    options.read(true).write(true).create(true);
    let output = open_destination(options).map_err(|e| e.to_string())?;
    let input = same_file::Handle::from_file(input).map_err(|e| e.to_string())?;
    let mut output = same_file::Handle::from_file(output).map_err(|e| e.to_string())?;
    if input == output {
        return Err("导出位置与原书是同一个文件，请选择其他位置".into());
    }
    let output = output.as_file_mut();
    // A new document provider may return a pipe; it has no existing tail to truncate.
    if output.metadata().map_err(|e| e.to_string())?.len() > 0 {
        output.set_len(0).map_err(|e| e.to_string())?;
    }
    std::io::copy(&mut input.as_file(), output).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_rejects_original_and_hard_link_without_modifying_the_book() {
        let directory =
            std::env::temp_dir().join(format!("lumina-export-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("原书.txt");
        let alias = directory.join("同一本书.txt");
        let text = "导出不能破坏原有书籍。";
        fs::write(&source, text).unwrap();
        fs::hard_link(&source, &alias).unwrap();
        for destination in [&source, &alias] {
            let result = copy_export_file(&source, |options| {
                fs::OpenOptions::from(options).open(destination)
            });
            assert!(
                result.is_err(),
                "Exporting onto the original must be rejected"
            );
            assert_eq!(fs::read_to_string(&source).unwrap(), text);
        }
        fs::remove_file(alias).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn export_replaces_destination_with_exact_contents() {
        let directory =
            std::env::temp_dir().join(format!("lumina-export-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("书籍.txt");
        let destination = directory.join("导出.txt");
        fs::write(&source, "书籍内容").unwrap();
        for existing in [None, Some("需要覆盖的更长旧内容，需要完全移除尾部。")]
        {
            if let Some(text) = existing {
                fs::write(&destination, text).unwrap();
            }
            copy_export_file(&source, |options| {
                fs::OpenOptions::from(options).open(&destination)
            })
            .unwrap();
            assert_eq!(fs::read(&destination).unwrap(), fs::read(&source).unwrap());
        }
        fs::remove_file(destination).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
