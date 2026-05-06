use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const RELEASES_URL: &str = "https://api.github.com/repos/a-grasso/portbook/releases/latest";

#[derive(Debug, Clone, Serialize, Default)]
pub struct VersionInfo {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
}

#[derive(Clone, Default)]
pub struct VersionState {
    inner: Arc<RwLock<VersionInfo>>,
}

impl VersionState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(VersionInfo {
                current: CURRENT.to_string(),
                latest: None,
                update_available: false,
            })),
        }
    }

    pub async fn snapshot(&self) -> VersionInfo {
        self.inner.read().await.clone()
    }

    pub async fn set_latest(&self, latest: String) {
        let mut w = self.inner.write().await;
        w.update_available = is_newer(&latest, &w.current);
        w.latest = Some(latest);
    }
}

/// Spawn a background task that polls GitHub for the latest release once.
/// Failures are silent — update info is best-effort.
pub fn spawn_check(state: VersionState) {
    tokio::spawn(async move {
        if let Some(tag) = fetch_latest_tag().await {
            state.set_latest(tag).await;
        }
    });
}

async fn fetch_latest_tag() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent(concat!("portbook/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let resp = client.get(RELEASES_URL).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("tag_name")?.as_str().map(strip_v).map(String::from)
}

fn strip_v(s: &str) -> &str {
    s.strip_prefix('v').unwrap_or(s)
}

/// Returns true if `latest` is strictly greater than `current`.
/// Naive semver compare on dot-separated numeric segments; non-numeric
/// suffixes (e.g. -rc.1) are ignored on both sides.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit() && c != '.')
            .next()
            .unwrap_or("")
            .split('.')
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    };
    parse(latest) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_detects_patch_bump() {
        assert!(is_newer("0.1.5", "0.1.4"));
    }

    #[test]
    fn newer_rejects_same() {
        assert!(!is_newer("0.1.4", "0.1.4"));
    }

    #[test]
    fn newer_rejects_older() {
        assert!(!is_newer("0.1.3", "0.1.4"));
    }

    #[test]
    fn newer_handles_minor_and_major() {
        assert!(is_newer("0.2.0", "0.1.99"));
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn newer_ignores_prerelease_suffix() {
        assert!(!is_newer("0.1.4-rc.1", "0.1.4"));
    }

    #[test]
    fn strip_v_works() {
        assert_eq!(strip_v("v1.2.3"), "1.2.3");
        assert_eq!(strip_v("1.2.3"), "1.2.3");
    }
}
