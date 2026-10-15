use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub fn validate_readable_file(path: &Path) -> Result<()> {
    let mut file = fs::File::open(path).context("书籍文件不存在或已被移动")?;
    let mut header = vec![0; 4096];
    let count = file.read(&mut header)?;
    header.truncate(count);
    anyhow::ensure!(count > 0, "书籍文件为空，请重新下载");
    let text = String::from_utf8_lossy(&header);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    let lower = text.to_ascii_lowercase();
    anyhow::ensure!(
        !lower.starts_with("<!doctype html") && !lower.starts_with("<html"),
        "下载到的是网页或登录验证页，请更换站点并重新下载"
    );
    if text.starts_with('{') {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&header) {
            anyhow::ensure!(
                value.get("success").is_none()
                    && value.get("file").is_none()
                    && value.get("error").is_none(),
                "文件是书源接口 JSON，不是电子书，请重新下载"
            );
        }
    }
    match LibraryFormat::from_path(path) {
        LibraryFormat::Epub => {
            crate::open_book_metadata(path).context("EPUB 内容损坏或格式不正确，请重新下载")?;
        }
        LibraryFormat::Pdf => {
            anyhow::ensure!(
                header.starts_with(b"%PDF-"),
                "文件不是真正的 PDF，请重新下载"
            );
        }
        LibraryFormat::Txt => {
            crate::open_book_metadata(path).context("无法读取文本编码")?;
        }
        LibraryFormat::Zip => {
            zip::ZipArchive::new(file).context("压缩书包内容损坏")?;
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryFormat {
    Epub,
    Pdf,
    Mobi,
    Azw3,
    Txt,
    Cbz,
    Djvu,
    Zip,
    Other,
}

impl LibraryFormat {
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "epub" => Self::Epub,
            "pdf" => Self::Pdf,
            "mobi" => Self::Mobi,
            "azw3" | "azw" => Self::Azw3,
            "txt" => Self::Txt,
            "cbz" | "cbr" => Self::Cbz,
            "djvu" | "djv" => Self::Djvu,
            "zip" => Self::Zip,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Epub => "epub",
            Self::Pdf => "pdf",
            Self::Mobi => "mobi",
            Self::Azw3 => "azw3",
            Self::Txt => "txt",
            Self::Cbz => "cbz",
            Self::Djvu => "djvu",
            Self::Zip => "zip",
            Self::Other => "other",
        }
    }

    pub fn is_supported_path(path: &Path) -> bool {
        !matches!(Self::from_path(path), Self::Other)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub path: PathBuf,
    pub format: LibraryFormat,
    pub size_bytes: u64,
    pub added_at_unix_ms: i64,
    pub modified_at_unix_ms: Option<i64>,
    pub source_provider: Option<String>,
    pub source_book_id: Option<String>,
}

impl LibraryItem {
    pub fn inspect(
        path: impl AsRef<Path>,
        title_hint: Option<&str>,
        source_provider: Option<String>,
        source_book_id: Option<String>,
    ) -> Result<Self> {
        let requested = expand_home(path.as_ref());
        let path = fs::canonicalize(&requested)
            .with_context(|| format!("open library file {}", requested.display()))?;
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(anyhow!("library item must be a file"));
        }
        if !LibraryFormat::is_supported_path(&path) {
            return Err(anyhow!("unsupported ebook format"));
        }

        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Untitled")
            .trim();
        let (guessed_title, authors) = infer_title_and_authors(stem);
        let title = title_hint
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&guessed_title)
            .to_string();
        let stable_key = path.to_string_lossy().replace('\\', "/");
        let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, stable_key.as_bytes()).to_string();

        Ok(Self {
            id,
            title,
            authors,
            format: LibraryFormat::from_path(&path),
            size_bytes: metadata.len(),
            added_at_unix_ms: now_unix_ms(),
            modified_at_unix_ms: metadata.modified().ok().map(system_time_ms),
            path,
            source_provider,
            source_book_id,
        })
    }
}

pub fn scan_folder(
    path: impl AsRef<Path>,
    recursive: bool,
    max_items: usize,
) -> Result<Vec<PathBuf>> {
    let requested = expand_home(path.as_ref());
    let root = fs::canonicalize(&requested)
        .with_context(|| format!("open library folder {}", requested.display()))?;
    if !root.is_dir() {
        return Err(anyhow!("library scan path must be a directory"));
    }

    let mut pending = vec![root];
    let mut files = Vec::new();
    while let Some(folder) = pending.pop() {
        for entry in
            fs::read_dir(&folder).with_context(|| format!("read folder {}", folder.display()))?
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(value) => value,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                if recursive {
                    pending.push(path);
                }
            } else if file_type.is_file() && LibraryFormat::is_supported_path(&path) {
                files.push(path);
                if files.len() >= max_items {
                    return Ok(files);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn infer_title_and_authors(stem: &str) -> (String, Vec<String>) {
    if let Some((left, right)) = stem.split_once(" - ") {
        let author = left.trim();
        let title = right.trim();
        if !author.is_empty() && !title.is_empty() {
            return (title.to_string(), vec![author.to_string()]);
        }
    }
    (stem.replace(['_', '.'], " ").trim().to_string(), Vec::new())
}

fn expand_home(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let remainder = raw.trim_start_matches('~').trim_start_matches(['/', '\\']);
            return PathBuf::from(home).join(remainder);
        }
    }
    path.to_path_buf()
}

pub fn now_unix_ms() -> i64 {
    system_time_ms(SystemTime::now())
}

fn system_time_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_is_path_based() {
        let path = std::env::temp_dir().join(format!("lumina-library-{}.txt", Uuid::new_v4()));
        std::fs::write(&path, "hello").unwrap();
        let first = LibraryItem::inspect(&path, None, None, None).unwrap();
        let second = LibraryItem::inspect(&path, None, None, None).unwrap();
        assert_eq!(first.id, second.id);
        let _ = std::fs::remove_file(path);
    }
}
