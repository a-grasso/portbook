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
