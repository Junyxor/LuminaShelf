use anyhow::{anyhow, Context, Result};
use encoding_rs::GBK;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek},
    path::{Path, PathBuf},
};
use zip::ZipArchive;

const MAX_CHAPTER_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TXT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
struct ManifestItem {
    href: String,
    media_type: Option<String>,
    properties: Option<String>,
}

type PackageManifest = HashMap<String, ManifestItem>;

#[derive(Debug)]
struct PackageData {
    title: Option<String>,
    manifest: PackageManifest,
    spine: Vec<String>,
    toc_id: Option<String>,
}

#[derive(Debug, Clone)]
struct TocEntry {
    href: String,
    title: String,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderBookMetadata {
    pub title: String,
    pub chapters: Vec<ReaderChapterMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderChapterMetadata {
    pub id: String,
    pub title: String,
}

pub fn open_book(path: impl AsRef<Path>) -> Result<ReaderBook> {
    let path = canonical_reader_path(path)?;
    match reader_extension(&path).as_str() {
        "txt" => open_txt(&path),
        "epub" => open_epub(&path),
        other => Err(anyhow!("reader does not support {other} yet")),
    }
}

pub fn open_book_metadata(path: impl AsRef<Path>) -> Result<ReaderBookMetadata> {
    let path = canonical_reader_path(path)?;
    match reader_extension(&path).as_str() {
        "txt" => {
            let title = file_title(&path);
            Ok(ReaderBookMetadata {
                title: title.clone(),
                chapters: vec![ReaderChapterMetadata {
                    id: "text".to_string(),
                    title,
                }],
            })
        }
        "epub" => open_epub_metadata(&path),
        other => Err(anyhow!("reader does not support {other} yet")),
    }
}

pub fn open_book_chapter(path: impl AsRef<Path>, chapter_id: &str) -> Result<ReaderChapter> {
    let path = canonical_reader_path(path)?;
    match reader_extension(&path).as_str() {
        "txt" => {
            if chapter_id != "text" {
                return Err(anyhow!("text reader only exposes the text chapter"));
            }
            open_txt(&path)?
                .chapters
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("text file has no readable chapter"))
        }
        "epub" => open_epub_chapter(&path, chapter_id),
        other => Err(anyhow!("reader does not support {other} yet")),
    }
}

fn canonical_reader_path(path: impl AsRef<Path>) -> Result<PathBuf> {
    std::fs::canonicalize(path.as_ref())
        .with_context(|| format!("open reader file {}", path.as_ref().display()))
}

fn reader_extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
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
            html: None,
        }],
    })
}

fn open_epub(path: &Path) -> Result<ReaderBook> {
    let (mut archive, package_dir, package_data) = load_epub_package(path)?;
    let toc_titles = load_toc_titles(&mut archive, &package_dir, &package_data);
    let mut chapters = Vec::new();

    for (index, idref) in package_data.spine.iter().enumerate() {
        let Some(item) = package_data.manifest.get(idref) else {
            continue;
        };
        let entry = normalize_zip_path(&package_dir.join(&item.href));
        let html = match read_zip_text(&mut archive, &entry) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let text = html_to_text(&html);
        if text.trim().is_empty() {
            continue;
        }
        let chapter_title = toc_titles
            .get(&normalize_toc_href(&item.href))
            .cloned()
            .or_else(|| extract_html_title(&html).filter(|value| !value.trim().is_empty()))
            .unwrap_or_else(|| format!("第 {} 章", index + 1));
        chapters.push(ReaderChapter {
            id: idref.clone(),
            title: chapter_title,
            text,
            html: formatted_chapter(&mut archive, &entry, &html),
        });
    }
    if chapters.is_empty() {
        return Err(anyhow!("EPUB does not contain readable spine chapters"));
    }
    Ok(ReaderBook {
        title: package_data.title.unwrap_or_else(|| file_title(path)),
        chapters,
    })
}

fn open_epub_metadata(path: &Path) -> Result<ReaderBookMetadata> {
    let (mut archive, package_dir, package_data) = load_epub_package(path)?;
    let toc_titles = load_toc_titles(&mut archive, &package_dir, &package_data);
    let chapters = epub_chapter_metadata(&package_data, &toc_titles);
    if chapters.is_empty() {
        return Err(anyhow!("EPUB does not contain readable spine chapters"));
    }
    Ok(ReaderBookMetadata {
        title: package_data.title.unwrap_or_else(|| file_title(path)),
        chapters,
    })
}

