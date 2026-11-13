use deadlock_mm_lib::net::{
    client::build_client,
    config::{NetConfig, ProxyConfig, ProxyKind, ProxyMode},
    diagnose::{diagnose, StepState, Verdict},
};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn direct() -> reqwest::Client {
    build_client(&NetConfig { proxy: ProxyConfig { mode: ProxyMode::None, ..Default::default() }, ..Default::default() }).unwrap()
}

/// A loopback port nothing listens on.
fn closed_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn state_of(r: &deadlock_mm_lib::net::diagnose::HostReport, name: &str) -> StepState {
    r.steps.iter().find(|s| s.name == name).unwrap().state
}

#[tokio::test]
async fn healthy_host_passes_every_step() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
    let r = diagnose(&direct(), &server.uri(), false).await;
    assert_eq!(r.verdict, Verdict::Ok);
    for s in ["dns", "tcp", "http"] {
        assert_eq!(state_of(&r, s), StepState::Ok, "{s}");
    }
}

#[tokio::test]
async fn client_error_status_still_proves_reachability_but_5xx_does_not() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(403)).mount(&server).await;
    assert_eq!(diagnose(&direct(), &server.uri(), false).await.verdict, Verdict::Ok);

    let bad = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&bad).await;
    assert_eq!(diagnose(&direct(), &bad.uri(), false).await.verdict, Verdict::HttpFailed);
}

#[tokio::test]
async fn refused_connection_is_reported_at_the_tcp_step() {
    let r = diagnose(&direct(), &format!("http://127.0.0.1:{}", closed_port()), false).await;
    assert_eq!(r.verdict, Verdict::TcpFailed);
    assert_eq!(state_of(&r, "dns"), StepState::Ok);
    assert_eq!(state_of(&r, "tcp"), StepState::Failed);
    assert!(r.steps.iter().all(|s| s.name != "http"), "later steps must not run after a failure");
}

#[tokio::test]
async fn unresolvable_name_is_reported_at_the_dns_step() {
    let r = diagnose(&direct(), "https://no-such-host.invalid", false).await;
    assert_eq!(r.verdict, Verdict::DnsFailed);
    assert_eq!(r.steps.len(), 1);
}

#[tokio::test]
async fn dead_proxy_skips_direct_steps_and_blames_the_proxy() {
    let cfg = NetConfig {
        proxy: ProxyConfig { mode: ProxyMode::Custom, kind: ProxyKind::Socks5, host: "127.0.0.1".into(), port: closed_port(), ..Default::default() },
        ..Default::default()
    };
    let client = build_client(&cfg).unwrap();
    let r = diagnose(&client, "https://example.com", true).await;
    assert_eq!(r.verdict, Verdict::ProxyFailed);
    assert_eq!(state_of(&r, "dns"), StepState::Skipped);
    assert_eq!(state_of(&r, "tcp"), StepState::Skipped);
    assert_eq!(state_of(&r, "http"), StepState::Failed);
}

#[tokio::test]
async fn garbage_url_is_rejected_without_network_access() {
    assert_eq!(diagnose(&direct(), "not a url", false).await.verdict, Verdict::BadUrl);
}

#[tokio::test]
async fn untrusted_certificate_is_reported_as_a_tls_problem() {
    // A plain-HTTP server answering a TLS handshake is the closest local stand-in for a broken TLS path.
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
    let url = server.uri().replace("http://", "https://");
    let r = diagnose(&direct(), &url, false).await;
    assert_eq!(state_of(&r, "tcp"), StepState::Ok);
    assert_eq!(r.verdict, Verdict::TlsFailed, "{:?}", r.steps);
}
