use super::StateStore;
use crate::library::{LibraryFormat, LibraryItem};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::path::PathBuf;

impl StateStore {
    fn ensure_library_items_schema(&self) -> Result<()> {
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

    pub fn upsert_library_item(&self, item: LibraryItem) -> Result<LibraryItem> {
        self.ensure_library_items_schema()?;
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
        self.library_item_by_path(&item.path)?
            .ok_or_else(|| anyhow::anyhow!("library item was not persisted"))
    }

    pub fn library_item_by_path(&self, path: &std::path::Path) -> Result<Option<LibraryItem>> {
        self.ensure_library_items_schema()?;
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

    pub fn list_library_items(&self) -> Result<Vec<LibraryItem>> {
        self.ensure_library_items_schema()?;
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

    pub fn remove_library_item(&self, id: &str) -> Result<bool> {
        self.ensure_library_items_schema()?;
        Ok(self
            .conn()?
            .execute("DELETE FROM library_items WHERE id=?1", [id])?
            == 1)
    }

    pub fn prune_missing_library_items(&self) -> Result<usize> {
        self.ensure_library_items_schema()?;
        let missing = self
            .list_library_items()?
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
        let store = StateStore::open_memory().unwrap();
        let path = std::env::temp_dir().join(format!("lumina-library-{}.epub", Uuid::new_v4()));
        fs::write(&path, b"epub-test").unwrap();

        let first = LibraryItem::inspect(&path, Some("First title"), None, None).unwrap();
        let first = store.upsert_library_item(first).unwrap();
        let second = LibraryItem::inspect(
            &path,
            Some("Updated title"),
            Some("provider".into()),
            Some("book".into()),
        )
        .unwrap();
        let second = store.upsert_library_item(second).unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.title, "Updated title");
        assert_eq!(second.source_provider.as_deref(), Some("provider"));
        assert_eq!(store.list_library_items().unwrap().len(), 1);

        let _ = fs::remove_file(path);
    }
}
