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
    pub probed_url: String,
    pub probed_at_unix: u64,
    pub elapsed_ms: u32,
    pub error_class: Option<ProbeError>,
    /// Truncated `Display` of the underlying error, for diagnostics paste.
    pub error_detail: Option<String>,
    pub attempts: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProbeKind {
    Live,
    Error,
    Dead,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProbeError {
    Timeout,
    Connect,
    Decode,
    Body,
    Redirect,
    Other,
}

pub struct Prober {
    client: reqwest::Client,
}

/// Per-attempt timeout, sized to cover cold dev-server starts (e.g. Next.js compiles
/// on first request — sub-3s on most machines).
const PROBE_TIMEOUT_MS: u64 = 2500;

/// Worst-case ≈ MAX_ATTEMPTS * PROBE_TIMEOUT_MS; must stay below the scheduler tick.
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
        let probed_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let start = Instant::now();

        // Try IPv4 loopback first; if nothing is listening there, retry the IPv6
        // loopback before giving up — Vite and other tools bind [::1] only.
        let resp = match self.request(&format!("http://127.0.0.1:{port}/")).await {
            Ok(r) => r,
            Err(res) if res.class == ProbeError::Connect => {
                match self.request(&format!("http://[::1]:{port}/")).await {
                    Ok(r) => r,
                    Err(res6) => {
                        return res6.into_result(start.elapsed().as_millis() as u32, probed_at_unix);
                    }
                }
            }
            Err(res) => return res.into_result(start.elapsed().as_millis() as u32, probed_at_unix),
        };

        let attempts = resp.attempts;
        let url = resp.url;
        let resp = resp.resp;
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

    async fn request(&self, url: &str) -> Result<ProbedResponse, ProbeFailure> {
        let mut attempts: u8 = 0;
        loop {
            attempts += 1;
            match self.client.get(url).send().await {
                Ok(resp) => {
                    return Ok(ProbedResponse { resp, url: url.to_string(), attempts });
                }
                Err(e) => {
                    let class = classify_err(&e);
                    // Decode/Body failures mean something non-HTTP is on the socket —
                    // retrying won't change the answer.
                    let retryable = matches!(class, ProbeError::Timeout | ProbeError::Connect);
                    if !retryable || attempts >= MAX_ATTEMPTS {
                        return Err(ProbeFailure {
                            class,
                            reason: short_err(&e),
                            detail: truncate(&e.to_string(), 240),
                            url: url.to_string(),
                            attempts,
                        });
                    }
                }
            }
        }
    }
}

struct ProbedResponse {
    resp: reqwest::Response,
    url: String,
    attempts: u8,
}

struct ProbeFailure {
    class: ProbeError,
    reason: String,
    detail: String,
    url: String,
    attempts: u8,
}

impl ProbeFailure {
    fn into_result(self, elapsed_ms: u32, probed_at_unix: u64) -> ProbeResult {
        // A redirect-cap error proves the server speaks HTTP, so it belongs in
        // Error (not Dead).
        let kind = match self.class {
            ProbeError::Redirect => ProbeKind::Error,
            _ => ProbeKind::Dead,
        };
        ProbeResult {
            kind,
            status: None,
            title: None,
            description: None,
            reason: Some(self.reason),
            probed_url: self.url,
            probed_at_unix,
            elapsed_ms,
            error_class: Some(self.class),
            error_detail: Some(self.detail),
            attempts: self.attempts,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn serve_once(addr: &str) -> u16 {
        let listener = TcpListener::bind(addr).await.expect("bind");
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let body = "<html><head><title>Renderer</title></head><body></body></html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        port
    }

    #[tokio::test]
    async fn falls_back_to_ipv6_loopback_when_v4_refuses() {
        let port = serve_once("[::1]:0").await;
        let result = Prober::new().probe(port).await;
        assert_eq!(result.kind, ProbeKind::Live, "reason: {:?}", result.reason);
        assert_eq!(result.probed_url, format!("http://[::1]:{port}/"));
        assert_eq!(result.title.as_deref(), Some("Renderer"));
    }

    #[tokio::test]
    async fn probes_ipv4_loopback_directly() {
        let port = serve_once("127.0.0.1:0").await;
        let result = Prober::new().probe(port).await;
        assert_eq!(result.kind, ProbeKind::Live, "reason: {:?}", result.reason);
        assert_eq!(result.probed_url, format!("http://127.0.0.1:{port}/"));
    }
}
