use crate::{
    download::DownloadState,
    library::now_unix_ms,
    provider::BookFormat,
    user_state::{AccountProfile, FavoriteBook, ReadingProgress},
};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedDownloadTask {
    pub id: String,
    pub provider_id: String,
    pub book_id: String,
    pub title: String,
    pub format: BookFormat,
    pub destination: PathBuf,
    pub state: DownloadState,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

impl PersistedDownloadTask {
    pub fn new(
        id: impl Into<String>,
        provider_id: impl Into<String>,
        book_id: impl Into<String>,
        title: impl Into<String>,
        format: BookFormat,
        destination: PathBuf,
    ) -> Self {
        let now = now_unix_ms();
        Self {
            id: id.into(),
            provider_id: provider_id.into(),
            book_id: book_id.into(),
            title: title.into(),
            format,
            destination,
            state: DownloadState::Queued,
            downloaded_bytes: 0,
            total_bytes: None,
            error: None,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
        }
    }
}

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

            CREATE TABLE IF NOT EXISTS download_tasks (
              id TEXT PRIMARY KEY,
              provider_id TEXT NOT NULL,
              book_id TEXT NOT NULL,
              title TEXT NOT NULL,
              format TEXT NOT NULL,
              destination TEXT NOT NULL,
              state TEXT NOT NULL,
              downloaded_bytes INTEGER NOT NULL DEFAULT 0,
              total_bytes INTEGER,
              error TEXT,
              created_at_unix_ms INTEGER NOT NULL,
              updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS download_tasks_updated_idx
              ON download_tasks(updated_at_unix_ms DESC);
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

    pub fn upsert_download_task(&self, task: &PersistedDownloadTask) -> Result<()> {
        self.conn()?.execute(
            r#"INSERT INTO download_tasks(
                 id, provider_id, book_id, title, format, destination, state,
                 downloaded_bytes, total_bytes, error, created_at_unix_ms, updated_at_unix_ms
               ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
               ON CONFLICT(id) DO UPDATE SET
                 provider_id=excluded.provider_id,
                 book_id=excluded.book_id,
                 title=excluded.title,
                 format=excluded.format,
                 destination=excluded.destination,
                 state=excluded.state,
                 downloaded_bytes=excluded.downloaded_bytes,
                 total_bytes=excluded.total_bytes,
                 error=excluded.error,
                 updated_at_unix_ms=excluded.updated_at_unix_ms"#,
            params![
                task.id,
                task.provider_id,
                task.book_id,
                task.title,
                task.format.extension(),
                task.destination.to_string_lossy(),
                task.state.as_str(),
                task.downloaded_bytes.min(i64::MAX as u64) as i64,
                task.total_bytes.map(|value| value.min(i64::MAX as u64) as i64),
                task.error,
                task.created_at_unix_ms,
                task.updated_at_unix_ms,
            ],
        )?;
        Ok(())
    }

    pub fn update_download_task(
        &self,
        id: &str,
        state: DownloadState,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn()?.execute(
            r#"UPDATE download_tasks SET
                 state=?2,
                 downloaded_bytes=?3,
                 total_bytes=COALESCE(?4, total_bytes),
                 error=?5,
                 updated_at_unix_ms=?6
               WHERE id=?1"#,
            params![
                id,
                state.as_str(),
                downloaded_bytes.min(i64::MAX as u64) as i64,
                total_bytes.map(|value| value.min(i64::MAX as u64) as i64),
                error,
                now_unix_ms(),
            ],
        )?;
        Ok(())
    }

    pub fn list_download_tasks(&self) -> Result<Vec<PersistedDownloadTask>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            r#"SELECT id, provider_id, book_id, title, format, destination, state,
                      downloaded_bytes, total_bytes, error, created_at_unix_ms, updated_at_unix_ms
               FROM download_tasks
               ORDER BY updated_at_unix_ms DESC"#,
        )?;
        let rows = statement.query_map([], |row| {
            let format: String = row.get(4)?;
            let state: String = row.get(6)?;
            let downloaded: i64 = row.get(7)?;
            let total: Option<i64> = row.get(8)?;
            Ok(PersistedDownloadTask {
                id: row.get(0)?,
                provider_id: row.get(1)?,
                book_id: row.get(2)?,
                title: row.get(3)?,
                format: BookFormat::parse(&format),
                destination: PathBuf::from(row.get::<_, String>(5)?),
                state: DownloadState::parse(&state),
                downloaded_bytes: downloaded.max(0) as u64,
                total_bytes: total.map(|value| value.max(0) as u64),
                error: row.get(9)?,
                created_at_unix_ms: row.get(10)?,
                updated_at_unix_ms: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn recover_incomplete_downloads(&self) -> Result<usize> {
        let changed = self.conn()?.execute(
            r#"UPDATE download_tasks
               SET state='paused', error=NULL, updated_at_unix_ms=?1
               WHERE state IN ('queued', 'connecting', 'downloading', 'verifying')"#,
            [now_unix_ms()],
        )?;
        Ok(changed)
    }

    pub fn remove_download_task(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()?
            .execute("DELETE FROM download_tasks WHERE id=?1", [id])?
            == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn download_queue_survives_and_recovers_active_tasks() {
        let store = StateStore::open_memory().unwrap();
        let mut task = PersistedDownloadTask::new(
            "zlibrary:42:epub",
            "zlibrary",
            "42",
            "Example",
            BookFormat::Epub,
            PathBuf::from("/tmp/Example.epub"),
        );
        task.state = DownloadState::Downloading;
        task.downloaded_bytes = 1024;
        task.total_bytes = Some(4096);
        store.upsert_download_task(&task).unwrap();
        assert_eq!(store.recover_incomplete_downloads().unwrap(), 1);

        let restored = store.list_download_tasks().unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].state, DownloadState::Paused);
        assert_eq!(restored[0].downloaded_bytes, 1024);
        assert_eq!(restored[0].total_bytes, Some(4096));
        assert_eq!(restored[0].provider_id, "zlibrary");
    }
}
