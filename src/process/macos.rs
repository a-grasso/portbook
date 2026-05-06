use super::{ProcInfo, ProcessInspector};
use std::process::Command;

pub struct LsofInspector;

impl ProcessInspector for LsofInspector {
    fn inspect(&self, pid: u32) -> ProcInfo {
        let cwd = lsof_cwd(pid);
        let cmdline = ps_cmdline(pid);
        ProcInfo { cwd, cmdline }
    }
}

fn lsof_cwd(pid: u32) -> Option<String> {
    let out = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix('n') {
            return Some(rest.to_string());
        }
    }
    None
}

fn ps_cmdline(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}
