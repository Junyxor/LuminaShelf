use lumina_core::{
    open_book_chapter, open_book_metadata, ReaderBookMetadata, ReaderChapter,
};
use serde::Serialize;

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

#[tauri::command]
pub fn open_local_book(path: String) -> Result<LazyReaderBook, String> {
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

#[tauri::command]
pub fn open_local_book_metadata(path: String) -> Result<ReaderBookMetadata, String> {
    open_book_metadata(path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_local_book_chapter(
    path: String,
    chapter_id: String,
) -> Result<ReaderChapter, String> {
    open_book_chapter(path, &chapter_id).map_err(|error| error.to_string())
}
