//! Saved settings must change what the live network client actually does.
use deadlock_mm_lib::net::{config::NetConfig, NetService};
use deadlock_mm_lib::settings::{AppSettings, ProxyInput, SettingsInput};
use deadlock_mm_lib::net::config::{ProxyKind, ProxyMode};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn input(mirrors: Vec<String>) -> SettingsInput {
    SettingsInput {
        proxy: ProxyInput { mode: ProxyMode::None, kind: ProxyKind::Http, host: String::new(), port: 0, username: None, password: None },
        mirrors,
        game_path: None,
        check_updates: true,
        update_endpoints: vec![],
    }
}

#[tokio::test]
async fn new_mirror_list_is_used_without_restart() {
    let dead = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(404)).mount(&dead).await;
    let live = MockServer::start().await;
    Mock::given(method("GET")).and(path("/x")).respond_with(ResponseTemplate::new(200).set_body_string("ok")).mount(&live).await;

    let mut base = NetConfig::default();
    base.max_retries = 0;
    let svc = NetService::new(base.clone()).unwrap();
    let mut settings = AppSettings { net: base, ..Default::default() };

    // Primary answers 404 (not retryable) -> request fails.
    settings = settings.with_input(input(vec![dead.uri()])).unwrap();
    svc.update_config(settings.net.clone()).unwrap();
    assert!(svc.get_bytes("gamebanana", "/x").await.is_err());

    // Switch the mirror list at runtime; the same service now succeeds.
    settings = settings.with_input(input(vec![live.uri()])).unwrap();
    svc.update_config(settings.net.clone()).unwrap();
    assert_eq!(svc.get_bytes("gamebanana", "/x").await.unwrap(), b"ok");
}
