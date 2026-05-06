use super::{ProcInfo, ProcessInspector};
use std::fs;

pub struct ProcInspector;

impl ProcessInspector for ProcInspector {
    fn inspect(&self, pid: u32) -> ProcInfo {
        let cwd = fs::read_link(format!("/proc/{pid}/cwd"))
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_string()));
        let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok().map(|b| {
            String::from_utf8_lossy(&b)
                .replace('\0', " ")
                .trim()
                .to_string()
        });
        ProcInfo { cwd, cmdline }
    }
}
