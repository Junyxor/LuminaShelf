use crate::{
    library::{import_reader, now_unix_ms},
    Bookmark, LibraryItem, ReadingProgress, StateStore,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, Write},
    path::Path,
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

const MAX_BOOK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_BACKUP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_BOOKS: usize = 2000;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupBook {
    id: String,
    title: String,
    authors: Vec<String>,
    entry: String,
    progress: Option<ReadingProgress>,
    bookmarks: Vec<Bookmark>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    created_at_unix_ms: i64,
    books: Vec<BackupBook>,
}

/// Portable ZIP containing ebooks and reading state, never accounts or sessions.
pub fn export_backup(store: &StateStore, output: impl Write + Seek) -> Result<usize> {
    let items = store.list_library_items()?;
    ensure!(items.len() <= MAX_BOOKS, "书库超过单个备份的 2000 本限制");
    let mut archive = ZipWriter::new(output);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut total = 0;
    let mut books = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let mut input = File::open(&item.path)
            .with_context(|| format!("无法备份《{}》，原文件不存在或不可读", item.title))?;
        let bytes = input.metadata()?.len();
        ensure!(bytes <= MAX_BOOK_BYTES, "单本书超过 512 MiB 备份限制");
        total += bytes;
        ensure!(total <= MAX_BACKUP_BYTES, "备份超过 2 GiB 限制");
        let entry = format!("books/{index}.{}", item.format.as_str());
        archive.start_file(&entry, options)?;
        std::io::copy(&mut input, &mut archive)?;
        books.push(BackupBook {
            id: item.id.clone(),
            title: item.title.clone(),
            authors: item.authors.clone(),
            entry,
            progress: store.reading_progress(&item.id)?,
            bookmarks: store.list_bookmarks(&item.id)?,
        });
    }
    archive.start_file(
        "luminashelf.json",
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
    )?;
    archive.write_all(&serde_json::to_vec(&Manifest {
        version: 1,
        created_at_unix_ms: now_unix_ms(),
        books,
    })?)?;
    archive.finish()?;
    Ok(items.len())
}

/// Restore to new owned paths, then commit records/positions/bookmarks together.
/// Pre-existing files and rows are preserved, including on malformed archives.
pub fn restore_backup(
    store: &StateStore,
    input: impl Read + Seek,
    directory: &Path,
) -> Result<usize> {
    let mut archive = ZipArchive::new(input).context("无效的书库备份文件")?;
    ensure!(archive.len() <= MAX_BOOKS + 1, "备份条目过多");
    let mut metadata = archive
        .by_name("luminashelf.json")
        .context("缺少书库备份清单")?;
    ensure!(metadata.size() <= 8 * 1024 * 1024, "备份清单过大");
    let mut json = Vec::new();
    metadata.read_to_end(&mut json)?;
    drop(metadata);
    let manifest: Manifest = serde_json::from_slice(&json).context("无法解析备份清单")?;
    ensure!(manifest.version == 1, "不支持的备份版本");
    ensure!(manifest.books.len() <= MAX_BOOKS, "备份书籍过多");
    let mut items: Vec<LibraryItem> = Vec::new();
    let result = (|| -> Result<usize> {
        let mut positions = Vec::new();
        let mut bookmarks = Vec::new();
        let mut identities = HashMap::new();
        let mut total = 0_u64;
        for (index, book) in manifest.books.into_iter().enumerate() {
            let extension = Path::new(&book.entry)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            ensure!(
                book.entry == format!("books/{index}.{extension}"),
                "备份路径不符合格式"
            );
            ensure!(!identities.contains_key(&book.id), "备份包含重复标识");
            let mut input = archive.by_name(&book.entry).context("备份缺少书籍文件")?;
            ensure!(input.size() <= MAX_BOOK_BYTES, "备份中的书籍过大");
            total = total.checked_add(input.size()).context("备份长度溢出")?;
            ensure!(total <= MAX_BACKUP_BYTES, "备份内容超过 2 GiB");
            let mut item = import_reader(&mut input, directory, &format!("import.{extension}"))?;
            item.title = book.title.chars().take(300).collect();
            item.authors = book
                .authors
                .into_iter()
                .take(20)
                .map(|value| value.chars().take(150).collect())
                .collect();
            identities.insert(book.id, item.id.clone());
            if let Some(mut position) = book.progress {
                position.library_id = item.id.clone();
                if position.fraction.is_finite()
                    && position
                        .locator
                        .as_ref()
                        .is_none_or(|value| value.len() <= 8192)
                {
                    positions.push(position.normalized());
                }
            }
            for mut bookmark in book.bookmarks.into_iter().take(10000) {
                if !bookmark.fraction.is_finite() || bookmark.locator.len() > 8192 {
                    continue;
                }
                bookmark.id = uuid::Uuid::new_v4().to_string();
                bookmark.library_id = item.id.clone();
                bookmark.label = bookmark.label.chars().take(160).collect();
                bookmarks.push(bookmark);
            }
            items.push(item);
        }
        store.restore_library(&items, &positions, &bookmarks)?;
        Ok(items.len())
    })();
    if result.is_err() {
        for item in items {
            let _ = std::fs::remove_file(item.path);
        }
    }
    result
}
