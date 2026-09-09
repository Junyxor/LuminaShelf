use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BookFormat {
    Epub,
    Pdf,
    Mobi,
    Azw3,
    Txt,
    Cbz,
    Other,
}

impl BookFormat {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "epub" => Self::Epub,
            "pdf" => Self::Pdf,
            "mobi" => Self::Mobi,
            "azw3" | "azw" => Self::Azw3,
            "txt" => Self::Txt,
            "cbz" | "cbr" => Self::Cbz,
            _ => Self::Other,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Epub => "epub",
            Self::Pdf => "pdf",
            Self::Mobi => "mobi",
            Self::Azw3 => "azw3",
            Self::Txt => "txt",
            Self::Cbz => "cbz",
            Self::Other => "bin",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub authenticated: bool,
    pub downloadable: bool,
    pub searchable: bool,
    pub paginated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDescriptor {
    pub id: String,
    pub name: String,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub text: String,
    pub page: u32,
    pub page_size: u16,
    pub formats: Vec<BookFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSummary {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub language: Option<String>,
    pub format: Option<BookFormat>,
    #[serde(default)]
    pub available_formats: Vec<BookFormat>,
    pub size_bytes: Option<u64>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub items: Vec<BookSummary>,
    pub page: u32,
    pub has_next: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetails {
    #[serde(flatten)]
    pub summary: BookSummary,
    pub description: Option<String>,
    pub identifiers: Vec<(String, String)>,
}
