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
        // `ss -p` only fills the process column for sockets the caller can
        // see — own-user as non-root, all sockets as root. Treat missing
        // process info as evidence that the socket belongs to a *different*
        // user and skip it. This mirrors the $USER filter we apply on macOS
        // and prevents leaking other users' listeners on shared dev hosts.
        // Caveat: running portbook as root will surface every user's
        // sockets, same as macOS would under root.
        let Some(proc) = fields.get(5) else { continue };

        let mut pid: u32 = 0;
        let mut command = String::new();
        // users:(("nginx",pid=1234,fd=6))
        if let Some(start) = proc.find('"') {
            if let Some(end) = proc[start + 1..].find('"') {
                command = proc[start + 1..start + 1 + end].to_string();
            }
        }
        if let Some(idx) = proc.find("pid=") {
            let rest = &proc[idx + 4..];
            let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            pid = n.parse().unwrap_or(0);
        }
        // Defence in depth: ss occasionally emits process columns with
        // pid=0 for kernel-owned listeners. Don't surface those.
        if pid == 0 { continue; }

        out.push(Listener { port, pid, command });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_listener_with_process_info() {
        let line = "LISTEN 0 128 127.0.0.1:7777 0.0.0.0:* users:((\"portbook\",pid=4242,fd=6))";
        let out = parse_ss(line);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].port, 7777);
        assert_eq!(out[0].pid, 4242);
        assert_eq!(out[0].command, "portbook");
    }

    #[test]
    fn drops_listener_without_process_info() {
        // Privacy boundary: ss only fills the process column for sockets the
        // current user can see, so a missing process column means the socket
        // belongs to *another* user — we must not surface it.
        let line = "LISTEN 0 128 127.0.0.1:5432 0.0.0.0:*";
        assert!(parse_ss(line).is_empty());
    }

    #[test]
    fn drops_listener_with_zero_pid() {
        // Some ss versions emit a process column with pid=0 for kernel-owned
        // listeners. They aren't user-owned services and shouldn't appear.
        let line = "LISTEN 0 128 127.0.0.1:1234 0.0.0.0:* users:((\"?\",pid=0,fd=0))";
        assert!(parse_ss(line).is_empty());
    }

    #[test]
    fn skips_non_loopback_binds() {
        let line = "LISTEN 0 128 192.168.1.5:8080 0.0.0.0:* users:((\"nginx\",pid=1,fd=6))";
        assert!(parse_ss(line).is_empty());
    }

    #[test]
    fn accepts_wildcard_and_v6() {
        let text = "LISTEN 0 128 0.0.0.0:3000 0.0.0.0:* users:((\"a\",pid=1,fd=6))\n\
                    LISTEN 0 128 [::]:8000 [::]:* users:((\"b\",pid=2,fd=6))\n\
                    LISTEN 0 128 [::1]:9000 [::]:* users:((\"c\",pid=3,fd=6))";
        let ports: Vec<u16> = parse_ss(text).into_iter().map(|l| l.port).collect();
        assert_eq!(ports, vec![3000, 8000, 9000]);
    }

    #[test]
    fn ignores_short_lines() {
        assert!(parse_ss("").is_empty());
        assert!(parse_ss("State Recv-Q Send-Q\n").is_empty());
    }
}
