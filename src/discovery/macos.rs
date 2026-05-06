use super::{Listener, PortEnumerator};
use std::collections::HashMap;
use std::process::Command;

pub struct LsofEnumerator;

impl PortEnumerator for LsofEnumerator {
    fn list(&self) -> anyhow::Result<Vec<Listener>> {
        // -F pcLn => fields: p=pid, c=command, L=login (user), n=name (host:port)
        // -nP => no DNS, no port-name resolution
        let out = Command::new("lsof")
            .args(["-iTCP", "-sTCP:LISTEN", "-nP", "-FpcLn"])
            .output()?;
        if !out.status.success() {
            anyhow::bail!("lsof failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(parse_lsof(&String::from_utf8_lossy(&out.stdout)))
    }
}

fn parse_lsof(text: &str) -> Vec<Listener> {
    let me = std::env::var("USER").unwrap_or_default();
    let mut out = Vec::new();
    let mut pid: Option<u32> = None;
    let mut cmd = String::new();
    let mut user = String::new();
    let mut seen: HashMap<u16, ()> = HashMap::new();

    for line in text.lines() {
        let Some((tag, val)) = line.split_at_checked(1) else { continue };
        match tag {
            "p" => {
                pid = val.parse().ok();
                cmd.clear();
                user.clear();
            }
            "c" => cmd = val.to_string(),
            "L" => user = val.to_string(),
            "n" => {
                if !me.is_empty() && user != me { continue; }
                let Some(port) = parse_port(val) else { continue };
                if seen.insert(port, ()).is_some() { continue; }
                if let Some(pid) = pid {
                    out.push(Listener { port, pid, command: cmd.clone() });
                }
            }
            _ => {}
        }
    }
    out
}

fn parse_port(name: &str) -> Option<u16> {
    // Names look like "*:3000", "127.0.0.1:7777", "[::1]:8080", "[::]:5432"
    let host_port = name.rsplit_once(':')?;
    let host = host_port.0;
    let port: u16 = host_port.1.parse().ok()?;
    let is_loopback_or_any = matches!(host, "*" | "127.0.0.1" | "[::1]" | "[::]" | "localhost");
    if !is_loopback_or_any { return None; }
    Some(port)
}
