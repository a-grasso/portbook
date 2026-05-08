//! Strip secrets out of process command lines before they leave the host.
//!
//! Dev tooling routinely passes credentials via argv (`--token foo`,
//! `DATABASE_URL=postgres://u:p@h/db`, etc.). Portbook surfaces cmdlines in
//! the dashboard and over `/api/*`, so anything we don't redact here is
//! visible to anyone who can read those — including a screen-share viewer
//! or a future bug that widens API exposure.

use std::borrow::Cow;

const REDACTED: &str = "***";

pub fn redact_cmdline(cmdline: &str) -> String {
    let toks: Vec<&str> = cmdline.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];

        if let Some(eq) = t.find('=') {
            let (k, v) = (&t[..eq], &t[eq + 1..]);
            if is_secret_key(&normalize_key(k)) {
                out.push(format!("{k}={REDACTED}"));
            } else {
                out.push(format!("{k}={}", redact_url_userinfo(v)));
            }
            i += 1;
            continue;
        }

        if t.starts_with('-') && is_secret_key(&normalize_key(t)) && i + 1 < toks.len() {
            out.push(t.to_string());
            out.push(REDACTED.to_string());
            i += 2;
            continue;
        }

        out.push(redact_url_userinfo(t).into_owned());
        i += 1;
    }
    out.join(" ")
}

fn normalize_key(k: &str) -> String {
    k.trim_start_matches("--")
        .trim_start_matches('-')
        .replace('-', "_")
        .to_ascii_lowercase()
}

fn is_secret_key(k: &str) -> bool {
    matches!(
        k,
        "p" // psql/mysql short flag for password
            | "token"
            | "password"
            | "passwd"
            | "pass"
            | "secret"
            | "key"
            | "api_key"
            | "apikey"
            | "auth"
            | "authorization"
            | "bearer"
            | "access_token"
            | "refresh_token"
            | "private_key"
            | "client_secret"
            | "aws_secret_access_key"
            | "aws_access_key_id"
            | "database_password"
    )
}

/// `scheme://user:pass@host/...` → `scheme://user:***@host/...`.
/// Touches only userinfo; host/path/query are kept so users still see which service it is.
fn redact_url_userinfo(s: &str) -> Cow<'_, str> {
    let Some(scheme_end) = s.find("://") else { return Cow::Borrowed(s) };
    let scheme_end = scheme_end + 3;
    let rest = &s[scheme_end..];
    let Some(at) = rest.find('@') else { return Cow::Borrowed(s) };
    let path_start = rest.find('/').unwrap_or(rest.len());
    if at >= path_start {
        return Cow::Borrowed(s);
    }
    let userinfo = &rest[..at];
    let after = &rest[at..]; // includes '@'
    let Some(colon) = userinfo.find(':') else {
        return Cow::Borrowed(s);
    };
    let user = &userinfo[..colon];
    Cow::Owned(format!("{}{}:{}{}", &s[..scheme_end], user, REDACTED, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_innocuous_cmdline() {
        assert_eq!(
            redact_cmdline("node server.js --port 3000"),
            "node server.js --port 3000"
        );
    }

    #[test]
    fn redacts_kv_secret_flags() {
        assert_eq!(redact_cmdline("--token=abc123"), "--token=***");
        assert_eq!(redact_cmdline("--api-key=xyz"), "--api-key=***");
        assert_eq!(redact_cmdline("--client-secret=foo"), "--client-secret=***");
    }

    #[test]
    fn redacts_kv_env_style() {
        assert_eq!(redact_cmdline("TOKEN=abc"), "TOKEN=***");
        assert_eq!(redact_cmdline("PASSWORD=hunter2"), "PASSWORD=***");
        assert_eq!(redact_cmdline("AWS_SECRET_ACCESS_KEY=xyz"), "AWS_SECRET_ACCESS_KEY=***");
    }

    #[test]
    fn redacts_space_separated_flag() {
        assert_eq!(redact_cmdline("mycli --token abc123"), "mycli --token ***");
        assert_eq!(redact_cmdline("psql -p hunter2 db"), "psql -p *** db");
    }

    #[test]
    fn redacts_url_userinfo() {
        assert_eq!(
            redact_cmdline("postgres://user:pw@host/db"),
            "postgres://user:***@host/db"
        );
        assert_eq!(
            redact_cmdline("DATABASE_URL=postgres://u:secret@h:5432/app"),
            "DATABASE_URL=postgres://u:***@h:5432/app"
        );
    }

    #[test]
    fn leaves_url_without_password_alone() {
        assert_eq!(
            redact_cmdline("https://user@github.com/repo.git"),
            "https://user@github.com/repo.git"
        );
        assert_eq!(redact_cmdline("https://example.com/x"), "https://example.com/x");
    }

    #[test]
    fn does_not_treat_at_in_path_as_userinfo() {
        // The '@' is past the first '/' — not userinfo.
        assert_eq!(
            redact_cmdline("https://example.com/users/me@org"),
            "https://example.com/users/me@org"
        );
    }

    #[test]
    fn does_not_redact_words_mid_argv() {
        // "password" without a leading dash is not a flag — leave it.
        assert_eq!(
            redact_cmdline("echo my password is safe"),
            "echo my password is safe"
        );
    }

    #[test]
    fn handles_mixed_cmdline() {
        let input = "node app.js --token=abc DATABASE_URL=postgres://u:p@h/db --port 3000";
        let want = "node app.js --token=*** DATABASE_URL=postgres://u:***@h/db --port 3000";
        assert_eq!(redact_cmdline(input), want);
    }

    #[test]
    fn handles_empty() {
        assert_eq!(redact_cmdline(""), "");
        assert_eq!(redact_cmdline("   "), "");
    }

    #[test]
    fn trailing_secret_flag_with_no_value_is_safe() {
        // No following token — don't index past the end.
        assert_eq!(redact_cmdline("mycli --token"), "mycli --token");
    }
}
