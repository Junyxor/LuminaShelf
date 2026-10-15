use crate::{
    download::DownloadTask,
    library::{now_unix_ms, LibraryFormat, LibraryItem},
    network::DnsProfile,
    user_state::{AccountProfile, FavoriteBook, ReadingProgress},
};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone)]
pub struct StateStore {
    connection: Arc<Mutex<Connection>>,
}

impl StateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path).context("open LuminaShelf SQLite state")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.migrate()?;
        Ok(store)
    }

    fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("SQLite state lock was poisoned"))
    }

    fn migrate(&self) -> Result<()> {
        self.conn()?.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS app_settings (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS download_tasks (
              id TEXT PRIMARY KEY,
              task_json TEXT NOT NULL,
              created_at_unix_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS dns_profiles (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              policy_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS accounts (
              id TEXT PRIMARY KEY,
              provider_id TEXT NOT NULL,
              label TEXT NOT NULL,
              login_hint TEXT,
              active INTEGER NOT NULL DEFAULT 0,
              created_at_unix_ms INTEGER NOT NULL,
              updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS accounts_provider_idx ON accounts(provider_id);
            CREATE UNIQUE INDEX IF NOT EXISTS accounts_one_active_provider
              ON accounts(provider_id) WHERE active = 1;

            CREATE TABLE IF NOT EXISTS favorites (
              provider_id TEXT NOT NULL,
              book_id TEXT NOT NULL,
              title TEXT NOT NULL,
              authors_json TEXT NOT NULL,
              format TEXT,
              cover_url TEXT,
              added_at_unix_ms INTEGER NOT NULL,
              PRIMARY KEY(provider_id, book_id)
            );

            CREATE TABLE IF NOT EXISTS reading_progress (
              library_id TEXT PRIMARY KEY,
              locator TEXT,
              fraction REAL NOT NULL DEFAULT 0,
              updated_at_unix_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS library_items (
              id TEXT PRIMARY KEY,
              title TEXT NOT NULL,
              authors_json TEXT NOT NULL,
              path TEXT NOT NULL UNIQUE,
              format TEXT NOT NULL,
              size_bytes INTEGER NOT NULL,
              added_at_unix_ms INTEGER NOT NULL,
              modified_at_unix_ms INTEGER,
              source_provider TEXT,
              source_book_id TEXT,
              updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS library_items_updated_idx
              ON library_items(updated_at_unix_ms DESC);
            "#,
        )?;
        Ok(())
    }

    pub fn upsert_account(&self, profile: &AccountProfile) -> Result<()> {
        self.conn()?.execute(
            r#"INSERT INTO accounts(id, provider_id, label, login_hint, active, created_at_unix_ms, updated_at_unix_ms)
               VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
               ON CONFLICT(id) DO UPDATE SET
                 provider_id=excluded.provider_id,
                 label=excluded.label,
                 login_hint=excluded.login_hint,
                 active=excluded.active,
                 updated_at_unix_ms=excluded.updated_at_unix_ms"#,
            params![profile.id, profile.provider_id, profile.label, profile.login_hint, profile.active as i64, profile.created_at_unix_ms, profile.updated_at_unix_ms],
        )?;
        Ok(())
    }

    pub fn download_tasks(&self) -> Result<Vec<DownloadTask>> {
        let connection = self.conn()?;
        let mut statement = connection
            .prepare("SELECT task_json FROM download_tasks ORDER BY created_at_unix_ms DESC, id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    pub fn save_download_task(&self, task: &DownloadTask) -> Result<()> {
        self.conn()?.execute("INSERT INTO download_tasks(id,task_json,created_at_unix_ms) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET task_json=excluded.task_json",
            params![task.id, serde_json::to_string(task)?, task.created_at_unix_ms])?;
        Ok(())
    }
    pub fn remove_download_task(&self, id: &str) -> Result<()> {
        self.conn()?
            .execute("DELETE FROM download_tasks WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn list_accounts(&self) -> Result<Vec<AccountProfile>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT id, provider_id, label, login_hint, active, created_at_unix_ms, updated_at_unix_ms FROM accounts ORDER BY provider_id, active DESC, label"
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AccountProfile {
                id: row.get(0)?,
                provider_id: row.get(1)?,
                label: row.get(2)?,
                login_hint: row.get(3)?,
                active: row.get::<_, i64>(4)? != 0,
                created_at_unix_ms: row.get(5)?,
                updated_at_unix_ms: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn active_account(&self, provider_id: &str) -> Result<Option<AccountProfile>> {
        self.conn()?.query_row(
            "SELECT id, provider_id, label, login_hint, active, created_at_unix_ms, updated_at_unix_ms FROM accounts WHERE provider_id=?1 AND active=1 LIMIT 1",
            [provider_id],
            |row| Ok(AccountProfile {
                id: row.get(0)?, provider_id: row.get(1)?, label: row.get(2)?, login_hint: row.get(3)?, active: true,
                created_at_unix_ms: row.get(5)?, updated_at_unix_ms: row.get(6)?,
            }),
        ).optional().map_err(Into::into)
    }

    pub fn activate_account(&self, provider_id: &str, account_id: &str) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let now = now_unix_ms();
        tx.execute(
            "UPDATE accounts SET active=0, updated_at_unix_ms=?2 WHERE provider_id=?1",
            params![provider_id, now],
        )?;
        let changed = tx.execute(
            "UPDATE accounts SET active=1, updated_at_unix_ms=?3 WHERE provider_id=?1 AND id=?2",
            params![provider_id, account_id, now],
        )?;
        if changed != 1 {
            anyhow::bail!("account does not exist for provider");
        }
        tx.commit()?;
        Ok(())
    }

    pub fn deactivate_provider(&self, provider_id: &str) -> Result<()> {
        self.conn()?.execute(
            "UPDATE accounts SET active=0, updated_at_unix_ms=?2 WHERE provider_id=?1",
            params![provider_id, now_unix_ms()],
        )?;
        Ok(())
    }

    pub fn remove_account(&self, account_id: &str) -> Result<bool> {
        Ok(self
            .conn()?
            .execute("DELETE FROM accounts WHERE id=?1", [account_id])?
            == 1)
    }

    pub fn upsert_favorite(&self, favorite: &FavoriteBook) -> Result<()> {
        let authors = serde_json::to_string(&favorite.authors)?;
        self.conn()?.execute(
            r#"INSERT INTO favorites(provider_id, book_id, title, authors_json, format, cover_url, added_at_unix_ms)
               VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
               ON CONFLICT(provider_id, book_id) DO UPDATE SET
                 title=excluded.title, authors_json=excluded.authors_json, format=excluded.format, cover_url=excluded.cover_url"#,
            params![favorite.provider_id, favorite.book_id, favorite.title, authors, favorite.format, favorite.cover_url, favorite.added_at_unix_ms],
        )?;
        Ok(())
    }

    pub fn remove_favorite(&self, provider_id: &str, book_id: &str) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM favorites WHERE provider_id=?1 AND book_id=?2",
            params![provider_id, book_id],
        )? == 1)
    }

    pub fn list_favorites(&self) -> Result<Vec<FavoriteBook>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT provider_id, book_id, title, authors_json, format, cover_url, added_at_unix_ms FROM favorites ORDER BY added_at_unix_ms DESC"
        )?;
        let rows = statement.query_map([], |row| {
            let authors_json: String = row.get(3)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                authors_json,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;
        let mut output = Vec::new();
        for row in rows {
            let (provider_id, book_id, title, authors_json, format, cover_url, added_at_unix_ms) =
                row?;
            output.push(FavoriteBook {
                provider_id,
                book_id,
                title,
                authors: serde_json::from_str(&authors_json).unwrap_or_default(),
                format,
                cover_url,
                added_at_unix_ms,
            });
        }
        Ok(output)
    }

    pub fn remove_library_item(&self, id: &str) -> Result<()> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM reading_progress WHERE library_id=?1", [id])?;
        transaction.execute("DELETE FROM library_items WHERE id=?1", [id])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.conn()?
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn()?.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value])?;
        Ok(())
    }

    pub fn dns_profiles(&self) -> Result<Vec<DnsProfile>> {
        let connection = self.conn()?;
        let mut statement =
            connection.prepare("SELECT id,name,policy_json FROM dns_profiles ORDER BY rowid")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, json) = row?;
            Ok(DnsProfile {
                id,
                name,
                policy: serde_json::from_str(&json)?,
            })
        })
        .collect()
    }

    pub fn save_dns_profile(&self, mut profile: DnsProfile) -> Result<DnsProfile> {
        profile.name = profile.name.trim().to_string();
        anyhow::ensure!(
            !profile.name.is_empty() && profile.name.chars().count() <= 80,
            "DNS 名称需为 1–80 个字符"
        );
        profile.policy = profile.policy.validated()?;
        if profile.id.is_empty() {
            profile.id = uuid::Uuid::new_v4().to_string();
        }
        let json = serde_json::to_string(&profile.policy)?;
        self.conn()?.execute("INSERT INTO dns_profiles(id,name,policy_json) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,policy_json=excluded.policy_json", params![profile.id,profile.name,json])?;
        Ok(profile)
    }

    pub fn remove_dns_profile(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()?
            .execute("DELETE FROM dns_profiles WHERE id=?1", [id])?
            == 1)
    }

    pub fn upsert_library_item(&self, item: &LibraryItem) -> Result<()> {
        let authors = serde_json::to_string(&item.authors)?;
        let size_bytes = i64::try_from(item.size_bytes).context("library item is too large")?;
        self.conn()?.execute(
            r#"INSERT INTO library_items(
                 id, title, authors_json, path, format, size_bytes, added_at_unix_ms,
                 modified_at_unix_ms, source_provider, source_book_id, updated_at_unix_ms
               ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
               ON CONFLICT(id) DO UPDATE SET
                 title=excluded.title,
                 authors_json=excluded.authors_json,
                 path=excluded.path,
                 format=excluded.format,
                 size_bytes=excluded.size_bytes,
                 modified_at_unix_ms=excluded.modified_at_unix_ms,
                 source_provider=COALESCE(excluded.source_provider, library_items.source_provider),
                 source_book_id=COALESCE(excluded.source_book_id, library_items.source_book_id),
                 updated_at_unix_ms=excluded.updated_at_unix_ms"#,
            params![
                item.id,
                item.title,
                authors,
                item.path.to_string_lossy(),
                item.format.as_str(),
                size_bytes,
                item.added_at_unix_ms,
                item.modified_at_unix_ms,
                item.source_provider,
                item.source_book_id,
                now_unix_ms(),
            ],
        )?;
        Ok(())
    }

    pub fn upsert_library_items(&self, items: &[LibraryItem]) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let updated_at = now_unix_ms();
        for item in items {
            let authors = serde_json::to_string(&item.authors)?;
            let size_bytes = i64::try_from(item.size_bytes).context("library item is too large")?;
            tx.execute(
                r#"INSERT INTO library_items(
                     id, title, authors_json, path, format, size_bytes, added_at_unix_ms,
                     modified_at_unix_ms, source_provider, source_book_id, updated_at_unix_ms
                   ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                   ON CONFLICT(id) DO UPDATE SET
                     title=excluded.title,
                     authors_json=excluded.authors_json,
                     path=excluded.path,
                     format=excluded.format,
                     size_bytes=excluded.size_bytes,
                     modified_at_unix_ms=excluded.modified_at_unix_ms,
                     source_provider=COALESCE(excluded.source_provider, library_items.source_provider),
                     source_book_id=COALESCE(excluded.source_book_id, library_items.source_book_id),
                     updated_at_unix_ms=excluded.updated_at_unix_ms"#,
                params![
                    item.id,
                    item.title,
                    authors,
                    item.path.to_string_lossy(),
                    item.format.as_str(),
                    size_bytes,
                    item.added_at_unix_ms,
                    item.modified_at_unix_ms,
                    item.source_provider,
                    item.source_book_id,
                    updated_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_library_items(&self) -> Result<Vec<LibraryItem>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            r#"SELECT id, title, authors_json, path, format, size_bytes, added_at_unix_ms,
                      modified_at_unix_ms, source_provider, source_book_id
               FROM library_items
               ORDER BY updated_at_unix_ms DESC, title COLLATE NOCASE"#,
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
        let mut output = Vec::new();
        for row in rows {
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
            ) = row?;
            output.push(LibraryItem {
                id,
                title,
                authors: serde_json::from_str(&authors_json).unwrap_or_default(),
                path: PathBuf::from(path),
                format: library_format_from_str(&format),
                size_bytes: u64::try_from(size_bytes.max(0)).unwrap_or_default(),
                added_at_unix_ms,
                modified_at_unix_ms,
                source_provider,
                source_book_id,
            });
        }
        Ok(output)
    }

    pub fn set_reading_progress(&self, progress: ReadingProgress) -> Result<()> {
        let progress = progress.normalized();
        self.conn()?.execute(
            r#"INSERT INTO reading_progress(library_id, locator, fraction, updated_at_unix_ms)
               VALUES(?1, ?2, ?3, ?4)
               ON CONFLICT(library_id) DO UPDATE SET locator=excluded.locator, fraction=excluded.fraction, updated_at_unix_ms=excluded.updated_at_unix_ms"#,
            params![progress.library_id, progress.locator, progress.fraction, progress.updated_at_unix_ms],
        )?;
        Ok(())
    }

    pub fn reading_progress(&self, library_id: &str) -> Result<Option<ReadingProgress>> {
        self.conn()?.query_row(
            "SELECT library_id, locator, fraction, updated_at_unix_ms FROM reading_progress WHERE library_id=?1",
            [library_id],
            |row| Ok(ReadingProgress { library_id: row.get(0)?, locator: row.get(1)?, fraction: row.get(2)?, updated_at_unix_ms: row.get(3)? }),
        ).optional().map_err(Into::into)
    }
}

