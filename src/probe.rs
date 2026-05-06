use scraper::{Html, Selector};
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProbeResult {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: u16,
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

    /// Returns Some(result) if the port speaks HTTP and responds, else None.
    pub async fn probe(&self, port: u16) -> Option<ProbeResult> {
        let url = format!("http://127.0.0.1:{port}/");
        let resp = self.client.get(&url).send().await.ok()?;
        let status = resp.status().as_u16();
        let bytes = resp.bytes().await.ok()?;
        let take = bytes.len().min(64 * 1024);
        let html = String::from_utf8_lossy(&bytes[..take]);
        let (title, description) = extract(&html);
        Some(ProbeResult { title, description, status })
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