fn open_epub_chapter(path: &Path, chapter_id: &str) -> Result<ReaderChapter> {
    let (mut archive, package_dir, package_data) = load_epub_package(path)?;
    let index = package_data
        .spine
        .iter()
        .position(|idref| idref == chapter_id)
        .ok_or_else(|| anyhow!("EPUB chapter is not in the spine"))?;
    let item = package_data
        .manifest
        .get(chapter_id)
        .ok_or_else(|| anyhow!("EPUB chapter is missing from the manifest"))?;
    let entry = normalize_zip_path(&package_dir.join(&item.href));
    let html = read_zip_text(&mut archive, &entry)?;
    let text = html_to_text(&html);
    let formatted = formatted_chapter(&mut archive, &entry, &html);
    if text.trim().is_empty()
        && !formatted
            .as_ref()
            .is_some_and(|html| html.contains("<img "))
    {
        return Err(anyhow!("EPUB chapter does not contain readable text"));
    }
    let toc_titles = load_toc_titles(&mut archive, &package_dir, &package_data);
    let title = toc_titles
        .get(&normalize_toc_href(&item.href))
        .cloned()
        .or_else(|| extract_html_title(&html).filter(|value| !value.trim().is_empty()))
        .unwrap_or_else(|| format!("第 {} 章", index + 1));
    Ok(ReaderChapter {
        id: chapter_id.to_string(),
        title,
        text,
        html: formatted,
    })
}

fn load_epub_package(path: &Path) -> Result<(ZipArchive<File>, PathBuf, PackageData)> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file).context("open EPUB archive")?;
    let container = read_zip_text(&mut archive, "META-INF/container.xml")?;
    let rootfile = parse_rootfile_path(&container)?;
    let package = read_zip_text(&mut archive, &rootfile)?;
    let package_dir = PathBuf::from(&rootfile)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let package_data = parse_package(&package)?;
    Ok((archive, package_dir, package_data))
}

// A small structural renderer rather than arbitrary author HTML/CSS. EPUB content
// is untrusted: scripts, event attributes, external URLs and style rules never reach
// the WebView. Local raster images are bounded and embedded as non-executable data.
fn formatted_chapter<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entry: &str,
    html: &str,
) -> Option<String> {
    let document = roxmltree::Document::parse(html).ok()?;
    let body = document
        .descendants()
        .find(|node| node.has_tag_name("body"))?;
    let directory = Path::new(entry).parent().unwrap_or(Path::new(""));
    let mut budget = 8 * 1024 * 1024_u64;
    let mut output = String::new();
    render_safe_node(body, archive, directory, &mut budget, &mut output, 0);
    Some(output)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_safe_node<R: Read + Seek>(
    node: roxmltree::Node<'_, '_>,
    archive: &mut ZipArchive<R>,
    directory: &Path,
    image_budget: &mut u64,
    output: &mut String,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    if node.is_text() {
        output.push_str(&escape_html(node.text().unwrap_or("")));
        return;
    }
    if !node.is_element() {
        return;
    }
    let tag = node.tag_name().name().to_ascii_lowercase();
    if matches!(
        tag.as_str(),
        "script"
            | "style"
            | "iframe"
            | "object"
            | "embed"
            | "form"
            | "svg"
            | "head"
            | "template"
            | "video"
            | "audio"
    ) {
        return;
    }
    if tag == "img" {
        if let Some(src) = node
            .attribute("src")
            .filter(|src| !src.contains(':') && !src.starts_with("//"))
        {
            let relative = percent_decode_path(src.split('#').next().unwrap_or(src));
            let name = normalize_zip_path(&directory.join(relative));
            if let Ok(mut file) = archive.by_name(&name) {
                let size = file.size();
                if size <= 4 * 1024 * 1024 && size <= *image_budget {
                    let mut bytes = Vec::new();
                    let limit = (4 * 1024 * 1024_u64).min(*image_budget);
                    if file
                        .by_ref()
                        .take(limit + 1)
                        .read_to_end(&mut bytes)
                        .is_ok()
                        && bytes.len() as u64 <= limit
                    {
                        let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                            Some("image/png")
                        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                            Some("image/jpeg")
                        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
                            Some("image/gif")
                        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
                            Some("image/webp")
                        } else {
                            None
                        };
                        if let Some(mime) = mime {
                            use base64::Engine;
                            *image_budget -= bytes.len() as u64;
                            output.push_str(&format!(
                                "<img loading=\"lazy\" alt=\"{}\" src=\"data:{mime};base64,{}\" />",
                                escape_html(node.attribute("alt").unwrap_or("")),
                                base64::engine::general_purpose::STANDARD.encode(bytes)
                            ));
                        }
                    }
                }
            }
        }
        return;
    }
    let allowed = matches!(
        tag.as_str(),
        "p" | "div"
            | "span"
            | "section"
            | "article"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "strong"
            | "b"
            | "em"
            | "i"
            | "u"
            | "s"
            | "blockquote"
            | "pre"
            | "code"
            | "ul"
            | "ol"
            | "li"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "td"
            | "th"
            | "caption"
            | "figure"
            | "figcaption"
            | "sup"
            | "sub"
            | "ruby"
            | "rt"
            | "rp"
            | "br"
            | "hr"
    );
    if allowed {
        output.push('<');
        output.push_str(&tag);
        output.push('>');
    }
    for child in node.children() {
        render_safe_node(child, archive, directory, image_budget, output, depth + 1);
    }
    if allowed && tag != "br" && tag != "hr" {
        output.push_str("</");
        output.push_str(&tag);
        output.push('>');
    }
}

