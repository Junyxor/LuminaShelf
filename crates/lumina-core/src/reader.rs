use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek},
    path::{Path, PathBuf},
};
use zip::ZipArchive;

const MAX_CHAPTER_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TXT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderBook {
    pub title: String,
    pub chapters: Vec<ReaderChapter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderChapter {
    pub id: String,
    pub title: String,
    pub text: String,
}

pub fn open_book(path: impl AsRef<Path>) -> Result<ReaderBook> {
    let path = std::fs::canonicalize(path.as_ref())
        .with_context(|| format!("open reader file {}", path.as_ref().display()))?;
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "txt" => open_txt(&path),
        "epub" => open_epub(&path),
        other => Err(anyhow!("reader does not support {other} yet")),
    }
}

fn open_txt(path: &Path) -> Result<ReaderBook> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_TXT_BYTES {
        return Err(anyhow!("text file is too large for the built-in reader"));
    }
    let bytes = std::fs::read(path)?;
    let text = decode_text(&bytes);
    Ok(ReaderBook {
        title: file_title(path),
        chapters: vec![ReaderChapter {
            id: "text".to_string(),
            title: file_title(path),
            text,
        }],
    })
}

fn open_epub(path: &Path) -> Result<ReaderBook> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).context("open EPUB archive")?;
    let container = read_zip_text(&mut archive, "META-INF/container.xml")?;
    let rootfile = parse_rootfile_path(&container)?;
    let package = read_zip_text(&mut archive, &rootfile)?;
    let package_dir = PathBuf::from(&rootfile)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let (title, manifest, spine) = parse_package(&package)?;
    let mut chapters = Vec::new();
    for (index, idref) in spine.into_iter().enumerate() {
        let Some(href) = manifest.get(&idref) else {
            continue;
        };
        let entry = normalize_zip_path(&package_dir.join(href));
        let html = match read_zip_text(&mut archive, &entry) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let text = html_to_text(&html);
        if text.trim().is_empty() {
            continue;
        }
        let chapter_title = extract_html_title(&html)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("第 {} 章", index + 1));
        chapters.push(ReaderChapter {
            id: idref,
            title: chapter_title,
            text,
        });
    }
    if chapters.is_empty() {
        return Err(anyhow!("EPUB does not contain readable spine chapters"));
    }
    Ok(ReaderBook {
        title: title.unwrap_or_else(|| file_title(path)),
        chapters,
    })
}

fn read_zip_text<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> Result<String> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("read EPUB entry {name}"))?;
    if entry.size() > MAX_CHAPTER_BYTES {
        return Err(anyhow!("EPUB entry is too large"));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes)?;
    Ok(decode_text(&bytes))
}

fn parse_rootfile_path(xml: &str) -> Result<String> {
    let document = roxmltree::Document::parse(xml).context("parse EPUB container.xml")?;
    document
        .descendants()
        .find(|node| node.has_tag_name("rootfile"))
        .and_then(|node| node.attribute("full-path"))
        .map(str::to_string)
        .ok_or_else(|| anyhow!("EPUB container has no rootfile"))
}

fn parse_package(
    xml: &str,
) -> Result<(
    Option<String>,
    std::collections::HashMap<String, String>,
    Vec<String>,
)> {
    let document = roxmltree::Document::parse(xml).context("parse EPUB package document")?;
    let title = document
        .descendants()
        .find(|node| node.tag_name().name() == "title")
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let manifest = document
        .descendants()
        .filter(|node| node.has_tag_name("item"))
        .filter_map(|node| {
            Some((
                node.attribute("id")?.to_string(),
                node.attribute("href")?.to_string(),
            ))
        })
        .collect();
    let spine = document
        .descendants()
        .filter(|node| node.has_tag_name("itemref"))
        .filter_map(|node| node.attribute("idref").map(str::to_string))
        .collect();
    Ok((title, manifest, spine))
}

fn normalize_zip_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            _ => {}
        }
    }
    parts.join("/")
}

fn decode_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn file_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled")
        .to_string()
}

fn extract_html_title(html: &str) -> Option<String> {
    let document = roxmltree::Document::parse(html).ok()?;
    for tag in ["h1", "h2", "title"] {
        if let Some(value) = document
            .descendants()
            .find(|node| node.tag_name().name().eq_ignore_ascii_case(tag))
            .and_then(|node| node.text())
        {
            let cleaned = collapse_whitespace(value);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

fn html_to_text(html: &str) -> String {
    let Ok(document) = roxmltree::Document::parse(html) else {
        return strip_markup_fallback(html);
    };
    let mut output = String::new();
    for node in document.descendants() {
        if node.is_text() {
            if let Some(text) = node.text() {
                let cleaned = collapse_whitespace(text);
                if !cleaned.is_empty() {
                    if !output.is_empty() {
                        output.push(' ');
                    }
                    output.push_str(&cleaned);
                }
            }
        } else if node.is_element()
            && matches!(
                node.tag_name().name().to_ascii_lowercase().as_str(),
                "p" | "div" | "section" | "article" | "h1" | "h2" | "h3" | "li" | "br"
            )
            && !output.ends_with('\n')
        {
            output.push('\n');
        }
    }
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn strip_markup_fallback(html: &str) -> String {
    let mut output = String::new();
    let mut inside = false;
    for ch in html.chars() {
        match ch {
            '<' => inside = true,
            '>' => {
                inside = false;
                output.push(' ');
            }
            _ if !inside => output.push(ch),
            _ => {}
        }
    }
    collapse_whitespace(&output)
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_markup_strip_keeps_text() {
        assert_eq!(
            strip_markup_fallback("<p>Hello <b>world</b></p>"),
            "Hello world"
        );
    }

    #[test]
    fn zip_path_normalizes_parent_components() {
        assert_eq!(
            normalize_zip_path(Path::new("OPS/../Text/ch1.xhtml")),
            "Text/ch1.xhtml"
        );
    }
}
