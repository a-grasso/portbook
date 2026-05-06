# Contributing to portbook

Thanks for working on portbook! This doc covers the **process** rules:
how to commit, how to release. For the **content** rules — what good
code looks like in this repo — see:

- [ARCHITECTURE.md](ARCHITECTURE.md) — Engine/CLI/UI layering.
- [CONVENTIONS.md](CONVENTIONS.md) — Rust CLI quality bar.

## Commit conventions

Every commit on `main` **must** follow [Conventional
Commits](https://www.conventionalcommits.org/). Non-conforming commits
are silently dropped from release notes by [cliff.toml](cliff.toml).

### Format

```
<type>(<scope>): <subject>

<optional body — wrap at 72 cols, explain *why*, include test counts>

<optional footers — BREAKING CHANGE:, Refs: #123, etc.>
```

- **Subject**: imperative, lowercase, no trailing period, ≤ 72 chars.
- **Body**: keep it. git-cliff renders only the subject in release
  notes, but bodies stay in `git log` for code archaeology — that's
  the right split. Don't compress bodies for release notes' sake.

### Types (ordered as they appear in CHANGELOG.md)

| Type       | Use for                                            | In release notes  |
|------------|----------------------------------------------------|-------------------|
| `feat`     | New user-visible capability or flag                | **Features**      |
| `fix`      | Bug fix that changes runtime behavior              | **Bug Fixes**     |
| `perf`     | Speed/memory improvement, no behavior change       | **Performance**   |
| `refactor` | Internal restructuring, no behavior change         | **Refactor**      |
| `docs`     | README, ARCHITECTURE.md, code comments             | **Documentation** |
| `test`     | Test-only changes (new tests, fixtures, harnesses) | **Tests**         |
| `build`    | Cargo.toml, dist-workspace.toml, build scripts     | **Build**         |
| `ci`       | `.github/workflows/`, release automation           | **CI**            |
| `chore`    | Tooling, lockfile bumps, anything else             | **Chores**        |
| `revert`   | `git revert` of a prior commit                     | **Reverts**       |

Anything outside this list is dropped — keep to it. If unsure,
default to `chore` (visible under "Chores" — better than dropped).

### Scopes (optional but encouraged)

The scope narrows the type. Use what's accurate, don't invent
decorative ones. Common scopes in this repo:

- `cli` — anything under `src/cli/` or affecting CLI surface
- `engine` — `src/engine.rs`, discovery, probing
- `api` — `src/api.rs`, HTTP routes
- `ui` — `src/web/` assets, `serve` subcommand UI
- `readme` — README.md edits specifically (use `docs(readme)`)

### Breaking changes

A breaking change to the CLI surface, JSON schema, or HTTP API:

```
feat(cli)!: rename `ui` subcommand to `serve`

BREAKING CHANGE: `portbook ui` is now `portbook serve`. The old name
is kept as a hidden alias for one release.
```

Either the `!` after the type/scope **or** a `BREAKING CHANGE:` footer
triggers a `[breaking]` marker in CHANGELOG.md.

### Examples (drawn from this repo)

```
feat(cli): add `portbook ls --json` for machine-readable output
refactor(cli): split cli.rs into focused submodules
docs: add ARCHITECTURE.md — north star for portbook layering
docs(readme): document CLI usage, JSON schema, exit codes
```

## Release flow

Releases are tag-driven. Pushing a `vX.Y.Z` tag triggers cargo-dist's
[release workflow](.github/workflows/release.yml), which builds
artifacts and creates a GitHub Release using the matching section of
`CHANGELOG.md` as the body.

### Cutting a release

1. **Land all work on `main`** with conventional-commit messages.
2. **Bump `version`** in [Cargo.toml](Cargo.toml).
3. **Refresh the lockfile**: `cargo build`.
4. **Regenerate the changelog**: `git cliff --tag vX.Y.Z -o CHANGELOG.md`.
5. **Commit**: `git commit -am "chore(release): vX.Y.Z"`.
6. **Tag and push**: `git tag -a vX.Y.Z -m "vX.Y.Z" && git push --follow-tags`.
   The annotated `-a` tag is required for `--follow-tags` to ship it;
   lightweight tags stay local.

Do **not** edit `CHANGELOG.md` by hand — it's regenerated from
`git log`. If a release note is wrong, fix the underlying commit
message (or add a follow-up commit) and regenerate.

### Previewing release notes

```
git cliff --unreleased --tag vX.Y.Z
```

This prints what the release body will look like without writing the
file — handy before you commit a misnamed type.
