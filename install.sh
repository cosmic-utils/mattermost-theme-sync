#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
#
# Install cosmicmost from GitHub releases.
#
# One-liner:
#   curl -fsSL https://github.com/cosmic-utils/mattermost-theme-sync/releases/latest/download/install.sh | sh
#
# Options (also available as environment variables):
#   --version <tag>   Install a specific release (default: latest)
#   --bindir <dir>    Install the binary here (default: ~/.local/bin)
#   --no-verify       Skip SHA-256 verification
#   -h, --help        Show this help
#
# Environment:
#   COSMICMOST_VERSION     Same as --version
#   COSMICMOST_BINDIR      Same as --bindir
#   COSMICMOST_NO_VERIFY   Set to 1 to skip checksum verification
#   COSMICMOST_BASE_URL    Download directory override (releases/mirrors)
#
set -eu

REPO="${COSMICMOST_REPO:-cosmic-utils/mattermost-theme-sync}"
BIN="cosmicmost"
VERSION="${COSMICMOST_VERSION:-}"
DEFAULT_BINDIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DEFAULT_BINDIR="${DEFAULT_BINDIR%/}"
BINDIR="${COSMICMOST_BINDIR:-$DEFAULT_BINDIR}"
NO_VERIFY="${COSMICMOST_NO_VERIFY:-0}"
BASE_URL="${COSMICMOST_BASE_URL:-}"

say() {
    printf '%s\n' "$*"
}

warn() {
    printf 'warning: %s\n' "$*" >&2
}

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
Install $BIN from GitHub releases.

Usage:
  install.sh [options]

Options:
  --version <tag>   Install a specific release, for example v0.1.0 (default: latest)
  --bindir <dir>    Install the binary here (default: ~/.local/bin)
  --no-verify       Skip SHA-256 verification
  -h, --help        Show this help

Environment:
  COSMICMOST_VERSION, COSMICMOST_BINDIR, COSMICMOST_NO_VERIFY,
  COSMICMOST_BASE_URL

The installer is designed for a normal user account. It never uses sudo.
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            VERSION="$2"
            shift 2
            ;;
        --version=*)
            VERSION="${1#*=}"
            shift
            ;;
        --bindir)
            [ "$#" -ge 2 ] || fail "--bindir requires a value"
            BINDIR="$2"
            shift 2
            ;;
        --bindir=*)
            BINDIR="${1#*=}"
            shift
            ;;
        --no-verify)
            NO_VERIFY=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1 (try --help)"
            ;;
    esac
done

[ "$(uname -s)" = "Linux" ] || fail "cosmicmost requires Linux"
[ -n "${HOME:-}" ] || fail "HOME is not set"
[ "$(id -u)" -ne 0 ] || fail "run this installer as your user, not as root"

for command in curl tar; do
    command -v "$command" >/dev/null 2>&1 || fail "$command is required"
done
if [ "$NO_VERIFY" != "1" ]; then
    command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is required (or pass --no-verify)"
fi

case "$(uname -m)" in
    x86_64|amd64)
        TARGET="x86_64-unknown-linux-gnu"
        ;;
    aarch64|arm64)
        TARGET="aarch64-unknown-linux-gnu"
        ;;
    *)
        fail "unsupported architecture: $(uname -m); build from source with cargo install --path ."
        ;;
esac

# Accept 0.1.0, v0.1.0, or "latest".
case "$VERSION" in
    ""|latest) VERSION="" ;;
    v*) ;;
    *) VERSION="v$VERSION" ;;
esac

if [ -z "$VERSION" ]; then
    say "Looking up the latest release..."
    release_url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
        "https://github.com/$REPO/releases/latest") ||
        fail "cannot reach GitHub; check your network connection"
    case "$release_url" in
        */releases/tag/v*) VERSION="${release_url##*/}" ;;
        *) fail "no published release found for $REPO" ;;
    esac
fi

if [ -z "$BASE_URL" ]; then
    BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
else
    BASE_URL="${BASE_URL%/}"
fi

BINDIR="${BINDIR%/}"
[ -n "$BINDIR" ] || fail "install directory must not be empty"

ARCHIVE="$BIN-$VERSION-$TARGET.tar.gz"
URL="$BASE_URL/$ARCHIVE"

TMPDIR="${TMPDIR:-/tmp}"
tmp=$(mktemp -d "$TMPDIR/cosmicmost.XXXXXX") || fail "cannot create a temporary directory"
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT HUP INT TERM

say "Downloading $VERSION for $TARGET..."
curl -fsSL --proto-redir '=https' -o "$tmp/$ARCHIVE" "$URL" ||
    fail "cannot download $URL"

if [ "$NO_VERIFY" != "1" ]; then
    curl -fsSL --proto-redir '=https' -o "$tmp/$ARCHIVE.sha256" "$URL.sha256" ||
        fail "cannot download $URL.sha256"
    (cd "$tmp" && sha256sum -c "$ARCHIVE.sha256") >/dev/null 2>&1 ||
        fail "checksum verification failed for $ARCHIVE"
    say "Verified checksum."
else
    warn "skipping checksum verification"
fi

tar -xzf "$tmp/$ARCHIVE" -C "$tmp" "$BIN" ||
    fail "cannot extract $ARCHIVE"
install -d "$BINDIR" || fail "cannot create $BINDIR"
install -m 755 "$tmp/$BIN" "$BINDIR/$BIN" ||
    fail "cannot install $BIN to $BINDIR"

installed_version=$("$BINDIR/$BIN" --version 2>/dev/null || true)
say ""
say "Installed ${installed_version:-$BIN} to $BINDIR/$BIN"

RUN="$BIN"
case ":${PATH:-}:" in
    *":$BINDIR:"*) ;;
    *)
        RUN="$BINDIR/$BIN"
        warn "$BINDIR is not in PATH"
        say "Add this to your shell profile:"
        say "  export PATH=\"$BINDIR:\$PATH\""
        ;;
esac

if [ "$BINDIR" = "$DEFAULT_BINDIR" ] && command -v systemctl >/dev/null 2>&1 &&
    systemctl --user is-active --quiet cosmicmost.service 2>/dev/null; then
    if systemctl --user restart cosmicmost.service 2>/dev/null; then
        say "Restarted cosmicmost.service."
    else
        warn "could not restart cosmicmost.service; run: systemctl --user restart cosmicmost.service"
    fi
fi

say ""
if [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/cosmic/io.github.cosmic-utils.cosmicmost/v1/servers" ]; then
    say "To apply the current COSMIC theme:"
    say "  $RUN apply"
else
    say "Next steps:"
    say "  $RUN init"
    say "  $RUN install-service"
fi
