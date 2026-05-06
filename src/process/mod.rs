use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProcInfo {
    pub cwd: Option<String>,
    pub cmdline: Option<String>,
}

pub trait ProcessInspector: Send + Sync {
    fn inspect(&self, pid: u32) -> ProcInfo;
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
pub fn default() -> Box<dyn ProcessInspector> {
    Box::new(macos::LsofInspector)
}

#[cfg(target_os = "linux")]
pub fn default() -> Box<dyn ProcessInspector> {
    Box::new(linux::ProcInspector)
}