fn epub_chapter_metadata(
    package: &PackageData,
    toc_titles: &HashMap<String, String>,
) -> Vec<ReaderChapterMetadata> {
    package
        .spine
        .iter()
        .enumerate()
        .filter_map(|(index, idref)| {
            let item = package.manifest.get(idref)?;
            Some(ReaderChapterMetadata {
                id: idref.clone(),
                title: toc_titles
                    .get(&normalize_toc_href(&item.href))
                    .cloned()
                    .unwrap_or_else(|| format!("第 {} 章", index + 1)),
            })
        })
        .collect()
}

fn load_toc_titles<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    package_dir: &Path,
    package: &PackageData,
) -> HashMap<String, String> {
    if let Some(nav_item) = package.manifest.values().find(|item| {
        item.properties
            .as_deref()
            .is_some_and(|properties| properties.split_whitespace().any(|value| value == "nav"))
    }) {
        let entry = normalize_zip_path(&package_dir.join(&nav_item.href));
        if let Ok(xml) = read_zip_text(archive, &entry) {
            let entries = parse_epub3_nav(&xml);
            if !entries.is_empty() {
                return toc_map(entries);
            }
        }
    }

    let ncx_item = package
        .toc_id
        .as_ref()
        .and_then(|id| package.manifest.get(id))
        .or_else(|| {
            package
                .manifest
                .values()
                .find(|item| item.media_type.as_deref() == Some("application/x-dtbncx+xml"))
        });
    if let Some(ncx_item) = ncx_item {
        let entry = normalize_zip_path(&package_dir.join(&ncx_item.href));
        if let Ok(xml) = read_zip_text(archive, &entry) {
            return toc_map(parse_epub2_ncx(&xml));
        }
    }
    HashMap::new()
}

fn toc_map(entries: Vec<TocEntry>) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    for entry in entries.into_iter().filter(|entry| !entry.title.is_empty()) {
        titles
            .entry(normalize_toc_href(&entry.href))
            .or_insert(entry.title);
    }
    titles
}

fn parse_epub3_nav(xml: &str) -> Vec<TocEntry> {
    let Ok(document) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    let toc_nav = document.descendants().find(|node| {
        node.is_element()
            && node.tag_name().name().eq_ignore_ascii_case("nav")
            && node.attributes().any(|attribute| {
                attribute.name().eq_ignore_ascii_case("type")
                    && attribute
                        .value()
                        .split_whitespace()
                        .any(|value| value == "toc")
            })
    });
    let Some(toc_nav) = toc_nav else {
        return Vec::new();
    };
    toc_nav
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name().eq_ignore_ascii_case("a"))
        .filter_map(|node| {
            let href = node.attribute("href")?;
            let title = collapse_whitespace(
                &node
                    .descendants()
                    .filter(|child| child.is_text())
                    .filter_map(|child| child.text())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            (!title.is_empty()).then(|| TocEntry {
                href: href.to_string(),
                title,
            })
        })
        .collect()
}

fn parse_epub2_ncx(xml: &str) -> Vec<TocEntry> {
    let Ok(document) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "navPoint")
        .filter_map(|nav_point| {
            let href = nav_point
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "content")?
                .attribute("src")?;
            let title = nav_point
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "navLabel")?
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "text")?
                .text()
                .map(collapse_whitespace)?;
            (!title.is_empty()).then(|| TocEntry {
                href: href.to_string(),
                title,
            })
        })
        .collect()
}

fn normalize_toc_href(value: &str) -> String {
    let without_fragment = value.split('#').next().unwrap_or(value);
    let decoded = percent_decode_path(without_fragment);
    normalize_zip_path(Path::new(&decoded))
}

