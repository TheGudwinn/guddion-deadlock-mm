//! GameBanana source (public API v11, JSON only; no HTML scraping).
//! Verified against live responses on 2026-11-13: Deadlock game id 20948, `Mod/Index`,
//! `Util/Search/Results`, `Mod/{id}/ProfilePage`, `Mod/Categories`.
//! No rate-limit headers are published, so requests are spaced client-side and 429/5xx are retried.
use super::*;
use crate::error::NetError;
use crate::net::download::{download, DownloadRequest};
use crate::net::NetService;
use md5::{Digest, Md5};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SOURCE_ID: &str = "gamebanana";
pub const DEADLOCK_GAME_ID: i64 = 20948;
const MIN_GAP: Duration = Duration::from_millis(300);

// ---- wire format -------------------------------------------------------------------------------

#[derive(Deserialize)]
struct Meta {
    #[serde(rename = "_nRecordCount", default)]
    count: u64,
    #[serde(rename = "_bIsComplete", default)]
    complete: bool,
}

#[derive(Deserialize)]
struct ListResp {
    #[serde(rename = "_aMetadata")]
    meta: Meta,
    #[serde(rename = "_aRecords", default)]
    records: Vec<Rec>,
}

#[derive(Deserialize)]
struct Name {
    #[serde(rename = "_sName", default)]
    name: String,
}

#[derive(Deserialize)]
struct Img {
    #[serde(rename = "_sBaseUrl", default)]
    base: String,
    #[serde(rename = "_sFile", default)]
    file: String,
    #[serde(rename = "_sFile530")]
    f530: Option<String>,
}

#[derive(Deserialize)]
struct Media {
    #[serde(rename = "_aImages", default)]
    images: Vec<Img>,
}

#[derive(Deserialize)]
struct WireFile {
    #[serde(rename = "_idRow")]
    id: i64,
    #[serde(rename = "_sFile", default)]
    name: String,
    #[serde(rename = "_nFilesize", default)]
    size: u64,
    #[serde(rename = "_sDownloadUrl", default)]
    url: String,
    #[serde(rename = "_sMd5Checksum")]
    md5: Option<String>,
    #[serde(rename = "_sAvResult")]
    av: Option<String>,
    #[serde(rename = "_bIsArchived", default)]
    archived: bool,
}

/// Index/search records and the ProfilePage share these fields; the extras default to empty.
#[derive(Deserialize)]
struct Rec {
    #[serde(rename = "_idRow")]
    id: i64,
    #[serde(rename = "_sName", default)]
    name: String,
    #[serde(rename = "_sProfileUrl", default)]
    url: String,
    #[serde(rename = "_tsDateModified", default)]
    modified: i64,
    #[serde(rename = "_tsDateAdded", default)]
    added: i64,
    #[serde(rename = "_aPreviewMedia")]
    media: Option<Media>,
    #[serde(rename = "_aSubmitter")]
    submitter: Option<Name>,
    #[serde(rename = "_aRootCategory")]
    root_category: Option<Name>,
    #[serde(rename = "_aCategory")]
    category: Option<Name>,
    #[serde(rename = "_sVersion")]
    version: Option<String>,
    #[serde(rename = "_nLikeCount", default)]
    likes: u64,
    #[serde(rename = "_nViewCount", default)]
    views: u64,
    #[serde(rename = "_nDownloadCount")]
    downloads: Option<u64>,
    #[serde(rename = "_sText")]
    text: Option<String>,
    #[serde(rename = "_aFiles", default)]
    files: Vec<WireFile>,
}

#[derive(Deserialize)]
struct WireCat {
    #[serde(rename = "_idRow")]
    id: i64,
    #[serde(rename = "_sName", default)]
    name: String,
    #[serde(rename = "_nItemCount", default)]
    items: u64,
    #[serde(rename = "_nCategoryCount", default)]
    children: u64,
    #[serde(rename = "_bIsObsolete", default)]
    obsolete: bool,
}

// ---- mapping (pure, unit-tested on real fixtures) -----------------------------------------------

fn image_url(i: &Img) -> Option<String> {
    let f = i.f530.as_deref().unwrap_or(&i.file);
    (!i.base.is_empty() && !f.is_empty()).then(|| format!("{}/{}", i.base, f))
}

fn summary(r: &Rec) -> ModSummary {
    ModSummary {
        source: SOURCE_ID.into(),
        id: r.id.to_string(),
        name: r.name.clone(),
        author: r.submitter.as_ref().map(|s| s.name.clone()),
        category: r.root_category.as_ref().or(r.category.as_ref()).map(|c| c.name.clone()),
        thumbnail: r.media.as_ref().and_then(|m| m.images.first()).and_then(image_url),
        version: r.version.clone().filter(|v| !v.is_empty()),
        likes: r.likes,
        views: r.views,
        updated_at: r.modified.max(r.added),
        page_url: r.url.clone(),
    }
}

