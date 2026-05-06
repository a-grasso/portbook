use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub kind: ProbeKind,
    pub status: Option<u16>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProbeKind {
    /// HTTP 2xx/3xx — live, browsable service.
    Live,
    /// HTTP 4xx/5xx — speaks HTTP but doesn't serve a useful page at /.
    Error,
    /// Did not respond as HTTP — connection refused, timeout, non-HTTP protocol.
    Dead,
}

pub struct Prober {
    client: reqwest::Client,
}

impl Prober {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(1500))
            .redirect(reqwest::redirect::Policy::limited(1))
            .user_agent("portbook/0.1")
            .build()
            .expect("reqwest client");
        Self { client }
    }

    pub async fn probe(&self, port: u16) -> ProbeResult {
        let url = format!("http://127.0.0.1:{port}/");
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                return ProbeResult {
                    kind: ProbeKind::Dead,
                    status: None,
                    title: None,
                    description: None,
                    reason: Some(short_err(&e)),
                };
            }
        };
        let status = resp.status().as_u16();
        let body = resp.bytes().await.unwrap_or_default();
        let take = body.len().min(64 * 1024);
        let html = String::from_utf8_lossy(&body[..take]);
        let (title, description) = extract(&html);
        let kind = if (200..400).contains(&status) {
            ProbeKind::Live
        } else {
            ProbeKind::Error
        };
        let reason = if kind == ProbeKind::Error {
            Some(format!("HTTP {status}"))
        } else {
            None
        };
        ProbeResult { kind, status: Some(status), title, description, reason }
    }
}

fn short_err(e: &reqwest::Error) -> String {
    if e.is_timeout() { return "timeout".into(); }
    if e.is_connect() { return "connection refused".into(); }
    if e.is_decode() || e.is_body() { return "non-HTTP response".into(); }
    "not HTTP".into()
}

fn extract(html: &str) -> (Option<String>, Option<String>) {
    let doc = Html::parse_document(html);
    let title_sel = Selector::parse("title").unwrap();
    let title = doc
        .select(&title_sel)
        .next()
        .map(|n| clean(&n.text().collect::<String>()))
        .filter(|s| !s.is_empty());

    let meta_sel = Selector::parse("meta[name=description], meta[property='og:description']").unwrap();
    let description = doc
        .select(&meta_sel)
        .filter_map(|n| n.value().attr("content"))
        .map(clean)
        .find(|s| !s.is_empty());

    (title, description)
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

