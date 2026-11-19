pub mod client;
pub mod config;
pub mod diagnose;
pub mod download;
pub mod retry;

use crate::error::NetError;
use config::NetConfig;
use std::sync::RwLock;

/// Entry point for all network access. Rebuilds the client when settings change.
pub struct NetService {
    cfg: RwLock<NetConfig>,
    client: RwLock<reqwest::Client>,
}

impl NetService {
    pub fn new(cfg: NetConfig) -> Result<Self, NetError> {
        let client = client::build_client(&cfg)?;
        Ok(Self { cfg: RwLock::new(cfg), client: RwLock::new(client) })
    }

    pub fn update_config(&self, cfg: NetConfig) -> Result<(), NetError> {
        let c = client::build_client(&cfg)?; // validate before swapping
        *self.cfg.write().unwrap() = cfg;
        *self.client.write().unwrap() = c;
        Ok(())
    }

    pub fn config(&self) -> NetConfig {
        self.cfg.read().unwrap().clone()
    }

    pub fn client(&self) -> reqwest::Client {
        self.client.read().unwrap().clone()
    }

    /// GET `path` against each mirror of `source` in order, with retries and backoff.
    pub async fn get_bytes(&self, source: &str, path: &str) -> Result<Vec<u8>, NetError> {
        let cfg = self.config();
        let client = self.client();
        let bases = cfg.mirrors.get(source).cloned().unwrap_or_default();
        let mut last = format!("no mirrors configured for '{source}'");
        for attempt in 0..=cfg.max_retries {
            for base in &bases {
                let url = format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'));
                let res = async {
                    let r = client.get(&url).send().await?;
                    let s = r.status().as_u16();
                    if s != 200 {
                        return Err(NetError::Status(s));
                    }
                    Ok(r.bytes().await?.to_vec())
                }
                .await;
                match res {
                    Ok(b) => return Ok(b),
                    Err(e) if e.is_retryable() => last = format!("{url}: {e}"),
                    Err(e) => return Err(e),
                }
            }
            if attempt < cfg.max_retries {
                tokio::time::sleep(retry::backoff(attempt, cfg.backoff_base_ms, cfg.backoff_max_ms)).await;
            }
        }
        Err(NetError::AllMirrorsFailed(last))
    }
}
