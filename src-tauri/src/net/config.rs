use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// Use system proxy settings (default reqwest behaviour).
    System,
    /// Direct connection, ignore any system proxy.
    None,
    /// Use the explicit proxy below.
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Http,
    Https,
    Socks5,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self { mode: ProxyMode::System, kind: ProxyKind::Http, host: String::new(), port: 0, username: None, password: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetConfig {
    pub proxy: ProxyConfig,
    pub connect_timeout_secs: u64,
    pub read_timeout_secs: u64,
    pub max_retries: u32,
    pub backoff_base_ms: u64,
    pub backoff_max_ms: u64,
    /// source id -> ordered list of base URLs (primary first, then mirrors).
    /// Editable by hand; may be reloaded from a file.
    pub mirrors: HashMap<String, Vec<String>>,
}

impl Default for NetConfig {
    fn default() -> Self {
        let mut mirrors = HashMap::new();
        mirrors.insert("gamebanana".to_string(), vec!["https://gamebanana.com".to_string()]);
        Self {
            proxy: ProxyConfig::default(),
            connect_timeout_secs: 10,
            read_timeout_secs: 30,
            max_retries: 5,
            backoff_base_ms: 500,
            backoff_max_ms: 30_000,
            mirrors,
        }
    }
}

impl NetConfig {
    pub fn from_json_file(path: &std::path::Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}
