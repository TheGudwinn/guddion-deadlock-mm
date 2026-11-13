use deadlock_mm_lib::error::NetError;
use deadlock_mm_lib::net::{
    client::build_client,
    config::{NetConfig, ProxyConfig, ProxyMode},
    download::{download, DownloadRequest},
    NetService,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn fast_cfg() -> NetConfig {
    NetConfig {
        proxy: ProxyConfig { mode: ProxyMode::None, ..Default::default() },
        max_retries: 3,
        backoff_base_ms: 5,
        backoff_max_ms: 20,
        connect_timeout_secs: 2,
        read_timeout_secs: 2,
        ..Default::default()
    }
}

fn data() -> Vec<u8> {
    (0..100_000u32).map(|i| (i % 251) as u8).collect()
}

/// Serves `data`, honouring Range. The first request is cut short to simulate a dropped connection.
struct Flaky {
    data: Vec<u8>,
    hits: Arc<AtomicUsize>,
}
impl Respond for Flaky {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let n = self.hits.fetch_add(1, Ordering::SeqCst);
        let start = req
            .headers
            .get("range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes="))
            .and_then(|v| v.trim_end_matches('-').parse::<usize>().ok())
            .unwrap_or(0);
        let body = &self.data[start..];
        if n == 0 {
            // Promise the full length but we cannot truncate in wiremock; send 500 instead (transient failure).
            return ResponseTemplate::new(503);
        }
        if start > 0 {
            ResponseTemplate::new(206).set_body_bytes(body.to_vec())
        } else {
            ResponseTemplate::new(200).set_body_bytes(body.to_vec())
        }
    }
}

#[tokio::test]
async fn retries_after_transient_error_and_verifies_hash() {
    let srv = MockServer::start().await;
    let d = data();
    let hits = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET")).and(path("/f.vpk")).respond_with(Flaky { data: d.clone(), hits: hits.clone() }).mount(&srv).await;

    let cfg = fast_cfg();
    let client = build_client(&cfg).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.vpk");
    let urls = vec![format!("{}/f.vpk", srv.uri())];
    let sha = hex::encode(Sha256::digest(&d));
    let pause = AtomicBool::new(false);

    download(&client, &cfg, DownloadRequest { urls: &urls, dest: &dest, expected_sha256: Some(&sha) }, &pause, |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), d);
    assert!(hits.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn resumes_from_partial_file_with_range() {
    let srv = MockServer::start().await;
    let d = data();
    // hits starts at 1 so the Flaky responder serves data right away.
    let hits = Arc::new(AtomicUsize::new(1));
    Mock::given(method("GET")).and(path("/f.vpk")).respond_with(Flaky { data: d.clone(), hits }).mount(&srv).await;

    let cfg = fast_cfg();
    let client = build_client(&cfg).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.vpk");
    std::fs::write(dir.path().join("f.vpk.part"), &d[..40_000]).unwrap();
    let urls = vec![format!("{}/f.vpk", srv.uri())];
    let sha = hex::encode(Sha256::digest(&d));

    download(&client, &cfg, DownloadRequest { urls: &urls, dest: &dest, expected_sha256: Some(&sha) }, &AtomicBool::new(false), |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), d);
}

#[tokio::test]
async fn falls_back_to_mirror() {
    let dead = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(500)).mount(&dead).await;
    let good = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api")).respond_with(ResponseTemplate::new(200).set_body_string("ok")).mount(&good).await;

    let mut cfg = fast_cfg();
    cfg.mirrors.insert("gb".into(), vec![dead.uri(), good.uri()]);
    let svc = NetService::new(cfg).unwrap();
    assert_eq!(svc.get_bytes("gb", "api").await.unwrap(), b"ok");
}

#[tokio::test]
async fn all_mirrors_down_gives_clear_error() {
    let dead = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&dead).await;
    let mut cfg = fast_cfg();
    cfg.mirrors.insert("gb".into(), vec![dead.uri()]);
    let svc = NetService::new(cfg).unwrap();
    let err = svc.get_bytes("gb", "x").await.unwrap_err();
    assert!(matches!(err, NetError::AllMirrorsFailed(_)));
    assert_eq!(err.hint(), "check_connection_proxy_or_mirror");
}

#[tokio::test]
async fn hash_mismatch_is_rejected_and_part_removed() {
    let srv = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello".to_vec())).mount(&srv).await;
    let cfg = fast_cfg();
    let client = build_client(&cfg).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("x.bin");
    let urls = vec![srv.uri()];
    let err = download(&client, &cfg, DownloadRequest { urls: &urls, dest: &dest, expected_sha256: Some("00") }, &AtomicBool::new(false), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::HashMismatch { .. }));
    assert!(!dest.exists());
    assert!(!dir.path().join("x.bin.part").exists());
}

#[tokio::test]
async fn pause_keeps_part_file() {
    let srv = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_bytes(data())).mount(&srv).await;
    let cfg = fast_cfg();
    let client = build_client(&cfg).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("p.bin");
    let urls = vec![srv.uri()];
    let pause = AtomicBool::new(true);
    let err = download(&client, &cfg, DownloadRequest { urls: &urls, dest: &dest, expected_sha256: None }, &pause, |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::Paused));
    assert!(!dest.exists());
}

#[test]
fn custom_proxy_requires_host_and_port() {
    let mut cfg = fast_cfg();
    cfg.proxy.mode = ProxyMode::Custom;
    assert!(build_client(&cfg).is_err());
    cfg.proxy.host = "127.0.0.1".into();
    cfg.proxy.port = 1080;
    cfg.proxy.kind = deadlock_mm_lib::net::config::ProxyKind::Socks5;
    assert!(build_client(&cfg).is_ok());
}
