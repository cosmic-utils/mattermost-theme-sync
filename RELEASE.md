# Releasing

Tags (`v*`) trigger the release workflow: it builds linux binaries (x86_64 + aarch64), packages them, and publishes a GitHub release with checksums and `install.sh`.

Usual flow:

1. Update `Cargo.toml` version (+ `Cargo.lock` if needed) and add a `CHANGELOG.md` entry.
2. Run checks: `just release-check` (fmt, clippy, tests, installer syntax).
3. Commit and push to `main`.
4. Tag and push, e.g. `git tag -a v0.1.0 -m "v0.1.0" && git push origin v0.1.0` (tag must match `Cargo.toml`).
5. Sanity-check the published release/assets and the installer.

If a workflow run fails, re-run it from Actions or dispatch it for the tag (`gh workflow run Release -f tag=v0.1.0`). Avoid moving/reusing a tag people may have already installed.
