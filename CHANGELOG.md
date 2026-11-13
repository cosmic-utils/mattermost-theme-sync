# Changelog

## [0.1.0] - 2026-11-12

First tagged release.

- Generates a Mattermost custom theme from the active COSMIC theme (light/dark, accent, palette) and pushes it via the preferences API (global + optionally per-team).
- `init` / `preview` / `apply` / `daemon` / `reset` / `status` / `paths` / `install-service` commands.
- Watches COSMIC theme/config changes and resyncs (with a small debounce) in daemon mode.
- Config/state stored with `cosmic-config`; personal access token stored in the system keychain (Secret Service), entered via hidden prompt.
