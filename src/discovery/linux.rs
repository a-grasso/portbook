use super::{Listener, PortEnumerator};
use std::process::Command;

pub struct SsEnumerator;

impl PortEnumerator for SsEnumerator {
    fn list(&self) -> anyhow::Result<Vec<Listener>> {
        let out = Command::new("ss").args(["-tlnpH"]).output()?;
        if !out.status.success() {
            anyhow::bail!("ss failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(parse_ss(&String::from_utf8_lossy(&out.stdout)))
    }
}

fn parse_ss(text: &str) -> Vec<Listener> {
    let mut out = Vec::new();
    for line in text.lines() {
        // Columns: State Recv-Q Send-Q LocalAddress:Port PeerAddress:Port [Process]
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 { continue; }
        let local = fields[3];
        let Some(port) = local.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok()) else { continue };
        let host = local.rsplit_once(':').map(|(h, _)| h).unwrap_or("");
        let is_local = matches!(host, "*" | "0.0.0.0" | "127.0.0.1" | "[::]" | "[::1]");
        if !is_local { continue; }
        // Process column may not be present without root; try to extract.
        let mut pid: u32 = 0;
        let mut command = String::new();
        if let Some(proc) = fields.get(5) {
            // users:(("nginx",pid=1234,fd=6))
            if let Some(start) = proc.find("\"") {
                if let Some(end) = proc[start + 1..].find("\"") {
                    command = proc[start + 1..start + 1 + end].to_string();
                }
            }
            if let Some(idx) = proc.find("pid=") {
                let rest = &proc[idx + 4..];
                let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                pid = n.parse().unwrap_or(0);
            }
        }
        out.push(Listener { port, pid, command });
    }
    out
}
