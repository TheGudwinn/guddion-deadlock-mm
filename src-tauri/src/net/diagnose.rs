//! Step-by-step reachability check (DNS -> TCP -> TLS/HTTP) so the UI can say *where* it breaks.
//!
//! With an explicit proxy the machine never resolves or connects to the target itself,
//! so DNS and TCP are reported as skipped and only the request through the proxy counts.
use serde::Serialize;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    Ok,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub name: &'static str,
    pub state: StepState,
    pub ms: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    DnsFailed,
    TcpFailed,
    TlsFailed,
    ProxyFailed,
    HttpFailed,
    BadUrl,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostReport {
    pub url: String,
    pub host: String,
    pub steps: Vec<Step>,
    pub verdict: Verdict,
}

const STEP_TIMEOUT: Duration = Duration::from_secs(8);

fn step(name: &'static str, state: StepState, started: Option<Instant>, detail: Option<String>) -> Step {
    Step { name, state, ms: started.map(|t| t.elapsed().as_millis() as u64), detail }
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

fn looks_like_tls(text: &str) -> bool {
    let t = text.to_lowercase();
    ["certificate", "handshake", "tls", "ssl", "invalid peer"].iter().any(|k| t.contains(k))
}

pub async fn diagnose(client: &reqwest::Client, url: &str, via_proxy: bool) -> HostReport {
    let parsed = match url::Url::parse(url) {
        Ok(u) if u.host_str().is_some() => u,
        _ => {
            return HostReport {
                url: url.to_string(),
                host: String::new(),
                steps: vec![step("url", StepState::Failed, None, Some(url.to_string()))],
                verdict: Verdict::BadUrl,
            }
        }
    };
    let host = parsed.host_str().unwrap_or_default().to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);
    let mut steps = Vec::new();

    if via_proxy {
        steps.push(step("dns", StepState::Skipped, None, None));
        steps.push(step("tcp", StepState::Skipped, None, None));
    } else {
        // DNS
        let t = Instant::now();
        let addrs: Vec<SocketAddr> = match tokio::time::timeout(STEP_TIMEOUT, tokio::net::lookup_host((host.clone(), port))).await {
            Ok(Ok(it)) => it.collect(),
            Ok(Err(e)) => {
                steps.push(step("dns", StepState::Failed, Some(t), Some(e.to_string())));
                return finish(url, host, steps, Verdict::DnsFailed);
            }
            Err(_) => {
                steps.push(step("dns", StepState::Failed, Some(t), Some("timeout".into())));
                return finish(url, host, steps, Verdict::DnsFailed);
            }
        };
        if addrs.is_empty() {
            steps.push(step("dns", StepState::Failed, Some(t), Some("no addresses".into())));
            return finish(url, host, steps, Verdict::DnsFailed);
        }
        let shown = addrs.iter().take(3).map(|a| a.ip().to_string()).collect::<Vec<_>>().join(", ");
        steps.push(step("dns", StepState::Ok, Some(t), Some(shown)));

        // TCP: first address that accepts a connection wins.
        let t = Instant::now();
        let mut last = String::from("timeout");
        let mut connected = None;
        for a in addrs.iter().take(4) {
            match tokio::time::timeout(STEP_TIMEOUT, tokio::net::TcpStream::connect(a)).await {
                Ok(Ok(_)) => {
                    connected = Some(*a);
                    break;
                }
                Ok(Err(e)) => last = e.to_string(),
                Err(_) => last = "timeout".into(),
            }
        }
        match connected {
            Some(a) => steps.push(step("tcp", StepState::Ok, Some(t), Some(a.to_string()))),
            None => {
                steps.push(step("tcp", StepState::Failed, Some(t), Some(last)));
                return finish(url, host, steps, Verdict::TcpFailed);
            }
        }
    }

    // TLS + HTTP through the real client (so proxy and certificate settings are the ones in use).
    let t = Instant::now();
    match tokio::time::timeout(STEP_TIMEOUT, client.get(url).send()).await {
        Ok(Ok(resp)) => {
            let status = resp.status();
            // Any HTTP answer proves the path works; 5xx means the server itself is unhealthy.
            let ok = !status.is_server_error();
            steps.push(step("http", if ok { StepState::Ok } else { StepState::Failed }, Some(t), Some(status.as_u16().to_string())));
            finish(url, host, steps, if ok { Verdict::Ok } else { Verdict::HttpFailed })
        }
        Ok(Err(e)) => {
            let text = chain(&e);
            // TLS error texts are OS-localised (schannel), so keywords alone are not enough:
            // when our own TCP step already connected, a connect error on https must be the handshake.
            let handshake = !via_proxy && e.is_connect() && parsed.scheme() == "https";
            let verdict = if via_proxy && e.is_connect() && !looks_like_tls(&text) {
                Verdict::ProxyFailed
            } else if handshake || looks_like_tls(&text) {
                Verdict::TlsFailed
            } else {
                Verdict::HttpFailed
            };
            steps.push(step("http", StepState::Failed, Some(t), Some(text)));
            finish(url, host, steps, verdict)
        }
        Err(_) => {
            steps.push(step("http", StepState::Failed, Some(t), Some("timeout".into())));
            finish(url, host, steps, if via_proxy { Verdict::ProxyFailed } else { Verdict::HttpFailed })
        }
    }
}

fn finish(url: &str, host: String, steps: Vec<Step>, verdict: Verdict) -> HostReport {
    HostReport { url: url.to_string(), host, steps, verdict }
}

/// Checks all targets concurrently.
pub async fn diagnose_all(client: &reqwest::Client, urls: &[String], via_proxy: bool) -> Vec<HostReport> {
    futures_util::future::join_all(urls.iter().map(|u| diagnose(client, u, via_proxy))).await
}
