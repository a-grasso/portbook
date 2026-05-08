use std::path::{Path, PathBuf};

const MARKERS: &[&str] = &[
    ".git",
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "composer.json",
    "Gemfile",
];

pub fn detect_root(cwd: &str) -> Option<String> {
    let mut p = PathBuf::from(cwd);
    loop {
        if has_marker(&p) {
            return Some(p.to_string_lossy().to_string());
        }
        if !p.pop() { return None; }
    }
}

fn has_marker(dir: &Path) -> bool {
    MARKERS.iter().any(|m| dir.join(m).exists())
}

pub fn folder_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// Abbreviate a path: keep the leaf intact, collapse each parent segment to its first
/// character, and substitute `$HOME` with `~`. Examples:
/// `/Users/agr/Projects/private/portbook` → `~/P/p/portbook`,
/// `/var/www/site` → `/v/w/site`.
pub fn shrink_path(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let display = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path == home => return "~".into(),
        Ok(home) if !home.is_empty() && path.starts_with(&format!("{home}/")) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    };

    let (leading, rest) = if let Some(r) = display.strip_prefix("~/") {
        ("~/", r)
    } else if let Some(r) = display.strip_prefix('/') {
        ("/", r)
    } else {
        ("", display.as_str())
    };

    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 1 {
        return display;
    }

    let mut out = String::from(leading);
    let last = parts.len() - 1;
    for (i, part) in parts.iter().enumerate() {
        if i == last {
            out.push_str(part);
        } else {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                out.push(first);
                // Hidden dirs (.config) get two chars so they're distinguishable.
                if first == '.'
                    && let Some(c) = chars.next()
                {
                    out.push(c);
                }
            }
            out.push('/');
        }
    }
    out
}

#[cfg(test)]
mod shrink_path_tests {
    use super::*;

    fn with_home<F: FnOnce()>(home: &str, f: F) {
        let prev = std::env::var_os("HOME");
        // SAFETY: cargo test threads can race on env, but HOME reads in shrink_path are
        // brief and confined to this test module; we restore the previous value below.
        unsafe { std::env::set_var("HOME", home); }
        f();
        match prev {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }

    #[test]
    fn abbreviates_parents_and_keeps_leaf() {
        with_home("/nonexistent", || {
            assert_eq!(shrink_path("/Users/agr/Projects/private/portbook"), "/U/a/P/p/portbook");
            assert_eq!(shrink_path("/var/www/site"), "/v/w/site");
        });
    }

    #[test]
    fn replaces_home_with_tilde() {
        with_home("/Users/agr", || {
            assert_eq!(shrink_path("/Users/agr/Projects/private/portbook"), "~/P/p/portbook");
            assert_eq!(shrink_path("/Users/agr"), "~");
        });
    }

    #[test]
    fn keeps_short_paths_intact() {
        with_home("/nonexistent", || {
            assert_eq!(shrink_path("/portbook"), "/portbook");
            assert_eq!(shrink_path("portbook"), "portbook");
            assert_eq!(shrink_path(""), "");
        });
    }

    #[test]
    fn hidden_dirs_keep_two_chars() {
        with_home("/nonexistent", || {
            assert_eq!(shrink_path("/home/u/.config/portbook"), "/h/u/.c/portbook");
        });
    }

    #[test]
    fn home_prefix_must_be_full_segment() {
        with_home("/Users/agr", || {
            // /Users/agroup must NOT collapse to ~roup
            assert_eq!(shrink_path("/Users/agroup/x/y"), "/U/a/x/y");
        });
    }
}
