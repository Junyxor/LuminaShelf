use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};
use tauri::ipc::Response;

const MAX_LEGACY_PDF_BYTES: u64 = 96 * 1024 * 1024;
const MAX_PDF_RANGE_BYTES: u64 = 4 * 1024 * 1024;
const PDF_HEADER_BYTES: usize = 5;
const RANGE_LENGTH_PREFIX_BYTES: usize = 8;

fn open_pdf(path: String) -> Result<(File, PathBuf, u64), String> {
    let requested = PathBuf::from(path);
    let canonical = std::fs::canonicalize(&requested)
        .map_err(|error| format!("open PDF {}: {error}", requested.display()))?;
    if canonical
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| !value.eq_ignore_ascii_case("pdf"))
        .unwrap_or(true)
    {
        return Err("PDF reader only accepts .pdf files".to_string());
    }

    let mut file = File::open(&canonical)
        .map_err(|error| format!("open PDF {}: {error}", canonical.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect PDF {}: {error}", canonical.display()))?;
    if !metadata.is_file() {
        return Err("PDF reader path must be a file".to_string());
    }
    if metadata.len() < PDF_HEADER_BYTES as u64 {
        return Err("file does not have a valid PDF header".to_string());
    }

    let mut header = [0u8; PDF_HEADER_BYTES];
    file.read_exact(&mut header)
        .map_err(|error| format!("read PDF header {}: {error}", canonical.display()))?;
    if &header != b"%PDF-" {
        return Err("file does not have a valid PDF header".to_string());
    }

    Ok((file, canonical, metadata.len()))
}

#[tauri::command]
pub(crate) fn read_pdf_bytes(
    path: String,
    start: Option<u64>,
    end: Option<u64>,
) -> Result<Response, String> {
    let (mut file, canonical, length) = open_pdf(path)?;

    match (start, end) {
        (Some(start), Some(end)) => {
            if start >= length {
                return Err(format!(
                    "PDF range start {start} is outside the file ({length} bytes)"
                ));
            }
            let end = end.min(length);
            if end <= start {
                return Err("PDF range end must be greater than start".to_string());
            }
            let range_len = end - start;
            if range_len > MAX_PDF_RANGE_BYTES {
                return Err(format!(
                    "PDF range is too large ({} MiB limit)",
                    MAX_PDF_RANGE_BYTES / 1024 / 1024
                ));
            }

            file.seek(SeekFrom::Start(start))
                .map_err(|error| format!("seek PDF {}: {error}", canonical.display()))?;
            let mut data = vec![0u8; range_len as usize];
            file.read_exact(&mut data)
                .map_err(|error| format!("read PDF range {}: {error}", canonical.display()))?;

            let mut payload = Vec::with_capacity(RANGE_LENGTH_PREFIX_BYTES + data.len());
            payload.extend_from_slice(&length.to_le_bytes());
            payload.extend_from_slice(&data);
            Ok(Response::new(payload))
        }
        (None, None) => {
            if length > MAX_LEGACY_PDF_BYTES {
                return Err(format!(
                    "PDF is too large for the legacy in-memory reader ({} MiB limit); use range loading",
                    MAX_LEGACY_PDF_BYTES / 1024 / 1024
                ));
            }
            file.seek(SeekFrom::Start(0))
                .map_err(|error| format!("seek PDF {}: {error}", canonical.display()))?;
            let mut data = Vec::with_capacity(length as usize);
            file.read_to_end(&mut data)
                .map_err(|error| format!("read PDF {}: {error}", canonical.display()))?;
            Ok(Response::new(data))
        }
        _ => Err("PDF range requires both start and end".to_string()),
    }
}
