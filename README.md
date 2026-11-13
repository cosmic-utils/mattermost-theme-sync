# cosmicmost

Small CLI/daemon that keeps your Mattermost account theme in step with COSMIC (dark/light, accent, and the main palette colors).

I mainly use it with Mattermost Desktop on COSMIC; it pushes a custom theme preference to the server whenever COSMIC changes, so the web/desktop client follows along.

## Install

From a release:

```bash
curl -fsSL https://github.com/cosmic-utils/mattermost-theme-sync/releases/latest/download/install.sh | sh
```

Or from source:

```bash
git clone https://github.com/cosmic-utils/mattermost-theme-sync
cd mattermost-theme-sync
cargo install --path .
```

## Setup

1. Create a Mattermost personal access token (*Profile → Security → Personal Access Tokens*).
2. Run:

```bash
cosmicmost init
cosmicmost install-service
```

`init` asks for the server URL and token (token input is hidden) and stores the token in your system keychain via Secret Service. Server/settings go into cosmic-config.

To try it once without the service:

```bash
cosmicmost preview   # print generated theme JSON
cosmicmost apply
```

## Notes

- Mattermost Desktop v6+ has a separate setting to make the app chrome follow the server theme: *Settings → Advanced → Synchronize Desktop App theme with server*.
- By default `init` configures sync for the global theme and each team; pass `--no-sync-all-teams` if you only want the global one.
- Needs a working user keyring/Secret Service for token storage.
- Service logs: `journalctl --user -u cosmicmost -f`

## License

GPL-3.0-only. See [LICENSE](LICENSE).
