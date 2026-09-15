use lumina_core::{open_book_chapter, open_book_metadata, ReaderBookMetadata, ReaderChapter};
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::UNIX_EPOCH,
};

const MAX_CACHED_CHAPTERS: usize = 8;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LazyReaderBook {
    title: String,
    chapters: Vec<LazyReaderChapter>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LazyReaderChapter {
    id: String,
    title: String,
    text: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChapterCacheKey {
    path: PathBuf,
    size_bytes: u64,
    modified_nanos: u128,
    chapter_id: String,
}

#[derive(Default)]
struct ChapterCache {
    entries: VecDeque<(ChapterCacheKey, ReaderChapter)>,
}

impl ChapterCache {
    fn get(&mut self, key: &ChapterCacheKey) -> Option<ReaderChapter> {
        let index = self.entries.iter().position(|(entry, _)| entry == key)?;
        let (key, chapter) = self.entries.remove(index)?;
        let result = chapter.clone();
        self.entries.push_front((key, chapter));
        Some(result)
    }

    fn insert(&mut self, key: ChapterCacheKey, chapter: ReaderChapter) {
        if let Some(index) = self.entries.iter().position(|(entry, _)| entry == &key) {
            self.entries.remove(index);
        }
        self.entries.push_front((key, chapter));
        self.entries.truncate(MAX_CACHED_CHAPTERS);
    }
}

fn chapter_cache() -> &'static Mutex<ChapterCache> {
    static CACHE: OnceLock<Mutex<ChapterCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ChapterCache::default()))
}

fn chapter_cache_key(path: &Path, chapter_id: String) -> Result<ChapterCacheKey, String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("resolve reader path {}: {error}", path.display()))?;
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("inspect reader path {}: {error}", path.display()))?;
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    Ok(ChapterCacheKey {
        path,
        size_bytes: metadata.len(),
        modified_nanos,
        chapter_id,
    })
}

fn build_lazy_book(path: String) -> Result<LazyReaderBook, String> {
    let metadata = open_book_metadata(&path).map_err(|error| error.to_string())?;
    let chapters = metadata
        .chapters
        .into_iter()
        .map(|chapter| LazyReaderChapter {
            id: chapter.id,
            title: chapter.title,
            text: String::new(),
            path: path.clone(),
        })
        .collect();
    Ok(LazyReaderBook {
        title: metadata.title,
        chapters,
    })
}

fn load_cached_chapter(path: String, chapter_id: String) -> Result<ReaderChapter, String> {
    let key = chapter_cache_key(Path::new(&path), chapter_id.clone())?;
    if let Some(chapter) = chapter_cache()
        .lock()
        .map_err(|_| "reader chapter cache is unavailable".to_string())?
        .get(&key)
    {
        return Ok(chapter);
    }

    let chapter = open_book_chapter(&key.path, &chapter_id).map_err(|error| error.to_string())?;
    chapter_cache()
        .lock()
        .map_err(|_| "reader chapter cache is unavailable".to_string())?
        .insert(key, chapter.clone());
    Ok(chapter)
}

async fn run_reader_task<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("reader task failed: {error}"))?
}

#[tauri::command]
pub async fn open_local_book(path: String) -> Result<LazyReaderBook, String> {
    run_reader_task(move || build_lazy_book(path)).await
}

#[tauri::command]
pub async fn open_local_book_metadata(path: String) -> Result<ReaderBookMetadata, String> {
    run_reader_task(move || open_book_metadata(path).map_err(|error| error.to_string())).await
}

#[tauri::command]
pub async fn open_local_book_chapter(
    path: String,
    chapter_id: String,
) -> Result<ReaderChapter, String> {
    run_reader_task(move || load_cached_chapter(path, chapter_id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(id: &str) -> ReaderChapter {
        ReaderChapter {
            id: id.to_string(),
            title: id.to_string(),
            text: id.to_string(),
        }
    }

    fn key(id: &str) -> ChapterCacheKey {
        ChapterCacheKey {
            path: PathBuf::from("book.epub"),
            size_bytes: 1,
            modified_nanos: 1,
            chapter_id: id.to_string(),
        }
    }

    #[test]
    fn cache_promotes_hits_and_stays_bounded() {
        let mut cache = ChapterCache::default();
        for index in 0..MAX_CACHED_CHAPTERS {
            let id = index.to_string();
            cache.insert(key(&id), chapter(&id));
        }
        assert_eq!(
            cache.get(&key("0")).map(|value| value.id),
            Some("0".to_string())
        );
        cache.insert(key("next"), chapter("next"));
        assert_eq!(cache.entries.len(), MAX_CACHED_CHAPTERS);
        assert!(cache.get(&key("1")).is_none());
        assert!(cache.get(&key("0")).is_some());
    }
}
