use std::path::PathBuf;
use tauri::ipc::Response;

const MAX_PDF_BYTES: u64 = 96 * 1024 * 1024;

#[tauri::command]
pub(crate) fn read_pdf_bytes(path: String) -> Result<Response, String> {
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

    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| format!("inspect PDF {}: {error}", canonical.display()))?;
    if !metadata.is_file() {
        return Err("PDF reader path must be a file".to_string());
    }
    if metadata.len() > MAX_PDF_BYTES {
        return Err(format!(
            "PDF is too large for the current in-memory reader ({} MiB limit)",
            MAX_PDF_BYTES / 1024 / 1024
        ));
    }

    let data = std::fs::read(&canonical)
        .map_err(|error| format!("read PDF {}: {error}", canonical.display()))?;
    if !data.starts_with(b"%PDF-") {
        return Err("file does not have a valid PDF header".to_string());
    }
    Ok(Response::new(data))
}
