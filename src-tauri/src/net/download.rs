use super::{config::NetConfig, retry::backoff};
use crate::error::NetError;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

/// Download with resume (HTTP Range), retries, mirror fallback and SHA-256 check.
/// Setting `pause` to true stops the transfer and keeps the `.part` file.
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
    let mut last_err = String::from("no urls");
    let mut attempt = 0u32;

    'outer: loop {
        for url in req.urls {
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
    tokio::fs::rename(&part, req.dest).await?;
    Ok(())
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

    // 416: our .part is already the full file (or stale); treat full file as done.
    if status == 416 {
        return Ok(());
    }
    if !(status == 200 || status == 206) {
        return Err(NetError::Status(status));
    }
    // Server ignored Range -> restart from scratch.
    let append = status == 206;
    if !append {
        have = 0;
    }
    let total = resp.content_length().map(|l| l + have);

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
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        on_progress(Progress { downloaded, total });
    }
    file.flush().await?;
    if let Some(t) = total {
        if downloaded < t {
            return Err(NetError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "truncated body")));
        }
    }
    Ok(())
}
