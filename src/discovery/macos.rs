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

#[cfg(test)]
fn parse_lsof_as_user(text: &str, user: &str) -> Vec<Listener> {
    // Test seam: bypass $USER lookup so tests are deterministic.
    // Mirrors parse_lsof but takes the user explicitly.
    let mut out = Vec::new();
    let mut pid: Option<u32> = None;
    let mut cmd = String::new();
    let mut cur_user = String::new();
    let mut seen: HashMap<u16, ()> = HashMap::new();
    for line in text.lines() {
        let Some((tag, val)) = line.split_at_checked(1) else { continue };
        match tag {
            "p" => { pid = val.parse().ok(); cmd.clear(); cur_user.clear(); }
            "c" => cmd = val.to_string(),
            "L" => cur_user = val.to_string(),
            "n" => {
                if !user.is_empty() && cur_user != user { continue; }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_listener() {
        let input = "p1234\ncnode\nLalice\nn*:3000\n";
        let out = parse_lsof_as_user(input, "alice");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].port, 3000);
        assert_eq!(out[0].pid, 1234);
        assert_eq!(out[0].command, "node");
    }

    #[test]
    fn filters_other_users() {
        let input = "p1234\ncnode\nLbob\nn*:3000\n";
        assert!(parse_lsof_as_user(input, "alice").is_empty());
    }

    #[test]
    fn filters_non_loopback_binds() {
        let input = "p1234\ncnode\nLalice\nn192.168.1.5:3000\n";
        assert!(parse_lsof_as_user(input, "alice").is_empty());
    }

    #[test]
    fn dedupes_dual_stack_listeners() {
        // Same process bound to v4 and v6 on the same port: count once.
        let input = "p1234\ncnode\nLalice\nn127.0.0.1:3000\nn[::1]:3000\n";
        let out = parse_lsof_as_user(input, "alice");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].port, 3000);
    }

    #[test]
    fn handles_multiple_processes() {
        let input = "p1\ncnode\nLalice\nn*:3000\np2\ncpython\nLalice\nn127.0.0.1:8000\n";
        let out = parse_lsof_as_user(input, "alice");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].port, 3000);
        assert_eq!(out[1].port, 8000);
        assert_eq!(out[1].command, "python");
    }

    #[test]
    fn no_user_filter_when_empty() {
        let input = "p1234\ncnode\nLbob\nn*:3000\n";
        let out = parse_lsof_as_user(input, "");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn parse_port_rejects_external_host() {
        assert_eq!(parse_port("10.0.0.1:3000"), None);
        assert_eq!(parse_port("8.8.8.8:53"), None);
    }

    #[test]
    fn parse_port_accepts_loopback_forms() {
        assert_eq!(parse_port("*:3000"), Some(3000));
        assert_eq!(parse_port("127.0.0.1:7777"), Some(7777));
        assert_eq!(parse_port("[::1]:8080"), Some(8080));
        assert_eq!(parse_port("[::]:5432"), Some(5432));
        assert_eq!(parse_port("localhost:1234"), Some(1234));
    }
}
