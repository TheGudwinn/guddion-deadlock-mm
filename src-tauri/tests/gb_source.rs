use deadlock_mm_lib::error::NetError;
use deadlock_mm_lib::net::config::{NetConfig, ProxyConfig, ProxyMode};
use deadlock_mm_lib::net::NetService;
use deadlock_mm_lib::sources::gamebanana::GameBanana;
use deadlock_mm_lib::sources::{ModFile, ModSource, SearchQuery, Sort, SourceError};
use md5::{Digest, Md5};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn source_for(srv: &MockServer) -> GameBanana {
    let mut cfg = NetConfig {
        proxy: ProxyConfig { mode: ProxyMode::None, ..Default::default() },
        max_retries: 2,
        backoff_base_ms: 5,
        backoff_max_ms: 20,
        ..Default::default()
    };
    cfg.mirrors.insert("gamebanana".into(), vec![srv.uri()]);
    GameBanana::new(Arc::new(NetService::new(cfg).unwrap()))
}

#[tokio::test]
async fn list_query_filters_by_deadlock_category_and_sort() {
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apiv11/Mod/Index"))
        .and(query_param("_aFilters[Generic_Game]", "20948"))
        .and(query_param("_aFilters[Generic_Category]", "31713"))
        .and(query_param("_sSort", "Generic_Newest"))
        .and(query_param("_nPerpage", "50")) // clamped from 500
        .respond_with(ResponseTemplate::new(200).set_body_bytes(include_bytes!("fixtures/gb_index.json").to_vec()))
        .expect(1)
        .mount(&srv)
        .await;
    let gb = source_for(&srv);
    let page = gb
        .search(&SearchQuery { category: Some(31713), sort: Sort::Newest, page: 1, per_page: 500, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 3);
}

#[tokio::test]
async fn text_search_uses_search_endpoint_and_escapes_input() {
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/apiv11/Util/Search/Results"))
        .and(query_param("_sSearchString", "hud & más"))
        .and(query_param("_idGameRow", "20948"))
        .and(query_param("_sModelName", "Mod"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(include_bytes!("fixtures/gb_search.json").to_vec()))
        .expect(1)
        .mount(&srv)
        .await;
    let gb = source_for(&srv);
    let page = gb.search(&SearchQuery { text: Some("hud & más".into()), page: 1, per_page: 20, ..Default::default() }).await.unwrap();
    assert!(!page.items.is_empty());
}

#[tokio::test]
async fn details_404_is_not_found_and_non_numeric_id_never_hits_network() {
    let srv = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(404)).mount(&srv).await;
    let gb = source_for(&srv);
    assert!(matches!(gb.get_mod_details("123").await, Err(SourceError::NotFound)));
    assert!(matches!(gb.get_mod_details("../../etc").await, Err(SourceError::NotFound)));
    assert_eq!(srv.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn html_block_page_instead_of_json_gives_parse_error() {
    let srv = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_string("<html>blocked by provider</html>")).mount(&srv).await;
    let gb = source_for(&srv);
    assert!(matches!(gb.categories(None).await, Err(SourceError::Parse(_))));
}

fn file_for(srv: &MockServer, body: &[u8], md5: &str) -> ModFile {
    let _ = body;
    ModFile { id: "1".into(), name: "m.zip".into(), size: body.len() as u64, url: format!("{}/dl/1", srv.uri()), md5: Some(md5.into()), av_clean: true }
}

#[tokio::test]
async fn download_verifies_md5() {
    let srv = MockServer::start().await;
    let body = b"mod archive bytes".to_vec();
    Mock::given(method("GET")).and(path("/dl/1")).respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone())).mount(&srv).await;
    let gb = source_for(&srv);
    let dir = tempfile::tempdir().unwrap();
    let good = hex::encode(Md5::digest(&body));

    let dest = dir.path().join("ok.zip");
    gb.download(&file_for(&srv, &body, &good), &dest, &AtomicBool::new(false), &|_| {}).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);

    let bad = dir.path().join("bad.zip");
    let err = gb.download(&file_for(&srv, &body, &"0".repeat(32)), &bad, &AtomicBool::new(false), &|_| {}).await.unwrap_err();
    assert!(matches!(err, SourceError::Net(NetError::HashMismatch { .. })));
    assert!(!bad.exists(), "corrupt download must not be kept");
}

#[tokio::test]
async fn download_falls_back_to_mirror_when_primary_is_down() {
    let dead = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&dead).await;
    let mirror = MockServer::start().await;
    let body = b"from mirror".to_vec();
    Mock::given(method("GET")).and(path("/dl/1")).respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone())).mount(&mirror).await;

    let mut cfg = NetConfig { proxy: ProxyConfig { mode: ProxyMode::None, ..Default::default() }, max_retries: 2, backoff_base_ms: 5, backoff_max_ms: 20, ..Default::default() };
    cfg.mirrors.insert("gamebanana".into(), vec![mirror.uri()]);
    let gb = GameBanana::new(Arc::new(NetService::new(cfg).unwrap()));
    let f = ModFile { id: "1".into(), name: "m.zip".into(), size: 0, url: format!("{}/dl/1", dead.uri()), md5: None, av_clean: true };
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("m.zip");
    gb.download(&f, &dest, &AtomicBool::new(false), &|_| {}).await.unwrap();
    assert_eq!(std::fs::read(dest).unwrap(), body);
}

/// Hits the real GameBanana. Run manually: `cargo test --test gb_source live -- --ignored`
#[tokio::test]
#[ignore]
async fn live_smoke() {
    let gb = GameBanana::new(Arc::new(NetService::new(NetConfig::default()).unwrap()));
    let page = gb.search(&SearchQuery { sort: Sort::MostLiked, page: 1, per_page: 5, ..Default::default() }).await.unwrap();
    assert_eq!(page.items.len(), 5);
    let d = gb.get_mod_details(&page.items[0].id).await.unwrap();
    assert!(!d.files.is_empty());
    assert!(gb.categories(None).await.unwrap().iter().any(|c| c.name == "HUD"));
}
