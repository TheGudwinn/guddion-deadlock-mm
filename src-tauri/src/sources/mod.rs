pub mod cache;
pub mod gamebanana;

use crate::error::NetError;
use crate::net::download::Progress;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error(transparent)]
    Net(#[from] NetError),
    #[error("unexpected response from the source: {0}")]
    Parse(String),
    #[error("not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(String),
}

impl serde::Serialize for SourceError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    #[default]
    Default,
    Newest,
    LatestModified,
    MostLiked,
    MostViewed,
    MostDownloaded,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchQuery {
    /// Free-text search. When set, category and sort are ignored (the source's search endpoint has neither).
    pub text: Option<String>,
    /// Category (or sub-category, e.g. a hero) id from `categories()`.
    pub category: Option<i64>,
    pub sort: Sort,
    /// 1-based.
    pub page: u32,
    pub per_page: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModSummary {
    pub source: String,
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub category: Option<String>,
    pub thumbnail: Option<String>,
    pub version: Option<String>,
    pub likes: u64,
    pub views: u64,
    pub updated_at: i64,
    pub page_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPage {
    pub items: Vec<ModSummary>,
    pub total: u64,
    pub page: u32,
    pub has_more: bool,
    /// Set when the data came from the offline cache: unix time it was fetched.
    pub cached_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModFile {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub url: String,
    /// GameBanana publishes MD5 only (no SHA-256).
    pub md5: Option<String>,
    /// The source's own antivirus scan reported "clean".
    pub av_clean: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDetails {
    pub summary: ModSummary,
    /// Raw HTML from the source; the UI must sanitize it before rendering.
    pub description_html: Option<String>,
    pub downloads: Option<u64>,
    pub images: Vec<String>,
    pub files: Vec<ModFile>,
    pub cached_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub item_count: u64,
    pub has_children: bool,
}

#[async_trait]
pub trait ModSource: Send + Sync {
    fn id(&self) -> &'static str;
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, SourceError>;
    async fn get_mod_details(&self, id: &str) -> Result<ModDetails, SourceError>;
    async fn get_files(&self, id: &str) -> Result<Vec<ModFile>, SourceError> {
        Ok(self.get_mod_details(id).await?.files)
    }
    /// Top-level categories, or the children of `parent`.
    async fn categories(&self, parent: Option<i64>) -> Result<Vec<Category>, SourceError>;
    async fn download(
        &self,
        file: &ModFile,
        dest: &Path,
        pause: &AtomicBool,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(), SourceError>;
}
