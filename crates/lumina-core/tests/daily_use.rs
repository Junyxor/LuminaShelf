use lumina_core::{
    download::DownloadState,
    library::{import_reader, scan_folder},
    open_book, BookFormat, PersistedDownloadTask, ReadingProgress, StateStore,
};
use std::io::{self, Cursor, Read};

#[test]
fn import_read_and_resume_after_reopening_database() {
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("library");
    let first = import_reader(
        Cursor::new("一页书。\n第二段。"),
        &library,
        "作者 - 星书.txt",
    )
    .unwrap();
    let second = import_reader(Cursor::new("different book"), &library, "作者 - 星书.txt").unwrap();
    assert_ne!(first.path, second.path);
    assert_eq!(first.title, "星书");
    assert_eq!(first.authors, vec!["作者"]);
    assert!(first
        .path
        .starts_with(std::fs::canonicalize(&library).unwrap()));
    assert!(open_book(&first.path).unwrap().chapters[0]
        .text
        .contains("第二段"));

    let database = temp.path().join("state.sqlite3");
    {
        let store = StateStore::open(&database).unwrap();
        store
            .upsert_library_items(&[first.clone(), second])
            .unwrap();
        store
            .set_reading_progress(ReadingProgress {
                library_id: first.id.clone(),
                locator: Some("chapter-1".into()),
                fraction: 0.4,
                updated_at_unix_ms: 42,
            })
            .unwrap();
        let mut task = PersistedDownloadTask::new(
            "public:1:epub",
            "public",
            "1",
            "Book",
            BookFormat::Epub,
            library.join("download.epub"),
        );
        task.state = DownloadState::Downloading;
        task.downloaded_bytes = 4;
        task.total_bytes = Some(12);
        store.upsert_download_task(&task).unwrap();
    }
    let reopened = StateStore::open(&database).unwrap();
    assert_eq!(reopened.list_library_items().unwrap().len(), 2);
    let progress = reopened.reading_progress(&first.id).unwrap().unwrap();
    assert_eq!(progress.fraction, 0.4);
    assert_eq!(progress.locator.as_deref(), Some("chapter-1"));
    assert_eq!(reopened.recover_incomplete_downloads().unwrap(), 1);
    let tasks = reopened.list_download_tasks().unwrap();
    assert_eq!(tasks[0].state, DownloadState::Paused);
    assert_eq!(tasks[0].downloaded_bytes, 4);
    assert_eq!(tasks[0].total_bytes, Some(12));
    assert_eq!(scan_folder(&library, true, 10).unwrap().len(), 2);
}

#[test]
fn failed_or_unsupported_import_never_leaves_a_book_on_disk() {
    struct BrokenReader;
    impl Read for BrokenReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("lost file access"))
        }
    }
    let temp = tempfile::tempdir().unwrap();
    assert!(import_reader(BrokenReader, temp.path(), "Book.epub").is_err());
    assert!(import_reader(Cursor::new(""), temp.path(), "Empty.txt").is_err());
    assert!(import_reader(Cursor::new("payload"), temp.path(), "../unsafe.exe").is_err());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    let item = import_reader(Cursor::new("safe"), temp.path(), "../../Outside.txt").unwrap();
    assert!(item
        .path
        .starts_with(std::fs::canonicalize(temp.path()).unwrap()));
    assert_eq!(item.title, "Outside");
    assert!(scan_folder(temp.path(), true, 0).unwrap().is_empty());
}

#[test]
fn publication_preserves_existing_books_and_can_finish_after_interruption() {
    use lumina_core::library::publish_download;
    let temp = tempfile::tempdir().unwrap();
    let partial = temp.path().join("download.part");
    let destination = temp.path().join("book.txt");
    std::fs::write(&partial, b"new book").unwrap();
    std::fs::write(&destination, b"old book").unwrap();
    assert!(publish_download(&partial, &destination).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"old book");
    assert_eq!(std::fs::read(&partial).unwrap(), b"new book");
    let published = temp.path().join("published.txt");
    publish_download(&partial, &published).unwrap();
    assert!(!partial.exists());
    assert_eq!(std::fs::read(&published).unwrap(), b"new book");
    std::fs::hard_link(&published, &partial).unwrap();
    publish_download(&partial, &published).unwrap();
    assert!(!partial.exists());
}

#[test]
fn an_http_error_page_cannot_be_imported_as_epub_or_pdf() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["broken.epub", "broken.pdf"] {
        assert!(import_reader(Cursor::new("<html>Sign in</html>"), temp.path(), name).is_err());
    }
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