fn parse_list(body: &[u8], page: u32) -> Result<SearchPage, SourceError> {
    let r: ListResp = serde_json::from_slice(body).map_err(|e| SourceError::Parse(e.to_string()))?;
    Ok(SearchPage { items: r.records.iter().map(summary).collect(), total: r.meta.count, page, has_more: !r.meta.complete, cached_at: None })
}

fn parse_details(body: &[u8]) -> Result<ModDetails, SourceError> {
    let r: Rec = serde_json::from_slice(body).map_err(|e| SourceError::Parse(e.to_string()))?;
    let files = r
        .files
        .iter()
        .filter(|f| !f.archived)
        .map(|f| ModFile {
            id: f.id.to_string(),
            name: f.name.clone(),
            size: f.size,
            url: f.url.clone(),
            md5: f.md5.clone().filter(|m| m.len() == 32),
            av_clean: f.av.as_deref() == Some("clean"),
        })
        .collect();
    let images = r.media.as_ref().map(|m| m.images.iter().filter_map(image_url).collect()).unwrap_or_default();
    Ok(ModDetails { summary: summary(&r), description_html: r.text.clone(), downloads: r.downloads, images, files, cached_at: None })
}

fn parse_categories(body: &[u8]) -> Result<Vec<Category>, SourceError> {
    let v: Vec<WireCat> = serde_json::from_slice(body).map_err(|e| SourceError::Parse(e.to_string()))?;
    Ok(v.into_iter().filter(|c| !c.obsolete).map(|c| Category { id: c.id, name: c.name, item_count: c.items, has_children: c.children > 0 }).collect())
}

fn sort_param(s: Sort) -> Option<&'static str> {
    match s {
        Sort::Default => None,
        Sort::Newest => Some("Generic_Newest"),
        Sort::LatestModified => Some("Generic_LatestModified"),
        Sort::MostLiked => Some("Generic_MostLiked"),
        Sort::MostViewed => Some("Generic_MostViewed"),
        Sort::MostDownloaded => Some("Generic_MostDownloaded"),
    }
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// `https://gamebanana.com/mods/650634` (also with trailing path/query) -> `650634`.
pub fn parse_mod_link(link: &str) -> Option<String> {
    let u = url::Url::parse(link.trim()).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    if host != "gamebanana.com" && host != "www.gamebanana.com" {
        return None;
    }
    let mut seg = u.path_segments()?;
    (seg.next()? == "mods").then_some(())?;
    let id = seg.next()?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

// ---- source -------------------------------------------------------------------------------------

pub struct GameBanana {
    net: Arc<NetService>,
    last: tokio::sync::Mutex<Option<Instant>>,
}

impl GameBanana {
    pub fn new(net: Arc<NetService>) -> Self {
        Self { net, last: tokio::sync::Mutex::new(None) }
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>, SourceError> {
        // Hold the lock while sleeping so concurrent callers are spaced out too.
        let mut last = self.last.lock().await;
        if let Some(t) = *last {
            let el = t.elapsed();
            if el < MIN_GAP {
                tokio::time::sleep(MIN_GAP - el).await;
            }
        }
        *last = Some(Instant::now());
        drop(last);
        self.net.get_bytes(SOURCE_ID, path).await.map_err(|e| match e {
            NetError::Status(404) => SourceError::NotFound,
            e => SourceError::Net(e),
        })
    }

    /// Primary download URL first, then the same path on every configured mirror.
    fn download_urls(&self, primary: &str) -> Vec<String> {
        let mut urls = vec![primary.to_string()];
        if let Ok(u) = url::Url::parse(primary) {
            let tail = u.path().to_string();
            let cfg = self.net.config();
            for base in cfg.mirrors.get(SOURCE_ID).into_iter().flatten() {
                let m = format!("{}{}", base.trim_end_matches('/'), tail);
                if !urls.contains(&m) {
                    urls.push(m);
                }
            }
        }
        urls
    }
}

#[async_trait]
impl ModSource for GameBanana {
    fn id(&self) -> &'static str {
        SOURCE_ID
    }

    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, SourceError> {
        let page = q.page.max(1);
        let per = q.per_page.clamp(1, 50);
        let path = match q.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => format!(
                "apiv11/Util/Search/Results?_sSearchString={}&_idGameRow={DEADLOCK_GAME_ID}&_sModelName=Mod&_nPage={page}&_nPerpage={per}",
                encode(t)
            ),
            None => {
                let mut p = format!("apiv11/Mod/Index?_nPage={page}&_nPerpage={per}&_aFilters%5BGeneric_Game%5D={DEADLOCK_GAME_ID}");
                if let Some(c) = q.category {
                    p.push_str(&format!("&_aFilters%5BGeneric_Category%5D={c}"));
                }
                if let Some(s) = sort_param(q.sort) {
                    p.push_str(&format!("&_sSort={s}"));
                }
                p
            }
        };
        parse_list(&self.get(&path).await?, page)
    }

    async fn get_mod_details(&self, id: &str) -> Result<ModDetails, SourceError> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SourceError::NotFound);
        }
        parse_details(&self.get(&format!("apiv11/Mod/{id}/ProfilePage")).await?)
    }

    async fn categories(&self, parent: Option<i64>) -> Result<Vec<Category>, SourceError> {
        let path = match parent {
            Some(p) => format!("apiv11/Mod/Categories?_idCategoryRow={p}&_sSort=a_to_z"),
            None => format!("apiv11/Mod/Categories?_idGameRow={DEADLOCK_GAME_ID}&_sSort=a_to_z"),
        };
        parse_categories(&self.get(&path).await?)
    }

    async fn download(
        &self,
        file: &ModFile,
        dest: &Path,
        pause: &AtomicBool,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(), SourceError> {
        let urls = self.download_urls(&file.url);
        let cfg = self.net.config();
        let client = self.net.client();
        download(&client, &cfg, DownloadRequest { urls: &urls, dest, expected_sha256: None }, pause, |p| on_progress(p)).await?;

        // GameBanana only publishes MD5; verify it when present and never keep a corrupt file.
        if let Some(expected) = &file.md5 {
            let actual = md5_file(dest).await.map_err(NetError::from)?;
            if !actual.eq_ignore_ascii_case(expected) {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(NetError::HashMismatch { expected: expected.clone(), actual }.into());
            }
        }
        Ok(())
    }
}

