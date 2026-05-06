use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub struct Listener {
    pub port: u16,
    pub pid: u32,
    pub command: String,
}

pub trait PortEnumerator: Send + Sync {
    fn list(&self) -> anyhow::Result<Vec<Listener>>;
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
pub fn default() -> Box<dyn PortEnumerator> {
    Box::new(macos::LsofEnumerator)
}

#[cfg(target_os = "linux")]
pub fn default() -> Box<dyn PortEnumerator> {
    Box::new(linux::SsEnumerator)
}
