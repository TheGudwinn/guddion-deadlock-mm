use super::{config::NetConfig, retry::backoff};
use crate::error::NetError;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Hard ceiling for a single download. Mods are far smaller; this only stops a hostile or broken
/// server from filling the disk.
pub const MAX_DOWNLOAD_BYTES: u64 = 16 << 30;

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub struct DownloadRequest<'a> {
    /// Candidate URLs: primary first, then mirrors.
    pub urls: &'a [String],
    pub dest: &'a Path,
    pub expected_sha256: Option<&'a str>,
}

pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

pub async fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut h = Sha256::new();
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

/// Destinations that currently have a download running in this process.
static ACTIVE: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Holds the right to write one `.part` file; two downloads can never interleave into the same file.
struct PartLock(PathBuf);

impl PartLock {
    fn acquire(dest: &Path) -> Option<Self> {
        let mut set = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        set.insert(dest.to_path_buf()).then(|| Self(dest.to_path_buf()))
    }
}

impl Drop for PartLock {
    fn drop(&mut self) {
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// Only plain http(s) URLs are ever requested (no file:, ftp:, data: ...).
fn check_url(u: &str) -> Result<(), NetError> {
    let parsed = url::Url::parse(u).map_err(|e| NetError::InvalidUrl(format!("{u}: {e}")))?;
    match parsed.scheme() {
        "http" | "https" if parsed.host_str().is_some() => Ok(()),
        s => Err(NetError::InvalidUrl(format!("{u}: unsupported scheme or host ({s})"))),
    }
}

/// Download with resume (HTTP Range), retries, mirror fallback and SHA-256 check.
/// Setting `pause` to true stops the transfer and keeps the `.part` file.
///
/// Without `expected_sha256` the caller is responsible for verifying the file (GameBanana only
/// publishes MD5, which `GameBanana::download` checks right after this returns).
pub async fn download(
    client: &reqwest::Client,
    cfg: &NetConfig,
    req: DownloadRequest<'_>,
    pause: &AtomicBool,
    on_progress: impl Fn(Progress),
) -> Result<(), NetError> {
    let part = part_path(req.dest);
    if let Some(dir) = req.dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let _lock = PartLock::acquire(req.dest).ok_or_else(|| {
        NetError::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "this file is already being downloaded"))
    })?;

    let mut last_err = String::from("no urls");
    let mut urls: Vec<&String> = Vec::new();
    for u in req.urls {
        match check_url(u) {
            Ok(()) => urls.push(u),
            Err(e) => last_err = e.to_string(),
        }
    }
    if urls.is_empty() {
        return Err(NetError::AllMirrorsFailed(last_err));
    }

    let mut attempt = 0u32;
    'outer: loop {
        for url in &urls {
            if pause.load(Ordering::Relaxed) {
                return Err(NetError::Paused);
            }
            match try_once(client, url, &part, pause, &on_progress).await {
                Ok(()) => break 'outer,
                Err(NetError::Paused) => return Err(NetError::Paused),
                Err(e) if e.is_retryable() => last_err = format!("{url}: {e}"),
                Err(e) => return Err(e),
            }
        }
        attempt += 1;
        if attempt > cfg.max_retries {
            return Err(NetError::AllMirrorsFailed(last_err));
        }
        tokio::time::sleep(backoff(attempt - 1, cfg.backoff_base_ms, cfg.backoff_max_ms)).await;
    }

    if let Some(expected) = req.expected_sha256 {
        let actual = sha256_file(&part).await?;
        if !actual.eq_ignore_ascii_case(expected) {
            // Corrupt data must not be resumed from again.
            let _ = tokio::fs::remove_file(&part).await;
            return Err(NetError::HashMismatch { expected: expected.to_string(), actual });
        }
    }
    // Everything is on disk before the final name appears.
    // (Windows needs write access on the handle to flush it, so a read-only open would fail.)
    tokio::fs::OpenOptions::new().write(true).open(&part).await?.sync_all().await?;
    tokio::fs::rename(&part, req.dest).await?;
    Ok(())
}

/// Total size from a `Content-Range` header (`bytes 5-9/10` or `bytes */10`).
fn content_range_total(h: &reqwest::header::HeaderMap) -> Option<u64> {
    h.get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?.rsplit('/').next()?.trim().parse().ok()
}

/// First byte of a `Content-Range: bytes START-END/TOTAL` header.
fn content_range_start(h: &reqwest::header::HeaderMap) -> Option<u64> {
    let v = h.get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    v.strip_prefix("bytes ")?.split('-').next()?.trim().parse().ok()
}

async fn discard(part: &Path, why: &str) -> NetError {
    let _ = tokio::fs::remove_file(part).await;
    NetError::BadResponse(why.to_string())
}

async fn try_once(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    pause: &AtomicBool,
    on_progress: &impl Fn(Progress),
) -> Result<(), NetError> {
    let mut have = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
    let mut rq = client.get(url);
    if have > 0 {
        rq = rq.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = rq.send().await?;
    let status = resp.status().as_u16();

    // 416: "range not satisfiable". It means the .part is already complete only if the server says
    // the whole file is exactly as long as what we have; otherwise it is stale, so start over.
    if status == 416 {
        if have > 0 && content_range_total(resp.headers()) == Some(have) {
            return Ok(());
        }
        return Err(discard(part, "stale partial download discarded").await);
    }
    if !(status == 200 || status == 206) {
        return Err(NetError::Status(status));
    }
    // Server ignored Range -> restart from scratch.
    let append = status == 206;
    // A 206 must continue exactly where we stopped; a server that names another offset is not trusted.
    if append && content_range_start(resp.headers()).is_some_and(|start| start != have) {
        return Err(discard(part, "server resumed from the wrong offset").await);
    }
    if !append {
        have = 0;
    }
    let total = resp.content_length().map(|l| l.saturating_add(have));
    if total.is_some_and(|t| t > MAX_DOWNLOAD_BYTES) {
        let _ = tokio::fs::remove_file(part).await;
        return Err(NetError::TooLarge);
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(part)
        .await?;

    let mut stream = resp.bytes_stream();
    let mut downloaded = have;
    while let Some(chunk) = stream.next().await {
        if pause.load(Ordering::Relaxed) {
            file.flush().await?;
            return Err(NetError::Paused);
        }
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                file.flush().await?; // keep what we got for the next resume
                return Err(e.into());
            }
        };
        downloaded += chunk.len() as u64;
        if downloaded > MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(part).await;
            return Err(NetError::TooLarge);
        }
        if total.is_some_and(|t| downloaded > t) {
            drop(file);
            return Err(discard(part, "body is longer than announced").await);
        }
        file.write_all(&chunk).await?;
        on_progress(Progress { downloaded, total });
    }
    file.flush().await?;
    file.sync_all().await?;
    if let Some(t) = total {
        if downloaded < t {
            return Err(NetError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "truncated body")));
        }
    }
    Ok(())
}
