use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub id: String,
    pub provider_id: String,
    pub label: String,
    pub login_hint: Option<String>,
    pub active: bool,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteBook {
    pub provider_id: String,
    pub book_id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub format: Option<String>,
    pub cover_url: Option<String>,
    pub added_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingProgress {
    pub library_id: String,
    pub locator: Option<String>,
    pub fraction: f64,
    pub updated_at_unix_ms: i64,
}

impl ReadingProgress {
    pub fn normalized(mut self) -> Self {
        self.fraction = self.fraction.clamp(0.0, 1.0);
        self
    }
}
