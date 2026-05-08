use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub kind: ProbeKind,
    pub status: Option<u16>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub reason: Option<String>,
    /// URL the prober actually requested.
    pub probed_url: String,
    /// Unix seconds at the moment the probe started.
    pub probed_at_unix: u64,
    /// Wall time from request start to response (or error).
    pub elapsed_ms: u32,
    /// Coarse classification of the underlying transport error, if any.
    pub error_class: Option<ProbeError>,
    /// Truncated `Display` of the underlying error — for diagnostics paste.
    pub error_detail: Option<String>,
    /// Number of probe attempts (currently always 1; reserved for future retry).
    pub attempts: u8,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProbeError {
    Timeout,
    Connect,
    Decode,
    Body,
    /// Server responded with redirects exceeding our follow limit. The
    /// host clearly speaks HTTP — surfaced as `Error` (not `Dead`) so
    /// users see "redirect chain" instead of "not HTTP".
    Redirect,
    Other,
}

pub struct Prober {
    client: reqwest::Client,
}

/// Per-attempt request timeout. Tuned to cover cold dev-server starts
/// (Next.js dev compiles on first request — sub-3s on most machines).
const PROBE_TIMEOUT_MS: u64 = 2500;

/// Maximum probe attempts. Retries on transient transport errors only
/// (timeout, connect refused). Total worst-case ≈ MAX_ATTEMPTS *
/// PROBE_TIMEOUT_MS — the scheduler tick should comfortably exceed it.
const MAX_ATTEMPTS: u8 = 2;

impl Default for Prober {
    fn default() -> Self {
        Self::new()
    }
}

impl Prober {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(PROBE_TIMEOUT_MS))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("portbook/0.1")
            .build()
            .expect("reqwest client");
        Self { client }
    }

    pub async fn probe(&self, port: u16) -> ProbeResult {
        let url = format!("http://127.0.0.1:{port}/");
        let probed_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let start = Instant::now();

        let mut attempts: u8 = 0;
        let resp = loop {
            attempts += 1;
            match self.client.get(&url).send().await {
                Ok(r) => break r,
                Err(e) => {
                    let class = classify_err(&e);
                    // Only retry transient transport errors. A `Decode`/`Body`
                    // failure means something non-HTTP is on the socket —
                    // retrying won't change the answer.
                    let retryable = matches!(class, ProbeError::Timeout | ProbeError::Connect);
                    if !retryable || attempts >= MAX_ATTEMPTS {
                        let elapsed_ms = start.elapsed().as_millis() as u32;
                        // A redirect-cap error proves the server speaks HTTP —
                        // classify as Error (not Dead) so users don't see
                        // "not HTTP" for a working dev server with a long
                        // redirect chain.
                        let kind = match class {
                            ProbeError::Redirect => ProbeKind::Error,
                            _ => ProbeKind::Dead,
                        };
                        return ProbeResult {
                            kind,
                            status: None,
                            title: None,
                            description: None,
                            reason: Some(short_err(&e)),
                            probed_url: url,
                            probed_at_unix,
                            elapsed_ms,
                            error_class: Some(class),
                            error_detail: Some(truncate(&e.to_string(), 240)),
                            attempts,
                        };
                    }
                }
            }
        };

        let status = resp.status().as_u16();
        let body = resp.bytes().await.unwrap_or_default();
        let elapsed_ms = start.elapsed().as_millis() as u32;
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
        ProbeResult {
            kind,
            status: Some(status),
            title,
            description,
            reason,
            probed_url: url,
            probed_at_unix,
            elapsed_ms,
            error_class: None,
            error_detail: None,
            attempts,
        }
    }
}

fn short_err(e: &reqwest::Error) -> String {
    if e.is_timeout() { return "timeout".into(); }
    if e.is_connect() { return "connection refused".into(); }
    if e.is_redirect() { return "redirect chain".into(); }
    if e.is_decode() || e.is_body() { return "non-HTTP response".into(); }
    "not HTTP".into()
}

fn classify_err(e: &reqwest::Error) -> ProbeError {
    if e.is_timeout() { return ProbeError::Timeout; }
    if e.is_connect() { return ProbeError::Connect; }
    if e.is_redirect() { return ProbeError::Redirect; }
    if e.is_decode() { return ProbeError::Decode; }
    if e.is_body() { return ProbeError::Body; }
    ProbeError::Other
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
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