async fn md5_file(path: &Path) -> std::io::Result<String> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(path).await?;
    let mut h = Md5::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_index_response() {
        let p = parse_list(include_bytes!("../../tests/fixtures/gb_index.json"), 1).unwrap();
        assert_eq!(p.items.len(), 3);
        assert!(p.total > 1000 && p.has_more);
        let m = &p.items[0];
        assert_eq!(m.source, "gamebanana");
        assert!(m.id.chars().all(|c| c.is_ascii_digit()) && !m.name.is_empty());
        assert!(m.thumbnail.as_deref().is_some_and(|t| t.starts_with("https://images.gamebanana.com/")));
        assert!(m.category.is_some() && m.author.is_some());
    }

    #[test]
    fn parses_real_search_response() {
        let p = parse_list(include_bytes!("../../tests/fixtures/gb_search.json"), 1).unwrap();
        assert!(!p.items.is_empty());
    }

    #[test]
    fn parses_real_profile_with_files_and_md5() {
        let d = parse_details(include_bytes!("../../tests/fixtures/gb_profile.json")).unwrap();
        assert!(!d.files.is_empty());
        let f = &d.files[0];
        assert!(f.url.starts_with("https://gamebanana.com/dl/"));
        assert_eq!(f.md5.as_deref().map(str::len), Some(32));
        assert!(f.av_clean && f.size > 0);
        assert!(d.summary.updated_at > 0);
    }

    #[test]
    fn parses_real_categories() {
        let c = parse_categories(include_bytes!("../../tests/fixtures/gb_categories.json")).unwrap();
        assert!(c.iter().any(|c| c.name == "HUD" && c.has_children));
    }

    #[test]
    fn malformed_json_is_a_parse_error_not_a_panic() {
        assert!(matches!(parse_list(b"<html>blocked</html>", 1), Err(SourceError::Parse(_))));
    }

    #[test]
    fn recognises_gamebanana_links() {
        assert_eq!(parse_mod_link("https://gamebanana.com/mods/650634"), Some("650634".into()));
        assert_eq!(parse_mod_link("https://www.gamebanana.com/mods/650634?x=1#c"), Some("650634".into()));
        assert_eq!(parse_mod_link("https://gamebanana.com/mods/cats/46154"), None);
        assert_eq!(parse_mod_link("https://evil.example/mods/1"), None);
        assert_eq!(parse_mod_link("https://gamebanana.com.evil.example/mods/1"), None);
        assert_eq!(parse_mod_link("not a url"), None);
    }
}
