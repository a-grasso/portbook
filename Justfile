# portbook task runner.
#
# Install just: `brew install just` (or see https://github.com/casey/just).
# Run `just` with no args to list recipes.

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

_default:
    @just --list

# Cut a release: bump version, regen changelog, commit, tag, push.
# Mirrors the 6-step flow in CONTRIBUTING.md so it can't drift.
#
# Usage: just release 0.1.6
release VERSION:
    #!/usr/bin/env bash
    set -euo pipefail

    VERSION="{{VERSION}}"
    TAG="v${VERSION}"

    # Pre-flight: vX.Y.Z shape, on main, clean tree, in sync with origin.
    if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$ ]]; then
        echo "error: version must be X.Y.Z (got '$VERSION')" >&2
        exit 1
    fi
    branch=$(git rev-parse --abbrev-ref HEAD)
    if [[ "$branch" != "main" ]]; then
        echo "error: must be on main (currently on '$branch')" >&2
        exit 1
    fi
    if ! git diff-index --quiet HEAD --; then
        echo "error: working tree not clean — commit or stash first" >&2
        exit 1
    fi
    if git rev-parse "$TAG" >/dev/null 2>&1; then
        echo "error: tag $TAG already exists" >&2
        exit 1
    fi
    git fetch --quiet origin main
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
        echo "error: local main not in sync with origin/main — pull/push first" >&2
        exit 1
    fi
    for tool in git-cliff cargo; do
        command -v "$tool" >/dev/null || { echo "error: $tool not on PATH" >&2; exit 1; }
    done

    echo "→ bumping Cargo.toml to $VERSION"
    # Match only the [package] version line — not dependency versions.
    awk -v v="$VERSION" '
        /^\[package\]/ { in_pkg=1 }
        /^\[/ && !/^\[package\]/ { in_pkg=0 }
        in_pkg && /^version[[:space:]]*=/ { sub(/"[^"]*"/, "\"" v "\"") }
        { print }
    ' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml

    echo "→ refreshing Cargo.lock"
    cargo build --quiet

    echo "→ regenerating CHANGELOG.md for $TAG"
    git cliff --tag "$TAG" -o CHANGELOG.md

    echo "→ commit + annotated tag"
    git commit -am "chore(release): $TAG"
    git tag -a "$TAG" -m "$TAG"

    echo "→ pushing main + tag"
    git push --follow-tags

    echo
    echo "released $TAG — cargo-dist will build artifacts via .github/workflows/release.yml"

# Preview the release notes without cutting.
release-preview VERSION:
    git cliff --unreleased --tag v{{VERSION}}

# Standard checks: build, clippy, test.
check:
    cargo build --all-targets
    cargo clippy --all-targets -- -D warnings
    cargo test
