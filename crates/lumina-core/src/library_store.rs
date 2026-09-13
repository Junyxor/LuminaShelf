use crate::library::{LibraryFormat, LibraryItem};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone)]
pub struct LibraryStore {
    connection: Arc<Mutex<Connection>>,
}

impl LibraryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path).context("open LuminaShelf library state")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self> {
        let store = Self {
            connection: Arc::new(Mutex::new(Connection::open_in_memory()?)),
        };
        store.migrate()?;
        Ok(store)
    }

    fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("SQLite library lock was poisoned"))
    }

    fn migrate(&self) -> Result<()> {
        self.conn()?.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS library_items (
              path TEXT PRIMARY KEY,
              id TEXT NOT NULL UNIQUE,
              title TEXT NOT NULL,
              authors_json TEXT NOT NULL,
              format TEXT NOT NULL,
              size_bytes INTEGER NOT NULL,
              added_at_unix_ms INTEGER NOT NULL,
              modified_at_unix_ms INTEGER,
              source_provider TEXT,
              source_book_id TEXT
            );
            CREATE INDEX IF NOT EXISTS library_items_added_idx
              ON library_items(added_at_unix_ms DESC);
            CREATE INDEX IF NOT EXISTS library_items_source_idx
              ON library_items(source_provider, source_book_id);
            "#,
        )?;
        Ok(())
    }

    pub fn upsert(&self, item: LibraryItem) -> Result<LibraryItem> {
        let authors_json = serde_json::to_string(&item.authors)?;
        let path = item.path.to_string_lossy().into_owned();
        let size_bytes = item.size_bytes.min(i64::MAX as u64) as i64;
        self.conn()?.execute(
            r#"INSERT INTO library_items(
                 path, id, title, authors_json, format, size_bytes,
                 added_at_unix_ms, modified_at_unix_ms, source_provider, source_book_id
               ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
               ON CONFLICT(path) DO UPDATE SET
                 title=excluded.title,
                 authors_json=excluded.authors_json,
                 format=excluded.format,
                 size_bytes=excluded.size_bytes,
                 modified_at_unix_ms=excluded.modified_at_unix_ms,
                 source_provider=COALESCE(excluded.source_provider, library_items.source_provider),
                 source_book_id=COALESCE(excluded.source_book_id, library_items.source_book_id)"#,
            params![
                path,
                item.id,
                item.title,
                authors_json,
                item.format.as_str(),
                size_bytes,
                item.added_at_unix_ms,
                item.modified_at_unix_ms,
                item.source_provider,
                item.source_book_id,
            ],
        )?;
        self.by_path(&item.path)?
            .ok_or_else(|| anyhow::anyhow!("library item was not persisted"))
    }

    pub fn by_path(&self, path: &Path) -> Result<Option<LibraryItem>> {
        let key = path.to_string_lossy().into_owned();
        let row = self
            .conn()?
            .query_row(
                r#"SELECT id, title, authors_json, path, format, size_bytes,
                          added_at_unix_ms, modified_at_unix_ms, source_provider, source_book_id
                   FROM library_items WHERE path=?1"#,
                [key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                    ))
                },
            )
            .optional()?;
        row.map(library_item_from_row).transpose()
    }

    pub fn list(&self) -> Result<Vec<LibraryItem>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            r#"SELECT id, title, authors_json, path, format, size_bytes,
                      added_at_unix_ms, modified_at_unix_ms, source_provider, source_book_id
               FROM library_items
               ORDER BY added_at_unix_ms DESC, title COLLATE NOCASE"#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        rows.map(|row| row.map_err(Into::into).and_then(library_item_from_row))
            .collect()
    }

    pub fn list_existing(&self) -> Result<Vec<LibraryItem>> {
        self.import_completed_downloads()?;
        self.prune_missing()?;
        self.list()
    }

    pub fn remove(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()?
            .execute("DELETE FROM library_items WHERE id=?1", [id])?
            == 1)
    }

    pub fn prune_missing(&self) -> Result<usize> {
        let missing = self
            .list()?
            .into_iter()
            .filter(|item| !item.path.is_file())
            .map(|item| item.path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(0);
        }
        let conn = self.conn()?;
        let mut removed = 0;
        for path in missing {
            removed += conn.execute("DELETE FROM library_items WHERE path=?1", [path])?;
        }
        Ok(removed)
    }

    fn import_completed_downloads(&self) -> Result<usize> {
        let completed = {
            let conn = self.conn()?;
            let has_download_tasks = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='download_tasks')",
                [],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if !has_download_tasks {
                return Ok(0);
            }
            let mut statement = conn.prepare(
                r#"SELECT title, provider_id, book_id, destination
                   FROM download_tasks
                   WHERE state='completed'
                   ORDER BY updated_at_unix_ms DESC"#,
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut imported = 0;
        for (title, provider_id, book_id, destination) in completed {
            let path = PathBuf::from(destination);
            if !path.is_file() || self.by_path(&path)?.is_some() {
                continue;
            }
            if let Ok(item) = LibraryItem::inspect(&path, Some(&title), provider_id, book_id) {
                self.upsert(item)?;
                imported += 1;
            }
        }
        Ok(imported)
    }
}

type LibraryItemRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
);

fn library_item_from_row(row: LibraryItemRow) -> Result<LibraryItem> {
    let (
        id,
        title,
        authors_json,
        path,
        format,
        size_bytes,
        added_at_unix_ms,
        modified_at_unix_ms,
        source_provider,
        source_book_id,
    ) = row;
    let path = PathBuf::from(path);
    let parsed_format = match format.as_str() {
        "epub" => LibraryFormat::Epub,
        "pdf" => LibraryFormat::Pdf,
        "mobi" => LibraryFormat::Mobi,
        "azw3" => LibraryFormat::Azw3,
        "txt" => LibraryFormat::Txt,
        "cbz" => LibraryFormat::Cbz,
        "djvu" => LibraryFormat::Djvu,
        _ => LibraryFormat::from_path(&path),
    };
    Ok(LibraryItem {
        id,
        title,
        authors: serde_json::from_str(&authors_json).unwrap_or_default(),
        path,
        format: parsed_format,
        size_bytes: size_bytes.max(0) as u64,
        added_at_unix_ms,
        modified_at_unix_ms,
        source_provider,
        source_book_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn rescanning_same_path_preserves_library_id() {
        let store = LibraryStore::open_memory().unwrap();
        let path = std::env::temp_dir().join(format!("lumina-library-{}.epub", Uuid::new_v4()));
        fs::write(&path, b"epub-test").unwrap();

        let first = LibraryItem::inspect(&path, Some("First title"), None, None).unwrap();
        let first = store.upsert(first).unwrap();
        let second = LibraryItem::inspect(
            &path,
            Some("Updated title"),
            Some("provider".into()),
            Some("book".into()),
        )
        .unwrap();
        let second = store.upsert(second).unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.title, "Updated title");
        assert_eq!(second.source_provider.as_deref(), Some("provider"));
        assert_eq!(store.list().unwrap().len(), 1);

        let _ = fs::remove_file(path);
    }
}