fn percent_decode_path(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                output.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
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

fn parse_package(xml: &str) -> Result<PackageData> {
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
                ManifestItem {
                    href: node.attribute("href")?.to_string(),
                    media_type: node.attribute("media-type").map(str::to_string),
                    properties: node.attribute("properties").map(str::to_string),
                },
            ))
        })
        .collect();
    let spine_node = document
        .descendants()
        .find(|node| node.has_tag_name("spine"));
    let toc_id = spine_node
        .and_then(|node| node.attribute("toc"))
        .map(str::to_string);
    let spine = spine_node
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name("itemref"))
        .filter_map(|node| node.attribute("idref").map(str::to_string))
        .collect();
    Ok(PackageData {
        title,
        manifest,
        spine,
        toc_id,
    })
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
    let decoded = if let Some(stripped) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        String::from_utf8_lossy(stripped).into_owned()
    } else if let Some(stripped) = bytes.strip_prefix(&[0xff, 0xfe]) {
        decode_utf16(stripped, true)
    } else if let Some(stripped) = bytes.strip_prefix(&[0xfe, 0xff]) {
        decode_utf16(stripped, false)
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        text.to_string()
    } else {
        let (text, _, _) = GBK.decode(bytes);
        text.into_owned()
    };
    normalize_newlines(decoded)
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|pair| {
        if little_endian {
            u16::from_le_bytes(*pair)
        } else {
            u16::from_be_bytes(*pair)
        }
    });
    char::decode_utf16(units)
        .map(|value| value.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

fn normalize_newlines(value: String) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
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

    #[test]
    fn decodes_utf16le_bom_text() {
        let bytes = [0xff, 0xfe, b'Z', 0, b'\n', 0];
        assert_eq!(decode_text(&bytes), "Z\n");
    }

    #[test]
    fn decodes_gbk_text() {
        let (bytes, _, _) = GBK.encode("中文测试");
        assert_eq!(decode_text(&bytes), "中文测试");
    }

    #[test]
    fn parses_epub3_nav_titles() {
        let xml = r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="Text/ch1.xhtml#start">第一章 开始</a></li><li><a href="Text/ch2.xhtml">第二章 继续</a></li></ol></nav></body></html>"#;
        let entries = parse_epub3_nav(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "第一章 开始");
        assert_eq!(normalize_toc_href(&entries[0].href), "Text/ch1.xhtml");
    }

    #[test]
    fn parses_epub2_ncx_titles() {
        let xml = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="n1"><navLabel><text>Chapter One</text></navLabel><content src="Text/ch1.xhtml#p1"/></navPoint></navMap></ncx>"#;
        let entries = parse_epub2_ncx(xml);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Chapter One");
        assert_eq!(normalize_toc_href(&entries[0].href), "Text/ch1.xhtml");
    }

    #[test]
    fn decodes_percent_encoded_toc_paths() {
        assert_eq!(
            normalize_toc_href("Text/%E7%AC%AC%E4%B8%80%E7%AB%A0.xhtml#top"),
            "Text/第一章.xhtml"
        );
    }

    #[test]
    fn keeps_first_title_for_multiple_anchors_in_one_chapter() {
        let titles = toc_map(vec![
            TocEntry {
                href: "Text/ch1.xhtml#start".to_string(),
                title: "第一章".to_string(),
            },
            TocEntry {
                href: "Text/ch1.xhtml#section-2".to_string(),
                title: "第一章第二节".to_string(),
            },
        ]);
        assert_eq!(
            titles.get("Text/ch1.xhtml").map(String::as_str),
            Some("第一章")
        );
    }

    #[test]
    fn metadata_uses_toc_titles_without_loading_chapter_html() {
        let package = PackageData {
            title: Some("Book".to_string()),
            manifest: HashMap::from([
                (
                    "ch1".to_string(),
                    ManifestItem {
                        href: "Text/ch1.xhtml".to_string(),
                        media_type: Some("application/xhtml+xml".to_string()),
                        properties: None,
                    },
                ),
                (
                    "ch2".to_string(),
                    ManifestItem {
                        href: "Text/ch2.xhtml".to_string(),
                        media_type: Some("application/xhtml+xml".to_string()),
                        properties: None,
                    },
                ),
            ]),
            spine: vec!["ch1".to_string(), "ch2".to_string()],
            toc_id: None,
        };
        let toc = HashMap::from([("Text/ch1.xhtml".to_string(), "第一章".to_string())]);
        let chapters = epub_chapter_metadata(&package, &toc);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].title, "第一章");
        assert_eq!(chapters[1].title, "第 2 章");
    }
}
