use lumina_core::{
    backup::{export_backup, restore_backup},
    library::import_reader,
    open_book_chapter, Bookmark, ReadingProgress, StateStore,
};
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, ZipWriter};

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn portable_backup_restores_books_positions_and_bookmarks_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let source = StateStore::open_memory().unwrap();
    let item = import_reader(
        Cursor::new("My portable book"),
        &temp.path().join("source"),
        "Author - Book.txt",
    )
    .unwrap();
    source.upsert_library_item(&item).unwrap();
    source
        .set_reading_progress(ReadingProgress {
            library_id: item.id.clone(),
            locator: Some(r#"{"chapterId":"text","scroll":0.6}"#.into()),
            fraction: 0.6,
            updated_at_unix_ms: 1,
        })
        .unwrap();
    source
        .upsert_bookmark(&Bookmark {
            id: "bookmark".into(),
            library_id: item.id.clone(),
            label: "Remember".into(),
            locator: r#"{"chapterId":"text","scroll":0.4}"#.into(),
            fraction: 0.4,
            created_at_unix_ms: 2,
        })
        .unwrap();
    let mut zip = Cursor::new(Vec::new());
    assert_eq!(export_backup(&source, &mut zip).unwrap(), 1);
    let target = StateStore::open_memory().unwrap();
    let existing = import_reader(
        Cursor::new("Do not change me"),
        &temp.path().join("target"),
        "Existing.txt",
    )
    .unwrap();
    target.upsert_library_item(&existing).unwrap();
    zip.set_position(0);
    assert_eq!(
        restore_backup(&target, zip, &temp.path().join("target")).unwrap(),
        1
    );
    let items = target.list_library_items().unwrap();
    assert_eq!(items.len(), 2);
    let restored = items.iter().find(|item| item.title == "Book").unwrap();
    assert_ne!(restored.id, item.id);
    assert_eq!(restored.authors, vec!["Author"]);
    assert_eq!(
        std::fs::read_to_string(&existing.path).unwrap(),
        "Do not change me"
    );
    assert_eq!(
        target
            .reading_progress(&restored.id)
            .unwrap()
            .unwrap()
            .fraction,
        0.6
    );
    assert_eq!(
        target.list_bookmarks(&restored.id).unwrap()[0].label,
        "Remember"
    );
}

#[test]
fn malformed_restore_rolls_back_copied_files_and_does_not_extract_paths() {
    let temp = tempfile::tempdir().unwrap();
    let store = StateStore::open_memory().unwrap();
    let manifest = br#"{"version":1,"createdAtUnixMs":0,"books":[
        {"id":"a","title":"Valid","authors":[],"entry":"books/0.txt","progress":null,"bookmarks":[]},
        {"id":"b","title":"Invalid","authors":[],"entry":"../../escape.txt","progress":null,"bookmarks":[]}
    ]}"#;
    let zip = archive(&[
        ("luminashelf.json", manifest),
        ("books/0.txt", b"valid"),
        ("../../escape.txt", b"bad"),
    ]);
    assert!(restore_backup(&store, Cursor::new(zip), temp.path()).is_err());
    assert!(store.list_library_items().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn epub_formatting_preserves_structure_and_images_but_removes_active_content() {
    let temp = tempfile::tempdir().unwrap();
    let zip = archive(&[
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="OPS/package.opf"/></rootfiles></container>"#),
        ("OPS/package.opf", br#"<package><metadata><title>Safe book</title></metadata><manifest><item id="c" href="chapter.xhtml"/></manifest><spine><itemref idref="c"/></spine></package>"#),
        ("OPS/chapter.xhtml", br#"<html><body><h1 onclick="steal()">Heading</h1><p>A <strong>bold</strong> paragraph.</p><script>attack()</script><iframe src="https://evil.test"/><img src="image.png" onerror="attack()" alt="Art"/><img src="https://evil.test/track.png"/><a href="javascript:attack()">Link text</a></body></html>"#),
        ("OPS/image.png", b"\x89PNG\r\n\x1a\nexample"),
    ]);
    let path = temp.path().join("book.epub");
    std::fs::write(&path, zip).unwrap();
    let chapter = open_book_chapter(path, "c").unwrap();
    let html = chapter.html.unwrap();
    assert!(html.contains("<h1>Heading</h1>"));
    assert!(html.contains("<strong>bold</strong>"));
    assert!(html.contains("data:image/png;base64,"));
    assert!(!html.contains("attack"));
    assert!(!html.contains("onclick"));
    assert!(!html.contains("onerror"));
    assert!(!html.contains("https://"));
    assert!(!html.contains("javascript:"));
}
