use anyhow::{ensure, Context, Result};
use std::{fs::File, io::Write, path::Path};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

fn xml(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c == '\n' || *c == '\r' || *c == '\t' || *c >= ' ')
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Text-only conversion. Images and source typography are deliberately omitted.
pub fn convert_text_book(source: &Path, destination: &Path, title: &str) -> Result<()> {
    crate::library::validate_readable_file(source)?;
    let input = crate::LibraryFormat::from_path(source);
    let output = crate::LibraryFormat::from_path(destination);
    ensure!(
        matches!(
            (input, output),
            (crate::LibraryFormat::Txt, crate::LibraryFormat::Epub)
                | (crate::LibraryFormat::Epub, crate::LibraryFormat::Txt)
        ),
        "本地转换支持 TXT ↔ EPUB；PDF、扫描件与其他格式暂不支持"
    );
    let book = crate::open_book_metadata(source)?;
    if output == crate::LibraryFormat::Txt {
        let mut file = File::create(destination)?;
        writeln!(file, "{title}\n")?;
        for item in &book.chapters {
            let chapter = crate::open_book_chapter(source, &item.id)?;
            writeln!(file, "{}\n\n{}\n", chapter.title, chapter.text)?;
        }
        file.sync_all()?;
    } else {
        let mut zip = ZipWriter::new(File::create(destination)?);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let compressed =
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("mimetype", stored)?;
        zip.write_all(b"application/epub+zip")?;
        zip.start_file("META-INF/container.xml", compressed)?;
        zip.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#)?;
        let mut manifest = String::new();
        let mut spine = String::new();
        let mut nav = String::new();
        for (index, item) in book.chapters.iter().enumerate() {
            let chapter = crate::open_book_chapter(source, &item.id)?;
            let path = format!("chapter-{index}.xhtml");
            manifest.push_str(&format!(
                "<item id=\"c{index}\" href=\"{path}\" media-type=\"application/xhtml+xml\"/>"
            ));
            spine.push_str(&format!("<itemref idref=\"c{index}\"/>"));
            nav.push_str(&format!("<navPoint id=\"n{index}\" playOrder=\"{}\"><navLabel><text>{}</text></navLabel><content src=\"{path}\"/></navPoint>",index+1,xml(&chapter.title)));
            zip.start_file(format!("OEBPS/{path}"), compressed)?;
            let paragraphs = chapter
                .text
                .split("\n\n")
                .map(|p| format!("<p>{}</p>", xml(p).replace('\n', "<br/>")))
                .collect::<String>();
            write!(zip,"<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{}</title></head><body><h1>{}</h1>{paragraphs}</body></html>",xml(&chapter.title),xml(&chapter.title))?;
        }
        let identifier = format!("urn:uuid:{}", uuid::Uuid::new_v4());
        zip.start_file("OEBPS/content.opf", compressed)?;
        write!(zip,"<?xml version=\"1.0\" encoding=\"UTF-8\"?><package version=\"2.0\" xmlns=\"http://www.idpf.org/2007/opf\" unique-identifier=\"uid\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:language>zh</dc:language><dc:identifier id=\"uid\">{identifier}</dc:identifier></metadata><manifest><item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>{manifest}</manifest><spine toc=\"ncx\">{spine}</spine></package>",xml(title))?;
        zip.start_file("OEBPS/toc.ncx", compressed)?;
        write!(zip,"<?xml version=\"1.0\" encoding=\"UTF-8\"?><ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head><meta name=\"dtb:uid\" content=\"{identifier}\"/></head><docTitle><text>{}</text></docTitle><navMap>{nav}</navMap></ncx>",xml(title))?;
        zip.finish()?.sync_all()?;
    }
    crate::library::validate_readable_file(destination).context("转换后文件无法重新打开")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_text_epub_text_round_trip_preserves_chapters_and_text() {
        let directory =
            std::env::temp_dir().join(format!("lumina-convert-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let text = directory.join("原文.txt");
        let epub = directory.join("转换.epub");
        let roundtrip = directory.join("回读.txt");
        std::fs::write(&text, "第一段 <&> 中文内容。\n\n第二段阅读验证。").unwrap();
        convert_text_book(&text, &epub, "中文书籍").unwrap();
        convert_text_book(&epub, &roundtrip, "中文书籍").unwrap();
        let output = std::fs::read_to_string(&roundtrip).unwrap();
        assert!(output.contains("第一段 <&> 中文内容。"));
        assert!(output.contains("第二段阅读验证。"));
        let bad = directory.join("接口.epub");
        std::fs::write(
            &bad,
            br#"{"success":1,"file":{"downloadLink":"https://example.org/book.epub"}}"#,
        )
        .unwrap();
        assert!(crate::library::validate_readable_file(&bad)
            .unwrap_err()
            .to_string()
            .contains("JSON"));
        std::fs::remove_file(bad).unwrap();
        std::fs::remove_file(text).unwrap();
        std::fs::remove_file(epub).unwrap();
        std::fs::remove_file(roundtrip).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
