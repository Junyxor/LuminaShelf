use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

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
    if max_items == 0 {
        return Ok(Vec::new());
    }
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

/// Copy a user-selected stream (desktop file or Android SAF handle) into managed
/// storage. A failed copy never appears as an ebook, and existing books are never
/// overwritten. Original display names, not UUIDs, become library metadata.
pub fn import_reader(
    source: impl Read,
    directory: &Path,
    display_name: &str,
) -> Result<LibraryItem> {
    import_reader_with_limit(source, directory, display_name, 512 * 1024 * 1024)
}

pub(crate) fn import_reader_with_limit(
    mut source: impl Read,
    directory: &Path,
    display_name: &str,
    limit: u64,
) -> Result<LibraryItem> {
    let name = display_name.rsplit(['/', '\\']).next().unwrap_or("book");
    let format = LibraryFormat::from_path(Path::new(name));
    if format == LibraryFormat::Other {
        return Err(anyhow!("不支持的电子书格式：{name}"));
    }
    fs::create_dir_all(directory)?;
    let id = Uuid::new_v4();
    let partial = directory.join(format!("{id}.importing"));
    let destination = directory.join(format!("{id}.{}", format.as_str()));
    let result = (|| -> Result<LibraryItem> {
        let mut target = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial)?;
        let length = std::io::copy(&mut source.by_ref().take(limit + 1), &mut target)?;
        if length > limit {
            return Err(anyhow!("电子书超过导入大小限制"));
        }
        if length == 0 {
            return Err(anyhow!("不能导入空文件：{name}"));
        }
        target.flush()?;
        target.sync_all()?;
        drop(target);
        validate_ebook(&partial, format)?;
        fs::rename(&partial, &destination)?;
        let stem = Path::new(name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Untitled");
        let (title, authors) = infer_title_and_authors(stem);
        let mut item = LibraryItem::inspect(&destination, Some(&title), None, None)?;
        item.authors = authors;
        Ok(item)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
        let _ = fs::remove_file(&destination);
    }
    result
}

/// Validate containers before publishing downloads or imports to the bookshelf.
/// This rejects HTTP error pages saved with an ebook extension.
pub fn validate_ebook(path: &Path, format: LibraryFormat) -> Result<()> {
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() == 0 {
        return Err(anyhow!("电子书文件为空"));
    }
    match format {
        LibraryFormat::Epub => {
            let mut archive = zip::ZipArchive::new(file).context("无效的 EPUB 压缩容器")?;
            let container = archive
                .by_name("META-INF/container.xml")
                .context("EPUB 缺少 container.xml")?;
            if container.size() == 0 {
                return Err(anyhow!("EPUB container.xml 为空"));
            }
        }
        LibraryFormat::Pdf => {
            let mut header = [0; 5];
            file.read_exact(&mut header).context("PDF 文件头不完整")?;
            if &header != b"%PDF-" {
                return Err(anyhow!("无效的 PDF 文件头"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Publish without replacing an existing file. Both paths are on the same
/// filesystem. Retry after an interrupted finalization is idempotent only when
/// the existing file has exactly the staged bytes.
pub fn publish_download(staging: &Path, destination: &Path) -> Result<()> {
    if let Err(error) = validate_ebook(staging, LibraryFormat::from_path(destination)) {
        fs::remove_file(staging).context("清理无效下载内容失败")?;
        let manifest = PathBuf::from(format!("{}.lumina-part.json", staging.display()));
        if manifest.exists() {
            fs::remove_file(manifest)?;
        }
        return Err(error);
    }
    match publish_without_overwrite(staging, destination) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut source = fs::File::open(staging)?;
            let mut existing = fs::File::open(destination)?;
            if source.metadata()?.len() != existing.metadata()?.len() {
                return Err(anyhow!("目标文件已存在且内容不同；下载临时文件已保留"));
            }
            let mut left = [0u8; 64 * 1024];
            let mut right = [0u8; 64 * 1024];
            loop {
                let count = source.read(&mut left)?;
                if count == 0 {
                    break;
                }
                existing.read_exact(&mut right[..count])?;
                if left[..count] != right[..count] {
                    return Err(anyhow!("目标文件已存在且内容不同；下载临时文件已保留"));
                }
            }
        }
        Err(error) => return Err(error).context("发布已完成下载失败"),
    }
    if staging.exists() {
        fs::remove_file(staging)?;
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn publish_without_overwrite(staging: &Path, destination: &Path) -> std::io::Result<()> {
    fs::hard_link(staging, destination)
}

#[cfg(target_os = "android")]
fn publish_without_overwrite(staging: &Path, destination: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(staging.as_os_str().as_bytes())?;
    let target = CString::new(destination.as_os_str().as_bytes())?;
    // Android SELinux policies can disallow hard links. renameat2 is supported by
    // the Android kernels we target and RENAME_NOREPLACE preserves existing books.
    // SAF streams are copied into private storage first, so both paths share a FS.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
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
