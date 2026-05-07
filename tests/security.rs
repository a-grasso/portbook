//! Security spec for portbook.
//!
//! Each test names a *property* the system promises to uphold, expressed
//! against the real router that `main()` builds. If a future refactor
//! breaks one of these, the failing test name tells you which security
//! guarantee regressed.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use portbook::build_app;
use portbook::probe::{ProbeKind, ProbeResult};
use portbook::process::ProcInfo;
use portbook::state::{AppState, PortCard};
use portbook::VersionState;
use std::collections::HashMap;
use std::time::Duration;
use tower::ServiceExt;

// ───────────────────────────────────────────────────────────────────────
// Property 1: only loopback Host headers reach handlers (DNS rebinding).
// ───────────────────────────────────────────────────────────────────────

const ALLOWED_HOSTS: &[&str] = &["127.0.0.1:7777", "localhost:7777", "[::1]:7777"];

const ATTACKER_HOSTS: &[&str] = &[
    "evil.example.com",
    "evil.example.com:7777",
    "rebind.attacker.test",
    "127.0.0.1.nip.io:7777", // a domain that resolves to 127.0.0.1
    "localhost",             // no port
    "127.0.0.1",             // no port
    "127.0.0.1:8080",        // wrong port
    "",                      // empty
];

async fn status_for(host: Option<&str>, path: &str) -> StatusCode {
    let app = build_app(AppState::new(), VersionState::new());
    let mut b = Request::builder().uri(path);
    if let Some(h) = host {
        b = b.header(header::HOST, h);
    }
    app.oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn host_allowlist_admits_loopback() {
    for host in ALLOWED_HOSTS {
        assert_eq!(
            status_for(Some(host), "/api/ports").await,
            StatusCode::OK,
            "host {host:?} should be allowed"
        );
    }
}

#[tokio::test]
async fn host_allowlist_rejects_attacker_domains() {
    for host in ATTACKER_HOSTS {
        assert_eq!(
            status_for(Some(host), "/api/ports").await,
            StatusCode::FORBIDDEN,
            "attacker host {host:?} must be rejected"
        );
    }
}

#[tokio::test]
async fn host_allowlist_rejects_missing_header() {
    assert_eq!(status_for(None, "/api/ports").await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn host_allowlist_covers_every_egress_path() {
    // The guard must protect static assets and the SSE stream too —
    // not just the JSON poll endpoint.
    for path in ["/", "/style.css", "/app.js", "/api/stream"] {
        assert_eq!(
            status_for(Some("evil.example.com"), path).await,
            StatusCode::FORBIDDEN,
            "path {path:?} reachable from attacker host"
        );
    }
}

// ───────────────────────────────────────────────────────────────────────
// Property 2: secret values bound to known secret-key flags never appear
// in any API response body.
//
// We plant a card whose cmdline carries five distinct, recognisable
// sentinel strings, then assert none of them survive the trip to the
// wire — for both the JSON poll endpoint and the SSE stream.
// ───────────────────────────────────────────────────────────────────────

const SECRETS: [&str; 5] = [
    "supersecret123",
    "hunter2pw",
    "ghp_ABC123XYZ",
    "AKIA-FAKE-KEY",
    "rebind-bypass-token",
];

fn planted_cmdline() -> String {
    format!(
        "myapp --token={t0} TOKEN={t1} -p {t2} \
         DATABASE_URL=postgres://u:{t3}@h/db https://user:{t4}@host/x",
        t0 = SECRETS[0],
        t1 = SECRETS[1],
        t2 = SECRETS[2],
        t3 = SECRETS[3],
        t4 = SECRETS[4],
    )
}

async fn state_with_planted_card(port: u16) -> AppState {
    let state = AppState::new();
    let proc = ProcInfo {
        cwd: Some("/tmp/portbook-test".into()),
        cmdline: Some(planted_cmdline()),
    };
    let probe = ProbeResult {
        kind: ProbeKind::Live,
        status: Some(200),
        title: None,
        description: None,
        reason: None,
        probed_url: format!("http://127.0.0.1:{port}/"),
        probed_at_unix: 0,
        elapsed_ms: 0,
        error_class: None,
        error_detail: None,
        attempts: 1,
    };
    let card = PortCard::build(port, 999, "myapp".into(), &proc, &probe);
    let mut map = HashMap::new();
    map.insert(port, card);
    state.replace(map, None).await;
    state
}

fn assert_no_secrets(label: &str, body: &str) {
    // Sanity: planted card must actually be in the response, otherwise
    // "no secret present" would be trivially true.
    assert!(body.contains("18080"), "{label}: planted card missing — {body}");
    for secret in &SECRETS {
        assert!(
            !body.contains(secret),
            "{label}: secret {secret:?} leaked: {body}"
        );
    }
}

#[tokio::test]
async fn api_ports_never_emits_secret_values() {
    let state = state_with_planted_card(18080).await;
    let app = build_app(state, VersionState::new());
    let req = Request::builder()
        .uri("/api/ports")
        .header(header::HOST, "127.0.0.1:7777")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    assert_no_secrets("/api/ports", std::str::from_utf8(&body).unwrap());
}

#[tokio::test]
async fn api_stream_never_emits_secret_values() {
    let state = state_with_planted_card(18080).await;
    let app = build_app(state, VersionState::new());
    let req = Request::builder()
        .uri("/api/stream")
        .header(header::HOST, "127.0.0.1:7777")
        .body(Body::empty())
        .unwrap();
    let res = tokio::time::timeout(Duration::from_secs(2), app.oneshot(req))
        .await
        .expect("stream did not respond")
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Pull frames until we have the initial snapshot event (terminates
    // with "\n\n" per SSE framing) or the cap fires.
    let mut body = res.into_body();
    let mut buf: Vec<u8> = Vec::new();
    let read = async {
        while buf.len() < 32 * 1024 {
            match body.frame().await {
                Some(Ok(frame)) => {
                    if let Some(data) = frame.data_ref() {
                        buf.extend_from_slice(data);
                        if buf.windows(2).any(|w| w == b"\n\n") {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), read).await;
    assert_no_secrets("/api/stream", &String::from_utf8_lossy(&buf));
}