fn library_format_from_str(value: &str) -> LibraryFormat {
    match value {
        "epub" => LibraryFormat::Epub,
        "pdf" => LibraryFormat::Pdf,
        "mobi" => LibraryFormat::Mobi,
        "azw3" => LibraryFormat::Azw3,
        "txt" => LibraryFormat::Txt,
        "cbz" => LibraryFormat::Cbz,
        "djvu" => LibraryFormat::Djvu,
        _ => LibraryFormat::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_dns_and_favorites_survive_reopening_without_creating_downloads() {
        let directory =
            std::env::temp_dir().join(format!("lumina-profiles-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("state.sqlite3");
        let store = StateStore::open(&path).unwrap();
        let policy = crate::ResolverPolicy {
            mode: crate::ResolverMode::Doh,
            doh_url: Some("https://dns.example.org:8443/custom?profile=mine".into()),
            bootstrap_ips: vec![
                "203.0.113.1".parse().unwrap(),
                "2001:db8::1".parse().unwrap(),
            ],
            ..Default::default()
        };
        let profile = store
            .save_dns_profile(DnsProfile {
                id: String::new(),
                name: "我的 DNS".into(),
                policy,
            })
            .unwrap();
        let mut edited = profile.clone();
        edited.name = "改名后的 DNS".into();
        store.save_dns_profile(edited).unwrap();
        store
            .upsert_favorite(&FavoriteBook {
                provider_id: "gutendex".into(),
                book_id: "11".into(),
                title: "收藏测试".into(),
                authors: vec![],
                format: Some("epub".into()),
                cover_url: None,
                added_at_unix_ms: 1,
            })
            .unwrap();
        assert!(store.list_library_items().unwrap().is_empty());
        drop(store);
        let reopened = StateStore::open(&path).unwrap();
        let records = reopened.dns_profiles().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, profile.id);
        assert_eq!(records[0].name, "改名后的 DNS");
        assert_eq!(records[0].policy.doh_url, profile.policy.doh_url);
        assert_eq!(records[0].policy.bootstrap_ips.len(), 2);
        assert_eq!(reopened.list_favorites().unwrap().len(), 1);
        assert!(reopened.list_library_items().unwrap().is_empty());
        reopened.remove_dns_profile(&profile.id).unwrap();
        assert!(reopened.dns_profiles().unwrap().is_empty());
        drop(reopened);
        for file in [
            path,
            directory.join("state.sqlite3-wal"),
            directory.join("state.sqlite3-shm"),
        ] {
            if file.exists() {
                std::fs::remove_file(file).unwrap();
            }
        }
        std::fs::remove_dir(directory).unwrap();
    }

    fn account(id: &str, provider: &str, active: bool) -> AccountProfile {
        AccountProfile {
            id: id.into(),
            provider_id: provider.into(),
            label: id.into(),
            login_hint: None,
            active,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
        }
    }

    #[test]
    fn activation_is_exclusive_per_provider() {
        let store = StateStore::open_memory().unwrap();
        store
            .upsert_account(&account("a", "zlibrary", false))
            .unwrap();
        store
            .upsert_account(&account("b", "zlibrary", false))
            .unwrap();
        store.activate_account("zlibrary", "a").unwrap();
        store.activate_account("zlibrary", "b").unwrap();
        let rows = store.list_accounts().unwrap();
        assert_eq!(rows.iter().filter(|row| row.active).count(), 1);
        assert_eq!(store.active_account("zlibrary").unwrap().unwrap().id, "b");
    }

    #[test]
    fn reading_fraction_is_clamped() {
        let store = StateStore::open_memory().unwrap();
        store
            .set_reading_progress(ReadingProgress {
                library_id: "book".into(),
                locator: Some("p1".into()),
                fraction: 2.0,
                updated_at_unix_ms: 9,
            })
            .unwrap();
        assert_eq!(
            store.reading_progress("book").unwrap().unwrap().fraction,
            1.0
        );
    }

    #[test]
    fn library_items_round_trip() {
        let store = StateStore::open_memory().unwrap();
        let item = LibraryItem {
            id: "book-1".into(),
            title: "测试书".into(),
            authors: vec!["作者".into()],
            path: PathBuf::from("/tmp/test.epub"),
            format: LibraryFormat::Epub,
            size_bytes: 1024,
            added_at_unix_ms: 7,
            modified_at_unix_ms: Some(8),
            source_provider: Some("zlibrary".into()),
            source_book_id: Some("42".into()),
        };
        store.upsert_library_item(&item).unwrap();
        let items = store.list_library_items().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, item.id);
        assert_eq!(items[0].title, item.title);
        assert_eq!(items[0].authors, item.authors);
        assert_eq!(items[0].path, item.path);
        assert_eq!(items[0].format, item.format);
        assert_eq!(items[0].source_provider, item.source_provider);
        assert_eq!(items[0].source_book_id, item.source_book_id);
    }
}
